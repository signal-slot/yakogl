// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Fixed-function render state, clears, queries (`gl_state.c`).
//!
//! These record into the context; `draw` later maps the recorded state onto
//! the `GlesDraw` fields (depth/blend/cull/scissor/stencil) and the backend
//! enum space. glClear does not render on its own: it records a pending clear
//! (color captured per GL) that the next draw fuses via `GlesDraw::do_clear`;
//! glFlush/glFinish emit a clear-only kick for a clear with no following draw
//! (see `draw`).
//!
//! Errors are logged under the name of the C function that raised them
//! (`__func__` in the C library), which for glEnable/glDisable is the shared
//! `set_cap` helper and for the non-Separate blend/stencil calls the Separate
//! entry point they forward to.
#![allow(non_snake_case)]

use super::consts::*;
use super::context::{
    self, gl_trace, GlContext, CAP_COMBINED_TEXTURE_UNITS, CAP_TEXTURE_UNITS, MAX_CONTEXTS,
    MAX_FRAGMENT_UNIFORM_VECTORS, MAX_PROGRAM_TEXEL_OFFSET, MAX_UNIFORM_DWORDS, MAX_VARYING_VECTORS,
    MAX_VERTEX_ATTRIBS, MAX_VERTEX_UNIFORM_VECTORS, MIN_PROGRAM_TEXEL_OFFSET,
};
use super::draw;
use super::names::{self, Kind};
use super::types::*;
use crate::util::{self, Global};

/* The extension list, as one string (glGetString) and one name at a time
 * (glGetStringi). The ES 3.0 core features are not listed: Qt derives them
 * from the version. */
/// Every name here is something this library really does. GL_OES_fbo_render_
/// mipmap is the least obvious: glFramebufferTexture2D takes any level of the
/// chain and the render lands in that level, which is how Qt prefilters a
/// light probe.
/* A slice, not an array of a written-down length. The length was `8` beside
 * nine entries for a while and main did not build; correcting it to 9 fixes
 * that instance and leaves the next one, because the count is a fact about
 * the list that a person has to restate. The compiler counts them now. */
const ALL_EXTENSION_NAMES: &[&[u8]] = &[
    b"GL_OES_EGL_image\0",
    /* dFdx, dFdy and fwidth in a fragment shader. Advertised because both
     * GPUs really do it, measured rather than assumed: the shipped shader
     * tables hold 39 call sites of the three built-ins on the PowerVR and 19
     * on the GC7000, in sources that all have compiled blob rows. Fourteen
     * PowerVR sources also say `#extension GL_OES_standard_derivatives :
     * require`, and until this line existed the online front end refused them
     * -- correctly, against an extension the implementation did not claim. */
    b"GL_OES_standard_derivatives\0",
    b"GL_OES_compressed_ETC1_RGB8_texture\0",
    b"GL_OES_compressed_ETC2_RGB8_texture\0",
    b"GL_OES_compressed_ETC2_RGBA8_texture\0",
    b"GL_OES_vertex_array_object\0",
    /* glMapBufferOES / glUnmapBufferOES / glGetBufferPointervOES: the whole
     * store mapped write-only, on ES 3.0's mapping (gl/buffer.rs). glmark2's
     * buffer:update-method=map scenes ask for it (scene-buffer.cpp:330-337). */
    b"GL_OES_mapbuffer\0",
    b"GL_OES_fbo_render_mipmap\0",
    /* glTexImage2D / glTexSubImage2D / glCopyTexImage2D take GL_BGRA_EXT as
     * format and as unsized internal format, and such a texture is a colour
     * attachment like any other. NOT GL_EXT_read_format_bgra, which is a
     * separate extension for glReadPixels and is not claimed:
     * GL_IMPLEMENTATION_COLOR_READ_FORMAT stays GL_RGBA, which GL allows. */
    b"GL_EXT_texture_format_BGRA8888\0",
    /* GL_RGB8 and GL_RGBA8 are renderbuffer storage and are colour
     * renderable, and so are RGB / RGBA textures of GL_UNSIGNED_BYTE. That
     * is a statement of what already happens rather than a new capability:
     * every FBO texture and colour renderbuffer here IS RGBA8, whatever
     * smaller format was asked for.
     *
     * It has to be said out loud because ES 2.0 does not say it. dEQP's
     * renderable set for ES 2.0 is RGBA4, RGB5_A1, RGB565 and the matching
     * unsized texture formats; anything else is renderable only if an
     * extension permits it, and fbo.completeness reads GL_EXTENSIONS to
     * decide what to expect. Answering GL_FRAMEBUFFER_COMPLETE for RGBA8
     * while claiming nothing is what it was failing on.
     *
     * The other formats that reach GL_FRAMEBUFFER_COMPLETE here -- R8, RG8,
     * the half-float and sRGB ones -- would each need their own extension
     * named (GL_EXT_texture_rg, GL_EXT_color_buffer_half_float, ...), and
     * naming one commits this GL to everything that extension specifies,
     * not only to the completeness answer. They stay unclaimed until that
     * is measured. */
    b"GL_OES_rgb8_rgba8\0",
    /* Sized internal formats in glTexImage2D. `sized_format` takes every one
     * of them and stores at least the precision asked for: RGBA4, RGB5_A1
     * and RGB565 become RGBA8, which is more, and RGBA8 and RGB8 are exact.
     * ES 2.0 on its own has only the unsized formats, so a suite that asks
     * for GL_RGBA4 as an internal format has to be told this is allowed. */
    b"GL_OES_required_internalformat\0",
    /* R and RG textures. The storage is RGBA8 rather than one or two
     * channels -- an implementation detail -- but what a shader SAMPLES is
     * the extension's contract, and the upload writes exactly that: a RED
     * texel expands to (r, 0, 0, 1) and an RG one to (r, g, 0, 1). */
    b"GL_EXT_texture_rg\0",
    /* Half-float textures and half-float colour buffers. These are real
     * storage classes here, not RGBA8 in disguise: GLES_TEXFMT_RGBA16F and
     * GLES_TEXFMT_R16F have their own texel layouts, GLES_SURF_RGBA16F and
     * GLES_SURF_R16F their own render-target classes, and the clear path a
     * float encoding of its own (`pack_clear_float`).
     *
     * NOT the single-precision pair. GL_OES_texture_float would promise
     * 32-bit texels and `sized_format` stores GL_RGBA32F as halves, so
     * claiming it would be a claim about precision this GL does not have.
     * GL_EXT_sRGB is not claimed either: the ES 3.0 sRGB class
     * (docs/es3-srgb.md) is encoded only where the back end renders it (the
     * PowerVR), and the ES 2.0 report keeps the sRGB names refused, as they
     * always were. */
    b"GL_OES_texture_half_float\0",
    b"GL_EXT_color_buffer_half_float\0",
    /* Colour attachment points 1..GL_MAX_COLOR_ATTACHMENTS_NV (0x8CDF, the
     * same token as GL_MAX_COLOR_ATTACHMENTS): FramebufferTexture2D,
     * FramebufferRenderbuffer and GetFramebufferAttachmentParameteriv take
     * them, and completeness checks every one (framebuffer.rs
     * check_status). Nothing about drawing into them: that is
     * GL_NV_draw_buffers, not claimed (the registry text: "Rendering to color
     * attachments COLOR_ATTACHMENT[1..15]_NV can be activated through the
     * GL_NV_draw_buffers extension"). True on all three GPUs: each has
     * context::max_color_attachments() points (3 today). dEQP-GLES2's completeness tests run their
     * multi-attachment cases only when this is claimed
     * (es2fFboCompletenessTests.cpp:354): 54 were not supported on the Pi 4
     * (records/pi4/record_SW28_FB1_2c87c2e4.txt). */
    b"GL_NV_fbo_color_attachments\0",
    /* glInsertEventMarkerEXT, glPushGroupMarkerEXT, glPopGroupMarkerEXT
     * (below): accepted and ignored. The extension asks nothing more of an
     * implementation that has no debugger to show the markers to, and it
     * forbids errors outright, invalid arguments included. dEQP-GLES2's
     * debug_marker group runs only when this is claimed
     * (es2fDebugMarkerTests.cpp:48). */
    b"GL_EXT_debug_marker\0",
    /* The last ES2_ONLY_NAMES entries: listed only on the ES 2.0 surface
     * (NPOT_EXTENSION and DEPTH_TEXTURE_EXTENSIONS), whose core lacks them.
     *
     * GL_OES_texture_npot: a non-power-of-two texture may have a mip chain
     * and wrap with GL_REPEAT or GL_MIRRORED_REPEAT, and glGenerateMipmap
     * builds its chain (ES 2.0 3.7.10's NPOT completeness rule and 3.7.11's
     * glGenerateMipmap error go; texture.rs NPOT_RESTRICTED). glmark2's
     * terrain generates mipmaps of an 800x600 texture and sampled it black
     * without it (tools/cts/RESULTS.md, 2026-10-04). Where the chain lives
     * is the backend's: the V3D and the GC7000 place every level from the GL
     * core's own offsets and get the whole chain (texture.rs
     * npot_mip_chains); the PowerVR's TPU finds level i+1 by its own
     * power-of-two arithmetic, so a 2D chain there is laid out as the chain
     * of its power-of-two extent, as Mesa's pvr driver lays one out
     * (texture.rs pot_chain_layout). A stacked NPOT cube map on the PowerVR
     * keeps its base level, complete, and is sampled from it: the one way
     * the PowerVR falls short of the extension, right at level 0 and
     * aliased where minified. */
    b"GL_OES_texture_npot\0",
    /* GL_OES_depth_texture and GL_OES_packed_depth_stencil.
     *
     * GL_OES_depth_texture: glTexImage2D / glTexSubImage2D take
     * GL_DEPTH_COMPONENT with GL_UNSIGNED_SHORT or GL_UNSIGNED_INT on
     * GL_TEXTURE_2D (texture.rs es2_depth_image_error), such a texture is a
     * depth attachment (framebuffer.rs attachment_storage: the render writes
     * the ISP's own depth buffer, keyed by the texture), glGenerateMipmap of
     * one is GL_INVALID_OPERATION, and it samples as (d, d, d, 1)
     * (`GlesTextureBind::depth_luminance`; every GPU swizzles its R32F
     * look-up). GL_OES_packed_depth_stencil: the same for
     * GL_DEPTH_STENCIL_OES / GL_UNSIGNED_INT_24_8_OES textures, which are
     * stencil attachments too (one texture on both points is one packed
     * buffer, draw.rs resolve_ds_storage), and GL_DEPTH24_STENCIL8_OES
     * renderbuffers. dEQP-GLES2 fbo.completeness finds a texture for
     * GL_STENCIL_ATTACHMENT only through this pair
     * (glsFboCompletenessTests.cpp:207): 27 attachment_combinations cases were
     * NotSupported without it.
     *
     * Rendered depth: a render writes depth into the backend's depth/stencil
     * buffer, keyed by the texture (draw.rs ds_identity), and it reaches the
     * texels when the texture is next sampled or written by the CPU
     * (texture::resolve_depth, `backend::depth_resolve`: every GPU reads its
     * D24S8 back on the CPU). The stencil half of a
     * depth-stencil texture is only ever the stencil buffer: sampling one
     * reads "as if the base internal format was DEPTH", its stencil index
     * ignored (OES_packed_depth_stencil).
     *
     * Not done, on either surface: the other direction. Depth or stencil the
     * CPU writes into a texture that is then attached is not loaded into the
     * backend's buffer, which keeps what the last render left there. */
    b"GL_OES_depth_texture\0",
    b"GL_OES_packed_depth_stencil\0",
];
/// How many names at the end of the list are the ES 2.0 surface's own:
/// GL_OES_texture_npot and the depth-texture pair.
const ES2_ONLY_NAMES: usize = 3;
/// GL_OES_texture_npot is claimed on the ES 2.0 surface of every GPU (the
/// list above says how each backend holds the chain); ES 3.0 has it in core.
/// On the GC7000 the claim is the part's: it holds only while the hardware's
/// NON_POWER_OF_TWO bit is set (`vivante::backend::NON_POWER_OF_TWO`, pinned
/// from the etnaviv hwdb entry by tests/vivante_npot.rs). A build for a
/// GC7000 variant without the bit takes the extension out here.
#[cfg(feature = "gpu-vivante")]
pub(crate) const NPOT_EXTENSION: bool = crate::REPORT_ES2 && crate::vivante::backend::NON_POWER_OF_TWO;
/// GL_OES_texture_npot is claimed on the ES 2.0 surface of every GPU (the
/// list above says how each backend holds the chain); ES 3.0 has it in core.
#[cfg(not(feature = "gpu-vivante"))]
pub(crate) const NPOT_EXTENSION: bool = crate::REPORT_ES2;
/// GL_OES_depth_texture and GL_OES_packed_depth_stencil are claimed on the
/// ES 2.0 surface of every GPU. ES 3.0 has them in core (and samples a depth
/// texture as (d, 0, 0, 1), not the extension's luminance). The V3D samples
/// the depth texture's R32F texels at the TMU's 32-bit output (v3d/texture.rs
/// `return_32`, the ret32 sampler variant) and reads its D24S8 back on the
/// CPU (`v3d_depth_resolve`); gl_test case 169 dstex passed on the Pi 4 with
/// it (tools/cts/RESULTS.md, 2026-10-04).
pub(crate) const DEPTH_TEXTURE_EXTENSIONS: bool = crate::REPORT_ES2;
/* the ES 2.0-only tail is listed or left out whole. The GC7000's NPOT gate
 * (above) reads the part's hwdb bit; if a build ever finds it clear, this
 * assert is where the tail has to be split so npot can be left out on its
 * own. */
const _: () = assert!(NPOT_EXTENSION == DEPTH_TEXTURE_EXTENSIONS);
/// The extension list of this build: [`ALL_EXTENSION_NAMES`] less the
/// ES 2.0-only names where they are not claimed.
pub(crate) static EXTENSION_NAMES: &[&[u8]] = if DEPTH_TEXTURE_EXTENSIONS {
    ALL_EXTENSION_NAMES
} else {
    ALL_EXTENSION_NAMES.split_at(ALL_EXTENSION_NAMES.len() - ES2_ONLY_NAMES).0
};
const EXTENSION_COUNT: usize = EXTENSION_NAMES.len();

/// Does this implementation advertise `name`?
///
/// The online shader compiler asks this to decide `#extension <name> :
/// require`, so that what the compiler accepts and what
/// `glGetString(GL_EXTENSIONS)` claims cannot drift apart: a shader that
/// requires an extension the implementation does not advertise must fail to
/// compile, and one that requires an advertised extension must not.
///
/// Only the builds that have the online compiler ask, so it is gated: an
/// unused function is a warning, and the gate treats a warning as a defect
/// because a build that already prints one is a build where the next real
/// warning is invisible.
#[cfg(feature = "shaderc-online")]
pub(crate) fn advertises(name: &str) -> bool {
    extension_listed(name)
}

/// Is `name` in this build's extension list, the one `glGetString(GL_EXTENSIONS)`
/// prints? For the GL core code that has to follow what the application was
/// told; not gated, every build asks it.
pub(crate) fn extension_listed(name: &str) -> bool {
    EXTENSION_NAMES.iter().any(|e| e.strip_suffix(b"\0").is_some_and(|e| e == name.as_bytes()))
}
macro_rules! base_extension_string {
    () => {
        "GL_OES_EGL_image GL_OES_standard_derivatives \
GL_OES_compressed_ETC1_RGB8_texture \
GL_OES_compressed_ETC2_RGB8_texture GL_OES_compressed_ETC2_RGBA8_texture \
GL_OES_vertex_array_object GL_OES_mapbuffer GL_OES_fbo_render_mipmap GL_EXT_texture_format_BGRA8888 \
GL_OES_rgb8_rgba8 GL_OES_required_internalformat GL_EXT_texture_rg \
GL_OES_texture_half_float GL_EXT_color_buffer_half_float GL_NV_fbo_color_attachments \
GL_EXT_debug_marker"
    };
}
const EXTENSION_STRING: &[u8] = if DEPTH_TEXTURE_EXTENSIONS {
    concat!(base_extension_string!(), " GL_OES_texture_npot GL_OES_depth_texture GL_OES_packed_depth_stencil\0")
        .as_bytes()
} else {
    concat!(base_extension_string!(), "\0").as_bytes()
};

/// GL_FRAGMENT_SHADER_DERIVATIVE_HINT per context: an ES 3.0 hint the
/// implementation must accept and report back (nothing here acts on it; the
/// USC computes derivatives one way). It lives beside the context because
/// `struct gl_context` is an ABI the C view and the layout pins fix.
///
/// The slot of a context is (re)set by [`reset_context_hints`], which
/// `gl_context_create` calls.
static DERIVATIVE_HINT: Global<[GLenum; MAX_CONTEXTS]> = Global::zeroed();

/// The hints of context `index` at their defaults; `gl_context_create` calls
/// this, so a context does not inherit the hints of a destroyed one that had
/// its pool slot.
pub(super) fn reset_context_hints(index: u8) {
    // SAFETY: single-threaded by contract; no other reference to the array is live.
    if let Some(h) = unsafe { DERIVATIVE_HINT.get() }.get_mut(index as usize) {
        *h = GL_DONT_CARE;
    }
}

/// The recorded GL_FRAGMENT_SHADER_DERIVATIVE_HINT of a context (the zeroed
/// initial value of the array reads as the default).
fn derivative_hint(c: &GlContext) -> GLenum {
    // SAFETY: single-threaded by contract; no other reference to the array is live.
    match unsafe { DERIVATIVE_HINT.get() }.get(c.index as usize) {
        Some(0) | None => GL_DONT_CARE,
        Some(h) => *h,
    }
}

/// The largest viewport / scissor rectangle (GL_MAX_VIEWPORT_DIMS).
const MAX_VIEWPORT_DIM: GLsizei = 4096;

/// The largest point size (GL_ALIASED_POINT_SIZE_RANGE's maximum). No
/// backend clamps gl_PointSize itself, so this must be the size the GPU
/// clamps to: dEQP rasterization.limits.points draws a point 16 larger than
/// the maximum in an FBO (maximum + 1) / 2 + 8 pixels wide and expects it
/// drawn at the maximum. The V3D clamps at 512, Mesa's V3D_MAX_POINT_SIZE
/// (broadcom/common/v3d_limits.h:46): gl_test 136 psizmax on the Pi 4
/// (df5aa00e) drew 511 and 512 at their size and 513, 600 and 1024 at 512.
/// Mesa's source for the 512 is its developers' measurement, not a public
/// V3D document: Mesa commit 3539bd63 (MR !1499, 2019, reviewed by Eric
/// Anholt) says "For V3D this is [1.0, 512.0] ... For points larger than
/// the maximum size of the range the hardware clamps automatically".
/// With 64 reported, the limits case drew 80 and failed (c9eb1078).
///
/// The PowerVR: 511, where both its compilers clamp gl_PointSize -- the
/// online one in powervr/prepare.rs, pco through NIR's nir_lower_point_size
/// -- to Mesa's PVR_POINT_SIZE_RANGE_MAX (src/imagination/common/
/// pvr_limits.h:48, 511.0f, which Mesa's PowerVR Vulkan driver reports as
/// pointSizeRange, pvr_physical_device.c:755-756). So on the PowerVR the
/// clamp is the shader's own, not the hardware's. With 64 reported, the
/// AM62P drew dEQP's size-80 point at 80 and failed the limits case
/// (2026-09-25, echo image 9578c73ad151846e: the test area came out red).
/// The GC7000: 64, where its online compiler clamps gl_PointSize
/// (shaderc/online/select.rs, POINT_SIZE_MAX): the hardware itself drew
/// dEQP's size-80 point at 80 with 64 reported (i.MX8MP, Zephyr, 2362422d).
/// Mesa's etnaviv reports 8192 and relies on the hardware's clamp, which the
/// limits case can only see in an 8200-wide framebuffer; ours stop at 4096.
const POINT_SIZE_MAX: GLfloat = if cfg!(feature = "gpu-v3d") {
    512.0
} else if cfg!(feature = "gpu-vivante") {
    64.0
} else {
    511.0
};

fn clampf(v: GLfloat) -> GLclampf {
    /* like the C ternary chain: NaN and -0.0 pass through unchanged */
    v.clamp(0.0, 1.0)
}

/* --------------------------------------------------------------------------- */

#[no_mangle]
pub unsafe extern "C" fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) {
    gl_trace!(glViewport);
    let Some(c) = context::current() else { return };
    if width < 0 || height < 0 {
        c.set_error(GL_INVALID_VALUE, "glViewport");
        return;
    }
    /* GL ES 3.0 2.12.1: the size is clamped to the implementation's maximum
     * (the ISP cannot rasterise beyond it either) */
    c.viewport = [x, y, width.min(MAX_VIEWPORT_DIM), height.min(MAX_VIEWPORT_DIM)];
}

#[no_mangle]
pub unsafe extern "C" fn glDepthRangef(n: GLfloat, f: GLfloat) {
    gl_trace!(glDepthRangef);
    let Some(c) = context::current() else { return };
    c.depth_range[0] = clampf(n);
    c.depth_range[1] = clampf(f);
}

#[no_mangle]
pub unsafe extern "C" fn glScissor(x: GLint, y: GLint, width: GLsizei, height: GLsizei) {
    gl_trace!(glScissor);
    let Some(c) = context::current() else { return };
    if width < 0 || height < 0 {
        c.set_error(GL_INVALID_VALUE, "glScissor");
        return;
    }
    c.scissor = [x, y, width, height];
}

#[no_mangle]
pub unsafe extern "C" fn glClearColor(r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat) {
    gl_trace!(glClearColor);
    let Some(c) = context::current() else { return };
    /* ES 3.0 keeps the values as given: a float colour buffer clears to
     * them, a normalized one clamps (the backend's RGBA8 packing does). */
    c.clear_color = [r, g, b, a];
}

#[no_mangle]
pub unsafe extern "C" fn glClearDepthf(d: GLfloat) {
    gl_trace!(glClearDepthf);
    let Some(c) = context::current() else { return };
    c.clear_depth = clampf(d);
}

#[no_mangle]
pub unsafe extern "C" fn glClearStencil(s: GLint) {
    gl_trace!(glClearStencil);
    let Some(c) = context::current() else { return };
    c.clear_stencil = s;
}

#[no_mangle]
pub unsafe extern "C" fn glClear(mask: GLbitfield) {
    gl_trace!(glClear);
    let Some(c) = context::current() else { return };
    if mask & !(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) != 0 {
        /* GL ES 3.0 4.2.3: any other bit is an error and the call has no effect */
        c.set_error(GL_INVALID_VALUE, "glClear");
        return;
    }
    /* ES 3.0 4.2.3: with GL_RASTERIZER_DISCARD enabled clears have no effect
     * (glClearBuffer* reach the framebuffer through this call, their own
     * argument errors raised before it). The argument error above comes first. */
    if !crate::REPORT_ES2 && c.rasterizer_discard {
        return;
    }
    /* (a clear still pending for another framebuffer is emitted first) */
    let pending = draw::pending_clear_for(c);
    /* Record a pending clear; the values are captured now (per GL). COLOR,
     * DEPTH and STENCIL reach the backend as the next frame's background
     * object. */
    if mask & GL_COLOR_BUFFER_BIT != 0 {
        pending.color = true;
        pending.rgba = c.clear_color;
    }
    if mask & GL_DEPTH_BUFFER_BIT != 0 {
        /* The depth clear becomes the background depth of the next frame
         * (the on-chip depth buffer starts from it at every kick). */
        pending.depth = true;
        pending.depth_value = c.clear_depth;
    }
    if mask & GL_STENCIL_BUFFER_BIT != 0 {
        /* likewise the stencil clear -> the background object's stencil */
        pending.stencil = true;
        pending.stencil_value = (c.clear_stencil & 0xff) as u8;
    }
    /* If the app clears and never draws (clear-then-swap), glFlush/glFinish
     * flushes it. Nothing else to do here. */
}

#[no_mangle]
pub unsafe extern "C" fn glColorMask(r: GLboolean, g: GLboolean, b: GLboolean, a: GLboolean) {
    gl_trace!(glColorMask);
    let Some(c) = context::current() else { return };
    c.color_writemask = [r, g, b, a].map(|v| if v != 0 { 1.0 } else { 0.0 });
}

/* --------------------------------------------------------------------------- */

/// The capabilities ES 3.0 adds to the nine of ES 2.0 (4.1 and 6.1.1): a
/// `report-es2` build refuses them with GL_INVALID_ENUM in glEnable, glDisable,
/// glIsEnabled and the glGet* queries.
fn es3_only_cap(cap: GLenum) -> bool {
    crate::REPORT_ES2 && matches!(cap, GL_PRIMITIVE_RESTART_FIXED_INDEX | GL_RASTERIZER_DISCARD)
}

fn set_cap(cap: GLenum, on: bool) {
    let Some(c) = context::current() else { return };
    if es3_only_cap(cap) {
        c.set_error(GL_INVALID_ENUM, "set_cap");
        return;
    }
    match cap {
        GL_DEPTH_TEST => c.depth_test = on,
        GL_BLEND => c.blend = on,
        GL_CULL_FACE => c.cull_face = on,
        GL_SCISSOR_TEST => c.scissor_test = on,
        GL_STENCIL_TEST => c.stencil_test = on,
        /* Recorded for queries; not modelled by the backend (no dithering, no
         * multisampling; depth bias is not wired to the ISP yet). */
        GL_DITHER => c.dither = on,
        GL_POLYGON_OFFSET_FILL => c.polygon_offset_fill = on,
        GL_SAMPLE_ALPHA_TO_COVERAGE => c.sample_alpha_to_coverage = on,
        GL_SAMPLE_COVERAGE => c.sample_coverage = on,
        /* ES 3.0 2.8.1: glDrawElements* splits the index list at the
         * type's largest value (`draw::restart_to_lists`). */
        GL_PRIMITIVE_RESTART_FIXED_INDEX => c.primitive_restart = on,
        /* ES 3.0: draws and clears stop in the GL core while it is on
         * (`draw.rs`, `glClear`); recorded only under report-es2 */
        GL_RASTERIZER_DISCARD => c.rasterizer_discard = on,
        _ => c.set_error(GL_INVALID_ENUM, "set_cap"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glPolygonOffset(factor: GLfloat, units: GLfloat) {
    gl_trace!(glPolygonOffset);
    let Some(c) = context::current() else { return };
    c.polygon_offset_factor = factor;
    c.polygon_offset_units = units;
}

#[no_mangle]
pub unsafe extern "C" fn glSampleCoverage(value: GLfloat, invert: GLboolean) {
    gl_trace!(glSampleCoverage);
    let Some(c) = context::current() else { return };
    c.sample_coverage_value = clampf(value);
    c.sample_coverage_invert = invert != 0;
}

#[no_mangle]
pub unsafe extern "C" fn glHint(target: GLenum, mode: GLenum) {
    gl_trace!(glHint);
    let Some(c) = context::current() else { return };
    if mode != GL_FASTEST && mode != GL_NICEST && mode != GL_DONT_CARE {
        c.set_error(GL_INVALID_ENUM, "glHint");
        return;
    }
    match target {
        GL_GENERATE_MIPMAP_HINT => c.generate_mipmap_hint = mode,
        GL_FRAGMENT_SHADER_DERIVATIVE_HINT => {
            // SAFETY: single-threaded by contract; no other reference to the array is live.
            if let Some(h) = unsafe { DERIVATIVE_HINT.get() }.get_mut(c.index as usize) {
                *h = mode;
            }
        }
        _ => c.set_error(GL_INVALID_ENUM, "glHint"),
    }
}

/// The pixel-store names ES 3.0 adds to PACK/UNPACK_ALIGNMENT.
fn es3_pixel_store(pname: GLenum) -> bool {
    matches!(
        pname,
        GL_UNPACK_ROW_LENGTH
            | GL_UNPACK_SKIP_ROWS
            | GL_UNPACK_SKIP_PIXELS
            | GL_UNPACK_SKIP_IMAGES
            | GL_UNPACK_IMAGE_HEIGHT
            | GL_PACK_ROW_LENGTH
            | GL_PACK_SKIP_ROWS
            | GL_PACK_SKIP_PIXELS
    )
}

#[no_mangle]
pub unsafe extern "C" fn glPixelStorei(pname: GLenum, param: GLint) {
    gl_trace!(glPixelStorei);
    let Some(c) = context::current() else { return };
    if crate::REPORT_ES2 && es3_pixel_store(pname) {
        /* ES 2.0 3.6.1 Table 3.1 has the two alignments only */
        c.set_error(GL_INVALID_ENUM, "glPixelStorei");
        return;
    }
    match pname {
        GL_UNPACK_ALIGNMENT | GL_PACK_ALIGNMENT => {
            if param != 1 && param != 2 && param != 4 && param != 8 {
                c.set_error(GL_INVALID_VALUE, "glPixelStorei");
                return;
            }
            if pname == GL_UNPACK_ALIGNMENT {
                c.unpack_alignment = param;
            } else {
                c.pack_alignment = param;
            }
        }
        /* ES 3.0 pixel store state, honoured by the uploads (Qt's RHI
         * uploads sub-rectangles of a larger image with the row length) and
         * by glReadPixels. The skips, image height and pack row length were
         * accepted at 0 only and dropped otherwise until 2026-09-24 (dEQP-GLES3
         * state_query.integers.*pack*: 28 cases read back 0). */
        GL_UNPACK_ROW_LENGTH
        | GL_UNPACK_SKIP_ROWS
        | GL_UNPACK_SKIP_PIXELS
        | GL_UNPACK_SKIP_IMAGES
        | GL_UNPACK_IMAGE_HEIGHT
        | GL_PACK_ROW_LENGTH
        | GL_PACK_SKIP_ROWS
        | GL_PACK_SKIP_PIXELS => {
            if param < 0 {
                c.set_error(GL_INVALID_VALUE, "glPixelStorei");
                return;
            }
            let slot = match pname {
                GL_UNPACK_ROW_LENGTH => &mut c.unpack_row_length,
                GL_UNPACK_SKIP_ROWS => &mut c.unpack_skip_rows,
                GL_UNPACK_SKIP_PIXELS => &mut c.unpack_skip_pixels,
                GL_UNPACK_SKIP_IMAGES => &mut c.unpack_skip_images,
                GL_UNPACK_IMAGE_HEIGHT => &mut c.unpack_image_height,
                GL_PACK_ROW_LENGTH => &mut c.pack_row_length,
                GL_PACK_SKIP_ROWS => &mut c.pack_skip_rows,
                _ => &mut c.pack_skip_pixels,
            };
            *slot = param;
        }
        _ => c.set_error(GL_INVALID_ENUM, "glPixelStorei"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glEnable(cap: GLenum) {
    set_cap(cap, true);
}

#[no_mangle]
pub unsafe extern "C" fn glDisable(cap: GLenum) {
    set_cap(cap, false);
}

/// The enable flag of `cap`, `None` when `cap` is not a capability.
fn is_enabled(c: &GlContext, cap: GLenum) -> Option<bool> {
    if es3_only_cap(cap) {
        return None;
    }
    Some(match cap {
        GL_DEPTH_TEST => c.depth_test,
        GL_BLEND => c.blend,
        GL_CULL_FACE => c.cull_face,
        GL_SCISSOR_TEST => c.scissor_test,
        GL_STENCIL_TEST => c.stencil_test,
        GL_DITHER => c.dither,
        GL_POLYGON_OFFSET_FILL => c.polygon_offset_fill,
        GL_SAMPLE_ALPHA_TO_COVERAGE => c.sample_alpha_to_coverage,
        GL_SAMPLE_COVERAGE => c.sample_coverage,
        GL_PRIMITIVE_RESTART_FIXED_INDEX => c.primitive_restart,
        GL_RASTERIZER_DISCARD => c.rasterizer_discard,
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn glIsEnabled(cap: GLenum) -> GLboolean {
    gl_trace!(glIsEnabled);
    let Some(c) = context::current() else { return GL_FALSE_B };
    match is_enabled(c, cap) {
        Some(on) => on as GLboolean,
        None => {
            c.set_error(GL_INVALID_ENUM, "glIsEnabled");
            GL_FALSE_B
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDepthFunc(func: GLenum) {
    gl_trace!(glDepthFunc);
    let Some(c) = context::current() else { return };
    if !(GL_NEVER..=GL_ALWAYS).contains(&func) {
        c.set_error(GL_INVALID_ENUM, "glDepthFunc");
        return;
    }
    c.depth_func = func;
}

#[no_mangle]
pub unsafe extern "C" fn glDepthMask(flag: GLboolean) {
    gl_trace!(glDepthMask);
    let Some(c) = context::current() else { return };
    c.depth_mask = flag != 0;
}

fn valid_blend_factor(f: GLenum, dst: bool) -> bool {
    match f {
        GL_ZERO
        | GL_ONE
        | GL_SRC_COLOR
        | GL_ONE_MINUS_SRC_COLOR
        | GL_DST_COLOR
        | GL_ONE_MINUS_DST_COLOR
        | GL_SRC_ALPHA
        | GL_ONE_MINUS_SRC_ALPHA
        | GL_DST_ALPHA
        | GL_ONE_MINUS_DST_ALPHA
        | GL_CONSTANT_COLOR
        | GL_ONE_MINUS_CONSTANT_COLOR
        | GL_CONSTANT_ALPHA
        | GL_ONE_MINUS_CONSTANT_ALPHA => true,
        /* ES 2.0 4.1.6 takes it as a source factor only; ES 3.0 4.1.7 as
         * either (dEQP-GLES3 state_query.integers.blend_dst_*) */
        GL_SRC_ALPHA_SATURATE => !dst || !crate::REPORT_ES2,
        _ => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBlendFuncSeparate(srcRGB: GLenum, dstRGB: GLenum, srcAlpha: GLenum, dstAlpha: GLenum) {
    gl_trace!(glBlendFuncSeparate);
    let Some(c) = context::current() else { return };
    if !valid_blend_factor(srcRGB, false)
        || !valid_blend_factor(dstRGB, true)
        || !valid_blend_factor(srcAlpha, false)
        || !valid_blend_factor(dstAlpha, true)
    {
        c.set_error(GL_INVALID_ENUM, "glBlendFuncSeparate");
        return;
    }
    c.blend_src = srcRGB;
    c.blend_dst = dstRGB;
    c.blend_src_a = srcAlpha;
    c.blend_dst_a = dstAlpha;
}

#[no_mangle]
pub unsafe extern "C" fn glBlendFunc(sfactor: GLenum, dfactor: GLenum) {
    gl_trace!(glBlendFunc);
    glBlendFuncSeparate(sfactor, dfactor, sfactor, dfactor);
}

#[no_mangle]
pub unsafe extern "C" fn glBlendEquationSeparate(modeRGB: GLenum, modeAlpha: GLenum) {
    gl_trace!(glBlendEquationSeparate);
    let Some(c) = context::current() else { return };
    /* ES 3.0 4.1.7 adds MIN and MAX to the ES 2.0 three; GL_EXT_blend_minmax
     * is not in ALL_EXTENSION_NAMES, so a report-es2 build refuses them */
    let valid = |m: GLenum| {
        matches!(m, GL_FUNC_ADD | GL_FUNC_SUBTRACT | GL_FUNC_REVERSE_SUBTRACT)
            || (!crate::REPORT_ES2 && matches!(m, GL_MIN | GL_MAX))
    };
    if !valid(modeRGB) || !valid(modeAlpha) {
        c.set_error(GL_INVALID_ENUM, "glBlendEquationSeparate");
        return;
    }
    c.blend_eq = modeRGB;
    c.blend_eq_a = modeAlpha;
}

#[no_mangle]
pub unsafe extern "C" fn glBlendEquation(mode: GLenum) {
    gl_trace!(glBlendEquation);
    glBlendEquationSeparate(mode, mode);
}

#[no_mangle]
pub unsafe extern "C" fn glBlendColor(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat) {
    gl_trace!(glBlendColor);
    let Some(c) = context::current() else { return };
    /* Recorded for the queries; CONSTANT_* factors need a shader variant
     * that reads the blend constant, which the offline table does not carry. */
    c.blend_color = [clampf(red), clampf(green), clampf(blue), clampf(alpha)];
}

#[no_mangle]
pub unsafe extern "C" fn glLineWidth(width: GLfloat) {
    gl_trace!(glLineWidth);
    let Some(c) = context::current() else { return };
    if width <= 0.0 {
        c.set_error(GL_INVALID_VALUE, "glLineWidth");
        return;
    }
    c.line_width = width; /* the ISP takes up to 16 px */
}

#[no_mangle]
pub unsafe extern "C" fn glCullFace(mode: GLenum) {
    gl_trace!(glCullFace);
    let Some(c) = context::current() else { return };
    if mode != GL_FRONT && mode != GL_BACK && mode != GL_FRONT_AND_BACK {
        c.set_error(GL_INVALID_ENUM, "glCullFace");
        return;
    }
    c.cull_mode = mode;
}

#[no_mangle]
pub unsafe extern "C" fn glFrontFace(mode: GLenum) {
    gl_trace!(glFrontFace);
    let Some(c) = context::current() else { return };
    if mode != GL_CW && mode != GL_CCW {
        c.set_error(GL_INVALID_ENUM, "glFrontFace");
        return;
    }
    c.front_face = mode;
}

fn valid_face(face: GLenum) -> bool {
    face == GL_FRONT || face == GL_BACK || face == GL_FRONT_AND_BACK
}

fn valid_stencil_op(op: GLenum) -> bool {
    matches!(op, GL_KEEP | GL_ZERO | GL_REPLACE | GL_INCR | GL_DECR | GL_INVERT | GL_INCR_WRAP | GL_DECR_WRAP)
}

/* The stencil buffer is 8 bits; references are clamped per GL. The backend is
 * single-faced and applies the front state to both faces (see GlesDraw). */
#[no_mangle]
pub unsafe extern "C" fn glStencilFuncSeparate(face: GLenum, func: GLenum, ref_: GLint, mask: GLuint) {
    gl_trace!(glStencilFuncSeparate);
    let Some(c) = context::current() else { return };
    if !valid_face(face) || !(GL_NEVER..=GL_ALWAYS).contains(&func) {
        c.set_error(GL_INVALID_ENUM, "glStencilFuncSeparate");
        return;
    }
    let ref_ = ref_.clamp(0, 255);
    if face != GL_BACK {
        c.stencil_func = func;
        c.stencil_ref = ref_;
        c.stencil_mask = mask;
    }
    if face != GL_FRONT {
        c.stencil_back_func = func;
        c.stencil_back_ref = ref_;
        c.stencil_back_mask = mask;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glStencilFunc(func: GLenum, ref_: GLint, mask: GLuint) {
    gl_trace!(glStencilFunc);
    glStencilFuncSeparate(GL_FRONT_AND_BACK, func, ref_, mask);
}

#[no_mangle]
pub unsafe extern "C" fn glStencilOpSeparate(face: GLenum, sfail: GLenum, dpfail: GLenum, dppass: GLenum) {
    gl_trace!(glStencilOpSeparate);
    let Some(c) = context::current() else { return };
    if !valid_face(face) || !valid_stencil_op(sfail) || !valid_stencil_op(dpfail) || !valid_stencil_op(dppass) {
        c.set_error(GL_INVALID_ENUM, "glStencilOpSeparate");
        return;
    }
    if face != GL_BACK {
        c.stencil_sfail = sfail;
        c.stencil_dpfail = dpfail;
        c.stencil_dppass = dppass;
    }
    if face != GL_FRONT {
        c.stencil_back_sfail = sfail;
        c.stencil_back_dpfail = dpfail;
        c.stencil_back_dppass = dppass;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glStencilOp(sfail: GLenum, dpfail: GLenum, dppass: GLenum) {
    gl_trace!(glStencilOp);
    glStencilOpSeparate(GL_FRONT_AND_BACK, sfail, dpfail, dppass);
}

#[no_mangle]
pub unsafe extern "C" fn glStencilMaskSeparate(face: GLenum, mask: GLuint) {
    gl_trace!(glStencilMaskSeparate);
    let Some(c) = context::current() else { return };
    if !valid_face(face) {
        c.set_error(GL_INVALID_ENUM, "glStencilMaskSeparate");
        return;
    }
    if face != GL_BACK {
        c.stencil_writemask = mask;
    }
    if face != GL_FRONT {
        c.stencil_back_writemask = mask;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glStencilMask(mask: GLuint) {
    gl_trace!(glStencilMask);
    glStencilMaskSeparate(GL_FRONT_AND_BACK, mask);
}

/* --------------------------------------------------------------------------- */

#[no_mangle]
pub unsafe extern "C" fn glGetError() -> GLenum {
    gl_trace!(glGetError);
    if context::foreign_thread() {
        /* a second thread (the thread contract, context::foreign_thread):
         * its calls were refused, and the owner's error is not its to take */
        return GL_INVALID_OPERATION;
    }
    let Some(c) = context::current() else { return GL_NO_ERROR };
    core::mem::replace(&mut c.error, GL_NO_ERROR)
}

#[no_mangle]
pub unsafe extern "C" fn glGetString(name: GLenum) -> *const GLubyte {
    gl_trace!(glGetString);
    let s: &'static [u8] = match name {
        GL_VENDOR => b"Signal Slot\0",
        /* The GPU this build drives, not the one the project started on.
         * dEQP records it in every result file and a run on the wrong name is
         * a run that cannot be told apart later. */
        #[cfg(feature = "gpu-powervr")]
        GL_RENDERER => b"PowerVR BXS-4-64 (YakoGL)\0",
        #[cfg(feature = "gpu-vivante")]
        GL_RENDERER => b"Vivante GC7000UL (YakoGL)\0",
        #[cfg(feature = "gpu-v3d")]
        GL_RENDERER => b"Broadcom V3D 4.2 (YakoGL)\0",
        /* OpenGL ES 3.0: Qt's RHI takes the context version from this string
         * (QEGLPlatformContext::updateFormatFromGL) and, at 3.x, turns on the
         * features Qt Quick 3D needs (instancing, texelFetch, float and array
         * textures, mip-level render targets) and tries the GLSL ES 3.00 shader
         * variants first, falling back to the 1.00 ones.
         *
         * A `report-es2` build says ES 2.0 instead, and then says it
         * everywhere (crate::REPORT_ES2). The version string is not a label
         * on its own: a client that believes it and asks only for what ES 2.0
         * has must find nothing more. */
        GL_VERSION if crate::REPORT_ES2 => b"OpenGL ES 2.0 YakoGL\0",
        GL_SHADING_LANGUAGE_VERSION if crate::REPORT_ES2 => b"OpenGL ES GLSL ES 1.00\0",
        GL_VERSION => b"OpenGL ES 3.0 YakoGL\0",
        GL_SHADING_LANGUAGE_VERSION => b"OpenGL ES GLSL ES 3.00\0",
        GL_EXTENSIONS => EXTENSION_STRING,
        _ => {
            context::set_error_no_ctx(GL_INVALID_ENUM, "glGetString");
            b"\0"
        }
    };
    s.as_ptr()
}

/// glGetStringi(GL_EXTENSIONS, i): the same list, one name at a time.
#[no_mangle]
pub unsafe extern "C" fn glGetStringi(name: GLenum, index: GLuint) -> *const GLubyte {
    gl_trace!(glGetStringi);
    if name != GL_EXTENSIONS {
        context::set_error_no_ctx(GL_INVALID_ENUM, "glGetStringi");
        return core::ptr::null();
    }
    if index as usize >= EXTENSION_COUNT {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glGetStringi");
        return core::ptr::null();
    }
    EXTENSION_NAMES[index as usize].as_ptr()
}

/* ---- GL_EXT_debug_marker ---------------------------------------------------- */

/* The extension (registry EXT_debug_marker.txt, "Errors": "None") lets a tool
 * annotate a command stream; with no tool attached the calls do nothing. They
 * read nothing either: `marker` may be null and `length` negative (dEQP's
 * debug_marker.invalid passes both and expects GL_NO_ERROR), and neither the
 * pointer nor the length is looked at, so neither can fault. A pop without a
 * push is not an error either. No context is needed and the error word of the
 * current one is left as it was. */

/// GL_EXT_debug_marker: an event marker. Ignored, never an error.
#[no_mangle]
pub unsafe extern "C" fn glInsertEventMarkerEXT(_length: GLsizei, _marker: *const GLchar) {
    gl_trace!(glInsertEventMarkerEXT);
}

/// GL_EXT_debug_marker: opens a marker group. Ignored, never an error.
#[no_mangle]
pub unsafe extern "C" fn glPushGroupMarkerEXT(_length: GLsizei, _marker: *const GLchar) {
    gl_trace!(glPushGroupMarkerEXT);
}

/// GL_EXT_debug_marker: closes a marker group. Ignored, never an error, also
/// with no group open.
#[no_mangle]
pub unsafe extern "C" fn glPopGroupMarkerEXT() {
    gl_trace!(glPopGroupMarkerEXT);
}

/// Float-valued state: fills `out` and returns the value count, 0 if pname is
/// not float-valued (then glGetIntegerv owns it).
fn get_floatv(c: &GlContext, pname: GLenum, out: &mut [GLfloat; 4]) -> usize {
    match pname {
        GL_DEPTH_RANGE => {
            out[..2].copy_from_slice(&c.depth_range);
            2
        }
        GL_COLOR_CLEAR_VALUE => {
            *out = c.clear_color;
            4
        }
        GL_BLEND_COLOR => {
            *out = c.blend_color;
            4
        }
        GL_DEPTH_CLEAR_VALUE => {
            out[0] = c.clear_depth;
            1
        }
        GL_LINE_WIDTH => {
            out[0] = c.line_width;
            1
        }
        GL_POLYGON_OFFSET_FACTOR => {
            out[0] = c.polygon_offset_factor;
            1
        }
        GL_POLYGON_OFFSET_UNITS => {
            out[0] = c.polygon_offset_units;
            1
        }
        GL_SAMPLE_COVERAGE_VALUE => {
            out[0] = c.sample_coverage_value;
            1
        }
        GL_ALIASED_LINE_WIDTH_RANGE => {
            out[0] = 1.0;
            out[1] = 16.0;
            2
        }
        GL_ALIASED_POINT_SIZE_RANGE => {
            out[0] = 1.0;
            out[1] = POINT_SIZE_MAX;
            2
        }
        _ => 0,
    }
}

/// Whether pname's float state is a normalized value (a colour component, a
/// depth-range value or the depth clear value): glGetIntegerv maps those to
/// the full int range instead of rounding (GL ES 3.0 6.1.2).
fn is_normalized_state(pname: GLenum) -> bool {
    matches!(pname, GL_COLOR_CLEAR_VALUE | GL_BLEND_COLOR | GL_DEPTH_CLEAR_VALUE | GL_DEPTH_RANGE)
}

/// Whether pname's integer state is UNSIGNED. The four stencil masks are the
/// only ones in ES 2.0, and their default is all ones: `glGetIntegerv` hands
/// back the bit pattern (0xFFFFFFFF reads as -1, which is what GL says and
/// what dEQP masks), but `glGetFloatv` and `glGetInteger64v` must widen the
/// value as unsigned -- 4294967295, not -1. dEQP checks exactly that
/// (es2fIntegerStateQueryTests, the `*_getfloat` cases assert the default
/// mask is at least (1 << stencil bits) - 1).
fn is_unsigned_state(pname: GLenum) -> bool {
    matches!(
        pname,
        GL_STENCIL_VALUE_MASK | GL_STENCIL_WRITEMASK | GL_STENCIL_BACK_VALUE_MASK | GL_STENCIL_BACK_WRITEMASK
    )
}

/// [-1, 1] linearly onto the int range: 1.0 is INT_MAX, -1.0 INT_MIN
/// (GL ES 3.0 6.1.2). Values outside are clamped, NaN reads as 0.
fn normalized_to_int(f: GLfloat) -> GLint {
    if f.is_nan() {
        return 0;
    }
    if f >= 1.0 {
        return GLint::MAX;
    }
    if f <= -1.0 {
        return GLint::MIN;
    }
    (f as f64 * GLint::MAX as f64) as GLint
}

/// The glGet* pnames ES 3.0 adds (audit Table A row 20, with the two caps of
/// row 14 and the pixel-store names of row 19). `report-es2` answers them with
/// GL_INVALID_ENUM; the version queries and GL_MAX_DRAW_BUFFERS have their own
/// arms in `get_integerv`.
fn es3_only_pname(pname: GLenum) -> bool {
    es3_only_cap(pname)
        || es3_pixel_store(pname)
        || matches!(
            pname,
            GL_COPY_READ_BUFFER_BINDING
                | GL_COPY_WRITE_BUFFER_BINDING
                | GL_PIXEL_PACK_BUFFER_BINDING
                | GL_PIXEL_UNPACK_BUFFER_BINDING
                | GL_UNIFORM_BUFFER_BINDING
                | GL_SAMPLER_BINDING
                | GL_TRANSFORM_FEEDBACK_BINDING
                | GL_MAX_VARYING_COMPONENTS
                | GL_MAX_VERTEX_OUTPUT_COMPONENTS
                | GL_MAX_FRAGMENT_INPUT_COMPONENTS
                | GL_MAX_VERTEX_UNIFORM_COMPONENTS
                | GL_MAX_FRAGMENT_UNIFORM_COMPONENTS
                | GL_MAX_VERTEX_UNIFORM_BLOCKS
                | GL_MAX_FRAGMENT_UNIFORM_BLOCKS
                | GL_MAX_COMBINED_UNIFORM_BLOCKS
                | GL_MAX_UNIFORM_BUFFER_BINDINGS
                | GL_MAX_UNIFORM_BLOCK_SIZE
                | GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT
                | GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS
                | GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS
                | GL_MAX_SAMPLES
                | GL_NUM_PROGRAM_BINARY_FORMATS
                | GL_PROGRAM_BINARY_FORMATS
                | GL_MAX_ARRAY_TEXTURE_LAYERS
                | GL_MAX_3D_TEXTURE_SIZE
                | GL_MAX_ELEMENT_INDEX
                | GL_MAX_ELEMENTS_VERTICES
                | GL_MAX_ELEMENTS_INDICES
                | GL_MAX_TEXTURE_LOD_BIAS
                | GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS
                | GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS
                | GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS
                | GL_TRANSFORM_FEEDBACK_ACTIVE
                | GL_TRANSFORM_FEEDBACK_PAUSED
                | GL_MIN_PROGRAM_TEXEL_OFFSET
                | GL_MAX_PROGRAM_TEXEL_OFFSET
                | GL_MAX_SERVER_WAIT_TIMEOUT
                | GL_READ_FRAMEBUFFER_BINDING
                | GL_READ_BUFFER
                | GL_TEXTURE_BINDING_2D_ARRAY
                | GL_TEXTURE_BINDING_3D
                | 0x90EB /* GL_MAX_COMPUTE_WORK_GROUP_INVOCATIONS (ES 3.1) */
        )
}

/// The body of glGetIntegerv: fills `out` and returns how many values pname
/// wrote (0 for an unknown pname, after recording GL_INVALID_ENUM, and for
/// the empty GL_PROGRAM_BINARY_FORMATS list).
fn get_integerv(c: &mut GlContext, pname: GLenum, out: &mut [GLint; 4]) -> usize {
    let mut f = [0.0; 4];
    let n = get_floatv(c, pname, &mut f);
    if n != 0 {
        /* GL ES 3.0 6.1.2: normalized state is mapped onto the int range,
         * everything else rounded to the nearest integer */
        let normalized = is_normalized_state(pname);
        for i in 0..n {
            out[i] = if normalized {
                normalized_to_int(f[i])
            } else {
                (f[i] + if f[i] >= 0.0 { 0.5 } else { -0.5 }) as GLint
            };
        }
        return n;
    }

    if crate::REPORT_ES2 && es3_only_pname(pname) {
        c.set_error(GL_INVALID_ENUM, "glGetIntegerv");
        return 0;
    }

    let v: GLint = match pname {
        GL_BLEND
        | GL_CULL_FACE
        | GL_DEPTH_TEST
        | GL_DITHER
        | GL_POLYGON_OFFSET_FILL
        | GL_SAMPLE_ALPHA_TO_COVERAGE
        | GL_SAMPLE_COVERAGE
        | GL_SCISSOR_TEST
        | GL_STENCIL_TEST
        | GL_PRIMITIVE_RESTART_FIXED_INDEX
        | GL_RASTERIZER_DISCARD => {
            gl_trace!(glIsEnabled); /* the C asks the entry point */
            is_enabled(c, pname).unwrap_or(false) as GLint
        }
        /* ES 3.0 6.1's buffer bindings (state_query.integers.*_binding) */
        GL_COPY_READ_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_copy_read_buffer) as GLint,
        GL_COPY_WRITE_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_copy_write_buffer) as GLint,
        GL_PIXEL_PACK_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_pixel_pack_buffer) as GLint,
        GL_PIXEL_UNPACK_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_pixel_unpack_buffer) as GLint,
        GL_UNIFORM_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_uniform_buffer) as GLint,
        GL_TRANSFORM_FEEDBACK_BUFFER_BINDING if !crate::REPORT_ES2 => {
            names::name_of(c, Kind::Buffer, c.bound_transform_feedback_buffer) as GLint
        }
        GL_SAMPLER_BINDING => super::es3::sampler_binding(c) as GLint,
        GL_TRANSFORM_FEEDBACK_BINDING => super::es3::feedback_binding(c) as GLint,
        GL_COLOR_WRITEMASK => {
            *out = c.color_writemask.map(|m| (m != 0.0) as GLint);
            return 4;
        }
        GL_DEPTH_WRITEMASK => c.depth_mask as GLint,
        GL_VERTEX_ARRAY_BINDING /* (_OES) */ => c.bound_vertex_array as GLint,
        GL_STENCIL_CLEAR_VALUE => c.clear_stencil,
        GL_STENCIL_FUNC => c.stencil_func as GLint,
        GL_STENCIL_REF => c.stencil_ref,
        GL_STENCIL_VALUE_MASK => c.stencil_mask as GLint,
        GL_STENCIL_WRITEMASK => c.stencil_writemask as GLint,
        GL_STENCIL_FAIL => c.stencil_sfail as GLint,
        GL_STENCIL_PASS_DEPTH_FAIL => c.stencil_dpfail as GLint,
        GL_STENCIL_PASS_DEPTH_PASS => c.stencil_dppass as GLint,
        GL_STENCIL_BACK_FUNC => c.stencil_back_func as GLint,
        GL_STENCIL_BACK_REF => c.stencil_back_ref,
        GL_STENCIL_BACK_VALUE_MASK => c.stencil_back_mask as GLint,
        GL_STENCIL_BACK_WRITEMASK => c.stencil_back_writemask as GLint,
        GL_STENCIL_BACK_FAIL => c.stencil_back_sfail as GLint,
        GL_STENCIL_BACK_PASS_DEPTH_FAIL => c.stencil_back_dpfail as GLint,
        GL_STENCIL_BACK_PASS_DEPTH_PASS => c.stencil_back_dppass as GLint,
        GL_BLEND_SRC_RGB => c.blend_src as GLint,
        GL_BLEND_DST_RGB => c.blend_dst as GLint,
        GL_BLEND_SRC_ALPHA => c.blend_src_a as GLint,
        GL_BLEND_DST_ALPHA => c.blend_dst_a as GLint,
        GL_BLEND_EQUATION_RGB => c.blend_eq as GLint,
        GL_BLEND_EQUATION_ALPHA => c.blend_eq_a as GLint,
        GL_UNPACK_ALIGNMENT => c.unpack_alignment,
        GL_PACK_ALIGNMENT => c.pack_alignment,
        GL_GENERATE_MIPMAP_HINT => c.generate_mipmap_hint as GLint,
        GL_FRAGMENT_SHADER_DERIVATIVE_HINT => derivative_hint(c) as GLint,
        GL_UNPACK_ROW_LENGTH => c.unpack_row_length,
        GL_UNPACK_IMAGE_HEIGHT => c.unpack_image_height,
        GL_UNPACK_SKIP_ROWS => c.unpack_skip_rows,
        GL_UNPACK_SKIP_PIXELS => c.unpack_skip_pixels,
        GL_UNPACK_SKIP_IMAGES => c.unpack_skip_images,
        GL_PACK_ROW_LENGTH => c.pack_row_length,
        GL_PACK_SKIP_ROWS => c.pack_skip_rows,
        GL_PACK_SKIP_PIXELS => c.pack_skip_pixels,
        GL_SAMPLE_COVERAGE_INVERT => c.sample_coverage_invert as GLint,
        /* the bound framebuffer's: the surface's samples, or a complete
         * framebuffer object's (one-sample renderbuffers in an ES 3.0
         * build; 0 in an ES 2.0 report, as always) */
        GL_SAMPLE_BUFFERS => {
            if c.bound_framebuffer == 0 {
                (c.default_samples > 1) as GLint
            } else {
                (super::framebuffer::framebuffer_samples(c) > 0) as GLint
            }
        }
        GL_SAMPLES => {
            if c.bound_framebuffer == 0 {
                if c.default_samples > 1 {
                    c.default_samples as GLint
                } else {
                    0
                }
            } else {
                super::framebuffer::framebuffer_samples(c) as GLint
            }
        }
        GL_NUM_COMPRESSED_TEXTURE_FORMATS => 3,
        GL_COMPRESSED_TEXTURE_FORMATS => {
            /* ETC1_RGB8_OES, RGB8_ETC2, RGBA8_ETC2_EAC */
            out[..3].copy_from_slice(&[0x8D64, 0x9274, 0x9278]);
            return 3;
        }
        GL_NUM_SHADER_BINARY_FORMATS => 0,
        GL_SHADER_COMPILER => GL_TRUE as GLint,
        GL_MAX_VIEWPORT_DIMS => {
            out[..2].copy_from_slice(&[MAX_VIEWPORT_DIM, MAX_VIEWPORT_DIM]);
            return 2;
        }
        GL_MAX_VERTEX_ATTRIBS => MAX_VERTEX_ATTRIBS as GLint,
        GL_MAX_TEXTURE_IMAGE_UNITS => CAP_TEXTURE_UNITS as GLint,
        GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS => CAP_COMBINED_TEXTURE_UNITS as GLint,
        /* the vertex stage has its own descriptors (Qt Quick 3D reads bone
         * matrices and morph targets from a texture in the vertex shader) */
        GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS => crate::backend::GLES_VS_TEX_UNITS as GLint,
        GL_MAX_TEXTURE_SIZE => 4096,
        GL_MAX_CUBE_MAP_TEXTURE_SIZE => 4096,
        GL_MAX_RENDERBUFFER_SIZE => 4096,
        /* the bindings keep slots; the application knows them by name (gl/names.rs) */
        GL_FRAMEBUFFER_BINDING => names::name_of(c, Kind::Framebuffer, c.bound_framebuffer) as GLint,
        GL_RENDERBUFFER_BINDING => names::name_of(c, Kind::Renderbuffer, c.bound_renderbuffer) as GLint,
        GL_IMPLEMENTATION_COLOR_READ_FORMAT => super::framebuffer::implementation_color_read(c).0 as GLint,
        GL_IMPLEMENTATION_COLOR_READ_TYPE => super::framebuffer::implementation_color_read(c).1 as GLint,
        GL_MAX_VERTEX_UNIFORM_VECTORS => MAX_VERTEX_UNIFORM_VECTORS as GLint,
        GL_MAX_FRAGMENT_UNIFORM_VECTORS => MAX_FRAGMENT_UNIFORM_VECTORS as GLint,
        GL_MAX_VARYING_VECTORS => MAX_VARYING_VECTORS as GLint,
        /* OpenGL ES 3.0 limits. The values describe what this core implements:
         * one colour attachment, no multisampled FBOs (Qt then draws Quick 3D's
         * antialiasingMode: MSAA views single-sampled), no program binaries. */
        /* The three queries that give the version away. An ES 2.0 report has
         * to refuse them: dEQP (and Qt) read GL_MAJOR_VERSION and conclude
         * they are talking to ES 3 whatever glGetString said. */
        GL_MAJOR_VERSION | GL_MINOR_VERSION | GL_NUM_EXTENSIONS if crate::REPORT_ES2 => {
            c.set_error(GL_INVALID_ENUM, "glGetIntegerv");
            return 0;
        }
        GL_MAJOR_VERSION => 3,
        GL_MINOR_VERSION => 0,
        GL_NUM_EXTENSIONS => EXTENSION_COUNT as GLint,
        GL_MAX_VERTEX_UNIFORM_COMPONENTS => MAX_UNIFORM_DWORDS as GLint,
        GL_MAX_FRAGMENT_UNIFORM_COMPONENTS => MAX_UNIFORM_DWORDS as GLint,
        /* four components per varying vector, both stages; per GPU with
         * MAX_VARYING_VECTORS */
        GL_MAX_VARYING_COMPONENTS => (MAX_VARYING_VECTORS * 4) as GLint,
        GL_MAX_VERTEX_OUTPUT_COMPONENTS => context::MAX_VERTEX_OUTPUT_COMPONENTS as GLint,
        GL_MAX_FRAGMENT_INPUT_COMPONENTS => context::MAX_FRAGMENT_INPUT_COMPONENTS as GLint,
        /* Uniform blocks are flattened into each stage's default block and
         * filled from the bound range at draw time (program.rs), the same on
         * both GPUs: a stage takes ES 3.0's twelve, a program
         * PROG_MAX_BLOCKS, and one block at most the stage's whole default
         * block, MAX_UNIFORM_DWORDS -- below ES 3.0's 16384 bytes (RESULTS.md
         * gap list). */
        GL_MAX_VERTEX_UNIFORM_BLOCKS | GL_MAX_FRAGMENT_UNIFORM_BLOCKS => 12,
        GL_MAX_COMBINED_UNIFORM_BLOCKS => super::program::PROG_MAX_BLOCKS as GLint,
        GL_MAX_UNIFORM_BUFFER_BINDINGS => context::MAX_UNIFORM_BUFFER_BINDINGS as GLint,
        GL_MAX_UNIFORM_BLOCK_SIZE => (MAX_UNIFORM_DWORDS * 4) as GLint,
        GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT => 4,
        GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS | GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS => {
            MAX_UNIFORM_DWORDS as GLint
        }
        /* colour attachments one render writes: the end-of-tile programs and
         * the kick's PBE registers carry this many (a G-buffer pass asks for
         * three) */
        /* ES 2.0 core has neither GL_MAX_DRAW_BUFFERS nor GL_DRAW_BUFFERi
         * (they come with GL_EXT_draw_buffers, which is not claimed), so the
         * ES 2.0 report refuses them, and gl_MaxDrawBuffers is 1 in the
         * shader (reflect::builtin_constant). dEQP's fragdata cases read the
         * query to learn whether more than one draw buffer exists; answering
         * 3 sent them down the multiple-buffer path, which an ES 1.00
         * compiler with one gl_FragData element cannot take. */
        GL_MAX_DRAW_BUFFERS if crate::REPORT_ES2 => {
            c.set_error(GL_INVALID_ENUM, "glGetIntegerv");
            return 0;
        }
        GL_MAX_DRAW_BUFFERS => context::max_draw_buffers() as GLint,
        GL_MAX_COLOR_ATTACHMENTS => context::max_color_attachments() as GLint,
        GL_MAX_SAMPLES => 1,
        GL_NUM_PROGRAM_BINARY_FORMATS => 0,
        GL_MAX_ARRAY_TEXTURE_LAYERS => 256,
        GL_MAX_3D_TEXTURE_SIZE => 256,
        GL_MAX_ELEMENT_INDEX => 0xFF_FFFF,
        GL_MAX_ELEMENTS_VERTICES | GL_MAX_ELEMENTS_INDICES => 0xFF_FFFF,
        GL_MAX_TEXTURE_LOD_BIAS => 2,
        /* ES 3.0's minimums; the ES 2.0 report keeps 0 (no capture there) */
        GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS => {
            if crate::REPORT_ES2 { 0 } else { super::tf::TF_MAX_INTERLEAVED_COMPONENTS as GLint }
        }
        GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS => {
            if crate::REPORT_ES2 { 0 } else { super::tf::TF_MAX_SEPARATE_ATTRIBS as GLint }
        }
        GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS => {
            if crate::REPORT_ES2 { 0 } else { super::tf::TF_MAX_SEPARATE_COMPONENTS as GLint }
        }
        /* the bound transform feedback object's state (always false under
         * report-es2); a conformance run reads them to decide whether it
         * must end a capture before resetting the state */
        GL_TRANSFORM_FEEDBACK_ACTIVE => super::tf::is_active(c) as GLint,
        GL_TRANSFORM_FEEDBACK_PAUSED => super::tf::is_paused(c) as GLint,
        GL_MIN_PROGRAM_TEXEL_OFFSET => MIN_PROGRAM_TEXEL_OFFSET as GLint,
        GL_MAX_PROGRAM_TEXEL_OFFSET => MAX_PROGRAM_TEXEL_OFFSET as GLint,
        GL_MAX_SERVER_WAIT_TIMEOUT => 0,
        GL_PROGRAM_BINARY_FORMATS => return 0,
        GL_READ_FRAMEBUFFER_BINDING => {
            names::name_of(c, Kind::Framebuffer, super::framebuffer::read_framebuffer(c)) as GLint
        }
        /* ES 3.0 4.2.1: the bound draw framebuffer's draw buffer i, i below
         * GL_MAX_DRAW_BUFFERS (only buffer 0 was answered, and always as
         * the default mapping, until 2026-09-29) */
        p if !crate::REPORT_ES2
            && (GL_DRAW_BUFFER0..GL_DRAW_BUFFER0 + context::max_draw_buffers() as GLenum).contains(&p) =>
        {
            super::framebuffer::draw_buffer(c, (p - GL_DRAW_BUFFER0) as usize) as GLint
        }
        GL_READ_BUFFER => {
            /* the read framebuffer's (ES 3.0 4.3.1) */
            let read = super::framebuffer::read_framebuffer(c);
            let none = if read == 0 { c.default_read_none } else { c.framebuffer_ref(read).is_some_and(|fb| fb.read_none) };
            if none {
                GL_NONE as GLint
            } else if read == 0 {
                GL_BACK as GLint
            } else {
                let n = c.framebuffer_ref(read).map_or(0, |fb| fb.read_index);
                (GL_COLOR_ATTACHMENT0 + n as GLenum) as GLint
            }
        }
        0x90EB /* GL_MAX_COMPUTE_WORK_GROUP_INVOCATIONS */ => 0, /* ES 3.1; Qt asks, no error wanted */
        GL_ARRAY_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_array_buffer) as GLint,
        GL_ELEMENT_ARRAY_BUFFER_BINDING => names::name_of(c, Kind::Buffer, c.bound_element_buffer) as GLint,
        GL_TEXTURE_BINDING_2D => {
            names::name_of(c, Kind::Texture, c.bound_texture_2d[c.active_texture as usize]) as GLint
        }
        GL_TEXTURE_BINDING_CUBE_MAP => {
            names::name_of(c, Kind::Texture, c.bound_texture_cube[c.active_texture as usize]) as GLint
        }
        GL_TEXTURE_BINDING_2D_ARRAY => {
            names::name_of(c, Kind::Texture, c.bound_texture_2d_array[c.active_texture as usize]) as GLint
        }
        GL_TEXTURE_BINDING_3D => {
            names::name_of(c, Kind::Texture, c.bound_texture_3d[c.active_texture as usize]) as GLint
        }
        GL_CURRENT_PROGRAM => c.cur_program as GLint,
        GL_ACTIVE_TEXTURE => GL_TEXTURE0.wrapping_add(c.active_texture) as GLint,
        GL_VIEWPORT => {
            *out = c.viewport;
            return 4;
        }
        GL_SCISSOR_BOX => {
            *out = c.scissor;
            return 4;
        }
        GL_DEPTH_FUNC => c.depth_func as GLint,
        GL_CULL_FACE_MODE => c.cull_mode as GLint,
        GL_FRONT_FACE => c.front_face as GLint,
        GL_RED_BITS | GL_GREEN_BITS | GL_BLUE_BITS | GL_ALPHA_BITS => 8,
        GL_DEPTH_BITS => 24,
        GL_STENCIL_BITS => 8,
        GL_SUBPIXEL_BITS => 4,
        _ => {
            c.set_error(GL_INVALID_ENUM, "glGetIntegerv");
            return 0;
        }
    };
    out[0] = v;
    1
}

#[no_mangle]
pub unsafe extern "C" fn glGetIntegerv(pname: GLenum, data: *mut GLint) {
    gl_trace!(glGetIntegerv);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let mut iv = [0; 4];
    let n = get_integerv(c, pname, &mut iv);
    for (i, v) in iv[..n].iter().enumerate() {
        // SAFETY: the application provides room for every value pname returns
        // (n of them, none for an unknown pname); any alignment.
        util::app_set(data, i, *v);
    }
}

/// OpenGL ES 3.0 64-bit state: every pname glGetIntegerv answers, widened.
/// The value count matters -- GL_COLOR_WRITEMASK yields four, and a caller
/// that is handed one GLint64 per call got four written into it.
#[no_mangle]
pub unsafe extern "C" fn glGetInteger64v(pname: GLenum, data: *mut GLint64) {
    gl_trace!(glGetInteger64v);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let mut iv = [0; 4];
    let n = get_integerv(c, pname, &mut iv);
    let unsigned = is_unsigned_state(pname);
    for (i, v) in iv[..n].iter().enumerate() {
        /* unsigned state widens as unsigned: the all-ones stencil mask is
         * 4294967295, not -1 */
        let w = if unsigned { *v as u32 as GLint64 } else { *v as GLint64 };
        // SAFETY: the application provides room for every value pname
        // returns (none for an unknown pname); any alignment.
        util::app_set(data, i, w);
    }
}

/// The indexed state of ES 3.0: the uniform-buffer binding points
/// (`GlContext::uniform_buffers`), and the transform-feedback ones, which
/// this library binds nothing to. `Err(GL_INVALID_ENUM)` for a pname that is
/// not indexed -- not a silent forward to the non-indexed query, which wrote
/// as many values as that pname yields into a caller expecting one -- and
/// `Err(GL_INVALID_VALUE)` for an index past the points.
fn indexed_state(c: &GlContext, target: GLenum, index: GLuint) -> Result<GLint64, GLenum> {
    let ubo = |i: GLuint| context::uniform_buffers(c).get(i as usize).copied().ok_or(GL_INVALID_VALUE);
    match target {
        GL_UNIFORM_BUFFER_BINDING => Ok(GLint64::from(names::name_of(c, Kind::Buffer, ubo(index)?.buffer))),
        GL_UNIFORM_BUFFER_START => Ok(ubo(index)?.offset as GLint64),
        GL_UNIFORM_BUFFER_SIZE => Ok(ubo(index)?.size as GLint64),
        GL_TRANSFORM_FEEDBACK_BUFFER_BINDING
        | GL_TRANSFORM_FEEDBACK_BUFFER_START
        | GL_TRANSFORM_FEEDBACK_BUFFER_SIZE
            if !crate::REPORT_ES2 =>
        {
            super::tf::indexed_state(c, target, index)
        }
        GL_TRANSFORM_FEEDBACK_BUFFER_BINDING
        | GL_TRANSFORM_FEEDBACK_BUFFER_START
        | GL_TRANSFORM_FEEDBACK_BUFFER_SIZE => Ok(0),
        _ => Err(GL_INVALID_ENUM),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetIntegeri_v(target: GLenum, index: GLuint, data: *mut GLint) {
    gl_trace!(glGetIntegeri_v);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let v = match indexed_state(c, target, index) {
        Ok(v) => v,
        Err(e) => {
            c.set_error(e, "glGetIntegeri_v");
            return;
        }
    };
    // SAFETY: room for one value (an indexed pname yields one); any alignment.
    unsafe { data.write_unaligned(v as GLint) };
}

#[no_mangle]
pub unsafe extern "C" fn glGetInteger64i_v(target: GLenum, index: GLuint, data: *mut GLint64) {
    gl_trace!(glGetInteger64i_v);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let v = match indexed_state(c, target, index) {
        Ok(v) => v,
        Err(e) => {
            c.set_error(e, "glGetInteger64i_v");
            return;
        }
    };
    // SAFETY: as in glGetIntegeri_v.
    unsafe { data.write_unaligned(v) };
}

#[no_mangle]
pub unsafe extern "C" fn glGetFloatv(pname: GLenum, data: *mut GLfloat) {
    gl_trace!(glGetFloatv);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let mut f = [0.0; 4];
    let mut iv = [0; 4];
    let mut n = get_floatv(c, pname, &mut f);
    if n == 0 {
        /* integer state: as many values as glGetIntegerv wrote (none for an
         * unknown pname: a failing command leaves the buffer alone) */
        gl_trace!(glGetIntegerv); /* the C asks the entry point */
        n = get_integerv(c, pname, &mut iv);
        let unsigned = is_unsigned_state(pname);
        for i in 0..n {
            f[i] = if unsigned { iv[i] as u32 as GLfloat } else { iv[i] as GLfloat };
        }
    }
    for (i, v) in f[..n].iter().enumerate() {
        // SAFETY: as in glGetIntegerv.
        util::app_set(data, i, *v);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetBooleanv(pname: GLenum, data: *mut GLboolean) {
    gl_trace!(glGetBooleanv);
    let Some(c) = context::current() else { return };
    if data.is_null() {
        return;
    }
    let mut f = [0.0; 4];
    let mut iv = [0; 4];
    let mut b = [GL_FALSE_B; 4];
    let mut n = get_floatv(c, pname, &mut f);
    if n != 0 {
        for i in 0..n {
            b[i] = (f[i] != 0.0) as GLboolean;
        }
    } else {
        gl_trace!(glGetIntegerv); /* the C asks the entry point */
        n = get_integerv(c, pname, &mut iv);
        for i in 0..n {
            b[i] = (iv[i] != 0) as GLboolean;
        }
    }
    for (i, v) in b[..n].iter().enumerate() {
        // SAFETY: as in glGetIntegerv (GLboolean needs no alignment, but the
        // helper keeps every application write in one place).
        util::app_set(data, i, *v);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glFinish() {
    gl_trace!(glFinish);
    /* without a current context there is no pending clear to emit, but the
     * backend is still flushed */
    if let Some(c) = context::current() {
        draw::flush_pending_clear(c);
    }
    crate::backend::flush();
    crate::backend::finish(); /* every kick has completed */
}

#[no_mangle]
pub unsafe extern "C" fn glFlush() {
    gl_trace!(glFlush);
    if let Some(c) = context::current() {
        draw::flush_pending_clear(c);
    }
    crate::backend::flush(); /* submit; the frame runs while the app goes on */
}

/// Unit tests: reset the per-context derivative hints (process-global;
/// testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised.
    unsafe {
        *DERIVATIVE_HINT.get() = [GL_DONT_CARE; MAX_CONTEXTS];
    }
}

#[cfg(test)]
mod tests {

    /// The two forms of the extension list say the same thing.
    ///
    /// `glGetString(GL_EXTENSIONS)` reads the string, `glGetStringi` and
    /// `advertises()` read the array, and nothing until now made them agree.
    /// They are edited by hand and separately -- adding
    /// GL_OES_standard_derivatives touched both -- so the failure mode is an
    /// extension the online front end permits and the string never mentions,
    /// or the reverse: a shader that compiles on one path and is refused on
    /// the other, with the application's own query agreeing with neither.
    #[test]
    fn the_extension_string_and_the_name_list_agree() {
        let s = core::str::from_utf8(EXTENSION_STRING.strip_suffix(b"\0").expect("NUL-terminated")).unwrap();
        let from_string: std::vec::Vec<&str> = s.split(' ').filter(|t| !t.is_empty()).collect();
        let from_names: std::vec::Vec<&str> = EXTENSION_NAMES
            .iter()
            .map(|n| core::str::from_utf8(n.strip_suffix(b"\0").expect("NUL-terminated")).unwrap())
            .collect();
        assert_eq!(from_string, from_names, "the string and the array must list the same names in the same order");
        assert_eq!(from_names.len(), EXTENSION_COUNT);
    }

    /// The depth-texture pair is claimed on the ES 2.0 surface of every GPU,
    /// the V3D included, and not on ES 3.0, which has both in core
    /// (DEPTH_TEXTURE_EXTENSIONS).
    #[test]
    fn the_depth_texture_pair_is_claimed_where_it_works() {
        let s = core::str::from_utf8(EXTENSION_STRING.strip_suffix(b"\0").expect("NUL-terminated")).unwrap();
        let listed: std::vec::Vec<&str> = s.split(' ').collect();
        let want = crate::REPORT_ES2;
        assert_eq!(DEPTH_TEXTURE_EXTENSIONS, want);
        for name in ["GL_OES_depth_texture", "GL_OES_packed_depth_stencil"] {
            assert_eq!(listed.contains(&name), want, "{name}");
        }
        assert_eq!(listed.len() + if want { 0 } else { ES2_ONLY_NAMES }, ALL_EXTENSION_NAMES.len());
    }

    /// GL_OES_texture_npot is claimed on the ES 2.0 surface of every GPU
    /// (this test runs in each GPU's build) and not on ES 3.0, which has it
    /// in core; the GL core's NPOT rules follow the claim.
    #[test]
    fn texture_npot_is_claimed_on_the_es2_surface() {
        let s = core::str::from_utf8(EXTENSION_STRING.strip_suffix(b"\0").expect("NUL-terminated")).unwrap();
        let listed: std::vec::Vec<&str> = s.split(' ').collect();
        assert_eq!(NPOT_EXTENSION, crate::REPORT_ES2);
        assert_eq!(listed.contains(&"GL_OES_texture_npot"), crate::REPORT_ES2);
        assert_eq!(extension_listed("GL_OES_texture_npot"), crate::REPORT_ES2);
        /* no build keeps ES 2.0's NPOT rules any more */
        const { assert!(!super::super::texture::NPOT_RESTRICTED) };
    }

    /// On the GC7000 the claim is the part's: the extension is listed
    /// exactly while the hardware's NON_POWER_OF_TWO bit is set, the bit the
    /// etnaviv hwdb entry for the board's part records (the constant's
    /// comment cites the entry; tests/vivante_npot.rs pins the constant).
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_claims_npot_while_the_part_has_the_bit() {
        assert_eq!(NPOT_EXTENSION, crate::REPORT_ES2 && crate::vivante::backend::NON_POWER_OF_TWO);
        assert_eq!(
            extension_listed("GL_OES_texture_npot"),
            crate::REPORT_ES2 && crate::vivante::backend::NON_POWER_OF_TWO
        );
    }

    /// Advertised means the online front end lets a shader require it.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn a_name_in_the_list_is_one_the_front_end_will_accept() {
        assert!(advertises("GL_OES_standard_derivatives"), "39 call sites of dFdx/dFdy/fwidth ship in the tables");
        assert!(!advertises("GL_OES_texture_float"), "not claimed, so a shader requiring it must fail to compile");
    }
    use super::*;
    use crate::testutil::TestGl;
    use std::ffi::CStr;
    use std::string::String;
    use std::vec::Vec;

    /// Sentinel the query helpers pre-fill with: a slot still holding it was not written.
    const UNSET: GLint = -12345;

    fn geti(pname: GLenum) -> [GLint; 4] {
        let mut v = [UNSET; 4];
        // SAFETY: room for the four values the widest pname returns.
        unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
        v
    }

    fn geti1(pname: GLenum) -> GLint {
        let v = geti(pname);
        assert_eq!(v[1], UNSET, "pname 0x{pname:x} wrote more than one value");
        v[0]
    }

    fn getf(pname: GLenum) -> [GLfloat; 4] {
        let mut v = [-777.0; 4];
        // SAFETY: room for the four values the widest pname returns.
        unsafe { glGetFloatv(pname, v.as_mut_ptr()) };
        v
    }

    fn getb(pname: GLenum) -> [GLboolean; 4] {
        let mut v = [0xEE; 4];
        // SAFETY: room for the four values the widest pname returns.
        unsafe { glGetBooleanv(pname, v.as_mut_ptr()) };
        v
    }

    fn gl_str(p: *const GLubyte) -> String {
        assert!(!p.is_null());
        // SAFETY: the library returns NUL-terminated static strings.
        unsafe { CStr::from_ptr(p as *const core::ffi::c_char) }.to_str().unwrap().into()
    }

    /* ---- the ES 3.0 64-bit and indexed queries ------------------------------ */

    #[test]
    fn get_integer64v_writes_as_many_values_as_the_pname_yields() {
        /* GL_COLOR_WRITEMASK yields four. A caller that passes room for what
         * the pname yields must get exactly that many, and a one-value pname
         * must not touch the second slot: a conformance run that handed one
         * GLint64 to a four-value pname died with "stack smashing detected". */
        let gl = TestGl::new(8, 8);
        let get = |pname| {
            let mut v = [-12345i64; 4];
            // SAFETY: room for the four values the widest pname returns.
            unsafe { glGetInteger64v(pname, v.as_mut_ptr()) };
            v
        };
        assert_eq!(get(GL_COLOR_WRITEMASK), [1, 1, 1, 1]);
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 0, 1, 0) };
        assert_eq!(get(GL_COLOR_WRITEMASK), [1, 0, 1, 0]);
        assert_eq!(get(GL_MAX_TEXTURE_SIZE)[1], -12345, "a one-value pname writes one");
        assert_eq!(get(GL_MAX_TEXTURE_SIZE)[0] as GLint, geti1(GL_MAX_TEXTURE_SIZE));
        assert_eq!(get(GL_VIEWPORT), [0, 0, 8, 8]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(get(0x1234), [-12345; 4], "an unknown pname writes nothing");
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn the_indexed_queries_answer_only_indexed_state() {
        /* Nothing can be bound to the indexed points, so every index is
         * unbound. A pname that is not indexed is an error, not a forward to
         * the non-indexed query: forwarding wrote as many values as that
         * pname yields into a caller expecting one. */
        let gl = TestGl::new(8, 8);
        let mut v: GLint = -12345;
        let mut v64: GLint64 = -12345;
        for pname in [
            GL_UNIFORM_BUFFER_BINDING,
            GL_UNIFORM_BUFFER_START,
            GL_UNIFORM_BUFFER_SIZE,
            GL_TRANSFORM_FEEDBACK_BUFFER_BINDING,
            GL_TRANSFORM_FEEDBACK_BUFFER_START,
            GL_TRANSFORM_FEEDBACK_BUFFER_SIZE,
        ] {
            // SAFETY: room for one value each.
            unsafe {
                glGetIntegeri_v(pname, 0, &mut v);
                glGetInteger64i_v(pname, 3, &mut v64);
            }
            assert_eq!((v, v64), (0, 0), "0x{pname:x}");
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{pname:x}");
        }
        v = -12345;
        // SAFETY: room for one value; GL_COLOR_WRITEMASK yields four, which
        // is exactly why it may not be forwarded.
        unsafe { glGetIntegeri_v(GL_COLOR_WRITEMASK, 0, &mut v) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(v, -12345, "nothing was written");
        // SAFETY: room for one value.
        unsafe { glGetInteger64i_v(GL_COLOR_WRITEMASK, 0, &mut v64) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn transform_feedback_is_never_active_and_says_so() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_TRANSFORM_FEEDBACK_ACTIVE), 0);
        assert_eq!(geti1(GL_TRANSFORM_FEEDBACK_PAUSED), 0);
        assert_eq!(getb(GL_TRANSFORM_FEEDBACK_ACTIVE)[0], 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- pure helpers ------------------------------------------------------ */

    #[test]
    fn clampf_clamps_to_unit_range() {
        assert_eq!(clampf(-0.25), 0.0);
        assert_eq!(clampf(0.0), 0.0);
        assert_eq!(clampf(0.5), 0.5);
        assert_eq!(clampf(1.0), 1.0);
        assert_eq!(clampf(1.0001), 1.0);
        assert_eq!(clampf(f32::INFINITY), 1.0);
        assert_eq!(clampf(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn clampf_passes_nan_through() {
        /* known quirk, as in C: the ternary chain leaves NaN alone */
        assert!(clampf(f32::NAN).is_nan());
    }

    #[test]
    fn blend_factor_validity() {
        for f in [
            GL_ZERO,
            GL_ONE,
            GL_SRC_COLOR,
            GL_ONE_MINUS_SRC_COLOR,
            GL_DST_COLOR,
            GL_ONE_MINUS_DST_COLOR,
            GL_SRC_ALPHA,
            GL_ONE_MINUS_SRC_ALPHA,
            GL_DST_ALPHA,
            GL_ONE_MINUS_DST_ALPHA,
            GL_CONSTANT_COLOR,
            GL_ONE_MINUS_CONSTANT_COLOR,
            GL_CONSTANT_ALPHA,
            GL_ONE_MINUS_CONSTANT_ALPHA,
        ] {
            assert!(valid_blend_factor(f, false), "src 0x{f:x}");
            assert!(valid_blend_factor(f, true), "dst 0x{f:x}");
        }
        assert!(valid_blend_factor(GL_SRC_ALPHA_SATURATE, false));
        /* a source factor only in ES 2.0 (4.1.6); either in ES 3.0 (4.1.7).
         * This pinned the ES 2.0 rule for both until 2026-09-24. */
        assert_eq!(valid_blend_factor(GL_SRC_ALPHA_SATURATE, true), !crate::REPORT_ES2);
        assert!(!valid_blend_factor(GL_FUNC_ADD, false));
        assert!(!valid_blend_factor(2, true));
    }

    #[test]
    fn stencil_op_and_face_validity() {
        for op in [GL_KEEP, GL_ZERO, GL_REPLACE, GL_INCR, GL_DECR, GL_INVERT, GL_INCR_WRAP, GL_DECR_WRAP] {
            assert!(valid_stencil_op(op), "0x{op:x}");
        }
        assert!(!valid_stencil_op(GL_ONE));
        assert!(!valid_stencil_op(GL_NEVER));
        assert!(valid_face(GL_FRONT) && valid_face(GL_BACK) && valid_face(GL_FRONT_AND_BACK));
        assert!(!valid_face(GL_CW));
        assert!(!valid_face(0));
    }

    #[test]
    fn normalized_state_maps_the_unit_range_onto_the_int_range() {
        assert!(is_normalized_state(GL_COLOR_CLEAR_VALUE) && is_normalized_state(GL_DEPTH_RANGE));
        assert!(!is_normalized_state(GL_LINE_WIDTH) && !is_normalized_state(GL_VIEWPORT));
        assert_eq!(normalized_to_int(1.0), GLint::MAX);
        assert_eq!(normalized_to_int(-1.0), GLint::MIN);
        assert_eq!(normalized_to_int(2.5), GLint::MAX, "clamped");
        assert_eq!(normalized_to_int(-7.0), GLint::MIN);
        assert_eq!(normalized_to_int(0.0), 0);
        assert_eq!(normalized_to_int(0.5), GLint::MAX / 2, "truncated, as the C conversion does");
        assert_eq!(normalized_to_int(f32::NAN), 0);
    }

    /* ---- viewport / scissor / depth range ---------------------------------- */

    #[test]
    fn viewport_and_scissor_default_to_the_surface() {
        let gl = TestGl::new(40, 30);
        assert_eq!(geti(GL_VIEWPORT), [0, 0, 40, 30]);
        assert_eq!(geti(GL_SCISSOR_BOX), [0, 0, 40, 30]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn viewport_is_recorded_and_queried() {
        let gl = TestGl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe { glViewport(-3, 5, 7, 0) };
        assert_eq!(gl.ctx().viewport, [-3, 5, 7, 0]);
        assert_eq!(geti(GL_VIEWPORT), [-3, 5, 7, 0]);
        assert_eq!(getf(GL_VIEWPORT), [-3.0, 5.0, 7.0, 0.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn viewport_negative_size_is_invalid_value_and_keeps_state() {
        let gl = TestGl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe { glViewport(1, 2, -1, 4) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        // SAFETY: plain GL calls.
        unsafe { glViewport(1, 2, 4, -1) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(geti(GL_VIEWPORT), [0, 0, 16, 16]);
    }

    #[test]
    fn viewport_size_is_clamped_to_max_viewport_dims() {
        let gl = TestGl::new(16, 16);
        let max = geti(GL_MAX_VIEWPORT_DIMS);
        // SAFETY: plain GL calls.
        unsafe { glViewport(0, 0, max[0] + 100, max[1] + 1) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(geti(GL_VIEWPORT), [0, 0, max[0], max[1]]);
    }

    #[test]
    fn scissor_is_recorded_and_negative_size_rejected() {
        let gl = TestGl::new(16, 16);
        // SAFETY: plain GL calls.
        unsafe { glScissor(-2, 3, 0, 9) };
        assert_eq!(geti(GL_SCISSOR_BOX), [-2, 3, 0, 9]);
        // SAFETY: plain GL calls.
        unsafe { glScissor(0, 0, -1, 1) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        // SAFETY: plain GL calls.
        unsafe { glScissor(0, 0, 1, -1) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(gl.ctx().scissor, [-2, 3, 0, 9]);
    }

    #[test]
    fn depth_range_is_clamped_and_may_be_reversed() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(GL_DEPTH_RANGE)[..2], [0.0, 1.0]);
        // SAFETY: plain GL calls.
        unsafe { glDepthRangef(2.0, -1.0) };
        assert_eq!(getf(GL_DEPTH_RANGE)[..2], [1.0, 0.0]);
        // SAFETY: plain GL calls.
        unsafe { glDepthRangef(0.25, 0.75) };
        assert_eq!(gl.ctx().depth_range, [0.25, 0.75]);
        assert_eq!(getf(GL_DEPTH_RANGE), [0.25, 0.75, -777.0, -777.0], "two values written");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- clear values and glClear ------------------------------------------ */

    #[test]
    fn clear_color_is_kept_unclamped_and_queried() {
        /* ES 3.0: the values as given (a float colour buffer clears to them;
         * the backend clamps for a normalized one) */
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(GL_COLOR_CLEAR_VALUE), [0.0; 4]);
        // SAFETY: plain GL calls.
        unsafe { glClearColor(-1.0, 0.25, 1.5, 0.0) };
        assert_eq!(gl.ctx().clear_color, [-1.0, 0.25, 1.5, 0.0]);
        assert_eq!(getf(GL_COLOR_CLEAR_VALUE), [-1.0, 0.25, 1.5, 0.0]);
        assert_eq!(getb(GL_COLOR_CLEAR_VALUE), [1, 1, 1, 0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_depth_is_clamped_and_queried() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(GL_DEPTH_CLEAR_VALUE)[0], 1.0);
        // SAFETY: plain GL calls.
        unsafe { glClearDepthf(-0.5) };
        assert_eq!(getf(GL_DEPTH_CLEAR_VALUE)[0], 0.0);
        // SAFETY: plain GL calls.
        unsafe { glClearDepthf(7.0) };
        assert_eq!(getf(GL_DEPTH_CLEAR_VALUE)[0], 1.0);
        // SAFETY: plain GL calls.
        unsafe { glClearDepthf(0.5) };
        assert_eq!(getf(GL_DEPTH_CLEAR_VALUE), [0.5, -777.0, -777.0, -777.0]);
        assert_eq!(gl.ctx().clear_depth, 0.5);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_stencil_is_recorded_and_queried() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_STENCIL_CLEAR_VALUE), 0);
        // SAFETY: plain GL calls.
        unsafe { glClearStencil(0x7F) };
        assert_eq!(geti1(GL_STENCIL_CLEAR_VALUE), 0x7F);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_integerv_scales_normalized_float_state_to_the_int_range() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 1.0, 0.0);
            glBlendColor(0.0, 1.0, 0.0, 1.0);
        }
        assert_eq!(geti(GL_COLOR_CLEAR_VALUE), [i32::MAX, 0, i32::MAX, 0]);
        assert_eq!(geti(GL_BLEND_COLOR), [0, i32::MAX, 0, i32::MAX]);
        assert_eq!(geti1(GL_DEPTH_CLEAR_VALUE), i32::MAX);
        assert_eq!(geti(GL_DEPTH_RANGE)[..2], [0, i32::MAX]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_records_the_pending_clear_with_values_at_call_time() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearDepthf(0.5);
            glClearStencil(0x1A5);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            /* later changes do not reach the recorded clear */
            glClearColor(1.0, 1.0, 1.0, 1.0);
            glClearDepthf(0.0);
            glClearStencil(3);
        }
        let p = draw::pending_clear();
        assert!(p.color && p.depth && p.stencil);
        assert_eq!(p.rgba, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(p.depth_value, 0.5);
        assert_eq!(p.stencil_value, 0xA5, "the stencil buffer is 8 bits");
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.draws().is_empty(), "glClear does not render on its own");
    }

    /* Two properties of glClear that the i.MX8MP's gl_test cases 58
     * (clearscis) and 59 (clearmask) read back from the framebuffer, written
     * here against the core alone so they need no board and no GPU.
     *
     * ES 2.0 4.2.3: "The pixel ownership test, the scissor test, dithering,
     * and the buffer writemasks affect the operation of glClear" -- as they
     * stand when glClear is called. PendingClear records the clear's *values*
     * at that moment, which is why the test above passes, but the scissor box
     * and the write masks are read later, from whatever the context holds when
     * the clear is finally emitted. */

    #[test]
    fn a_second_clear_does_not_swallow_the_first() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            /* the whole target blue */
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            /* then red inside a box: a different clear, not a replacement */
            glEnable(GL_SCISSOR_TEST);
            glScissor(2, 2, 4, 4);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
            glFlush();
        }
        let draws = gl.draws();
        let clears = draws.iter().filter(|d| d.draw.do_clear).count();
        assert!(
            clears >= 2,
            "both clears must reach the backend: the first covers the target and the second \
             covers a box inside it. Got {clears} clear(s) from {} draw(s); one PendingClear \
             slot means the second glClear overwrote the first and the blue never happened.",
            draws.len()
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_clear_is_not_bounded_by_a_viewport_set_after_it() {
        /* GL ES 2.0 4.2.3 lists what affects glClear: the pixel ownership
         * test, the scissor test, dithering and the write masks. The viewport
         * is not among them -- a clear covers the whole target whatever the
         * viewport is, and whatever it becomes before the clear is emitted.
         *
         * The draw this records still carries the viewport (it is one field
         * of a GlesDraw), and both backends ignore it for a clear: the PVR
         * clear quad is given the whole target as its viewport
         * (`record_clear_quad`) and the GC7000's clip rectangle is the target
         * narrowed by the scissor alone (`clear_clip_rect`). What must hold
         * here is that the core does not turn a later viewport into a clip on
         * the clear. */
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glViewport(0, 0, 4, 4);
            glFlush();
        }
        let draws = gl.draws();
        let cleared = draws.iter().find(|d| d.draw.do_clear).expect("the clear reaches the backend");
        assert!(!cleared.draw.scissor, "a clear is not scissored by a viewport set after it");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_clear_uses_the_write_mask_in_force_when_it_was_called() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glColorMask(0, 0, 0, 0);
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            /* the application puts the mask back before anything is drawn;
             * the clear above was masked off and must stay masked off */
            glColorMask(1, 1, 1, 1);
            glFlush();
        }
        let coloured = gl.draws().iter().filter(|d| d.draw.do_clear).count();
        assert_eq!(
            coloured, 0,
            "a colour clear issued with every channel masked off must change nothing, whatever \
             the mask is when the clear is emitted"
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_of_one_buffer_leaves_the_others_not_pending() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glClear(GL_DEPTH_BUFFER_BIT) };
        let p = draw::pending_clear();
        assert!(!p.color && p.depth && !p.stencil);
        assert_eq!(p.depth_value, 1.0);
        // SAFETY: plain GL calls.
        unsafe { glClear(GL_STENCIL_BUFFER_BIT) };
        let p = draw::pending_clear();
        assert!(!p.color && p.depth && p.stencil, "clears accumulate until a draw or flush takes them");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_with_no_bits_records_nothing() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glClear(0) };
        let p = draw::pending_clear();
        assert!(!p.color && !p.depth && !p.stencil);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_with_an_unknown_bit_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glClear(GL_COLOR_BUFFER_BIT | 0x1) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert!(!draw::pending_clear().color, "a failing command has no effect");
    }

    #[test]
    fn get_queries_write_through_misaligned_pointers() {
        let gl = TestGl::new(40, 30);
        let mut buf = [0u8; 20];
        // SAFETY: the outputs are inside buf at an odd offset; GL_VIEWPORT writes four values.
        unsafe {
            glGetIntegerv(GL_VIEWPORT, buf.as_mut_ptr().add(1) as *mut GLint);
            let got: Vec<GLint> =
                (0..4).map(|i| GLint::from_ne_bytes(buf[1 + i * 4..5 + i * 4].try_into().unwrap())).collect();
            assert_eq!(got, std::vec![0, 0, 40, 30]);
            glGetFloatv(GL_DEPTH_RANGE, buf.as_mut_ptr().add(1) as *mut GLfloat);
            assert_eq!(f32::from_ne_bytes(buf[1..5].try_into().unwrap()), 0.0);
            assert_eq!(f32::from_ne_bytes(buf[5..9].try_into().unwrap()), 1.0);
            glGetBooleanv(GL_COLOR_WRITEMASK, buf.as_mut_ptr().add(1));
            assert_eq!(&buf[1..5], &[1, 1, 1, 1]);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn viewport_clamp_leaves_the_origin_alone() {
        let gl = TestGl::new(16, 16);
        let max = geti(GL_MAX_VIEWPORT_DIMS);
        // SAFETY: plain GL calls.
        unsafe { glViewport(-9, GLint::MAX, GLsizei::MAX, 7) };
        assert_eq!(geti(GL_VIEWPORT), [-9, GLint::MAX, max[0], 7], "the origin is not clamped, the size is");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_rejects_every_undefined_bit() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(0x8000);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glClear(GLbitfield::MAX);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let p = draw::pending_clear();
            assert!(!p.color && !p.depth && !p.stencil);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(draw::pending_clear().color);
    }

    #[test]
    fn the_derivative_hint_is_per_context_and_starts_at_dont_care() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_DONT_CARE as GLint);
        // SAFETY: plain GL calls.
        unsafe {
            glHint(GL_FRAGMENT_SHADER_DERIVATIVE_HINT, GL_FASTEST);
            assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_FASTEST as GLint);
            assert_eq!(gl.ctx().generate_mipmap_hint, GL_DONT_CARE, "the two hints are separate");
            glHint(GL_FRAGMENT_SHADER_DERIVATIVE_HINT, GL_LINEAR);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "the mode is still checked");
            assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_FASTEST as GLint);
        }
        reset_context_hints(gl.ctx().index);
        assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_DONT_CARE as GLint, "a new context starts fresh");
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn min_and_max_blend_equations_are_recorded_like_the_others() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glBlendEquation(GL_MIN);
            assert_eq!((gl.ctx().blend_eq, gl.ctx().blend_eq_a), (GL_MIN, GL_MIN));
            glBlendEquationSeparate(GL_MAX, GL_FUNC_ADD);
        }
        assert_eq!(geti1(GL_BLEND_EQUATION_RGB), GL_MAX as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_ALPHA), GL_FUNC_ADD as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn the_es3_pixel_store_parameters_can_be_queried() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 12) };
        assert_eq!(geti1(GL_UNPACK_ROW_LENGTH), 12);
        assert_eq!(getf(GL_UNPACK_ROW_LENGTH)[0], 12.0);
        for p in [
            GL_UNPACK_SKIP_ROWS,
            GL_UNPACK_SKIP_PIXELS,
            GL_UNPACK_SKIP_IMAGES,
            GL_UNPACK_IMAGE_HEIGHT,
            GL_PACK_ROW_LENGTH,
            GL_PACK_SKIP_ROWS,
            GL_PACK_SKIP_PIXELS,
        ] {
            // SAFETY: plain GL call.
            unsafe { glPixelStorei(p, 3) };
            /* stored and honoured since 2026-09-24 (read back as 0 before) */
            assert_eq!(geti1(p), 3, "0x{p:x}");
            // SAFETY: plain GL call.
            unsafe { glPixelStorei(p, -1) };
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_integerv_of_normalized_state_after_a_clamp() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(-2.0, 0.5, 1.0, f32::NAN);
            glClearDepthf(0.5);
            glDepthRangef(0.25, 1.0);
        }
        assert_eq!(geti(GL_COLOR_CLEAR_VALUE), [GLint::MIN, GLint::MAX / 2, GLint::MAX, 0]);
        assert_eq!(geti1(GL_DEPTH_CLEAR_VALUE), GLint::MAX / 2);
        assert_eq!(geti(GL_DEPTH_RANGE)[..2], [GLint::MAX / 4, GLint::MAX]);
        /* state that is not normalized is still rounded */
        // SAFETY: plain GL call.
        unsafe { glPolygonOffset(0.0, 2.5) };
        assert_eq!(geti1(GL_POLYGON_OFFSET_UNITS), 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glFlush / glFinish -------------------------------------------------- */

    #[test]
    fn flush_submits_without_waiting() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glFlush() };
        assert_eq!((gl.flushes(), gl.finishes()), (1, 0));
        assert!(gl.draws().is_empty(), "nothing pending: no clear-only draw");
    }

    #[test]
    fn finish_flushes_then_waits() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glFinish() };
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
    }

    #[test]
    fn flush_emits_a_pending_clear_as_a_clear_only_draw() {
        let gl = TestGl::new(24, 12);
        // SAFETY: plain GL calls.
        unsafe {
            glViewport(1, 2, 3, 4);
            glClearColor(0.0, 1.0, 0.5, 0.25);
            glClearDepthf(0.75);
            glClearStencil(9);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glClearColor(1.0, 0.0, 0.0, 0.0);
            glFlush();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        let d = &draws[0].draw;
        assert!(d.do_clear && d.do_clear_depth && d.do_clear_stencil);
        assert_eq!(d.clear_rgba, [0.0, 1.0, 0.5, 0.25], "the colour captured by glClear");
        assert_eq!(d.clear_depth, 0.75);
        assert_eq!(d.clear_stencil, 9);
        assert_eq!(d.index_count, 0, "no geometry");
        assert_eq!((d.target_w, d.target_h), (24, 12));
        assert_eq!(d.target_gpu, gl.ctx().default_surface.gpu);
        assert_eq!(d.viewport, [1, 2, 3, 4]);
        assert!(d.color_write);
        assert_eq!(gl.flushes(), 1);
        let p = draw::pending_clear();
        assert!(!p.color && !p.depth && !p.stencil, "the clear was consumed");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_flushed_clear_is_not_emitted_again() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
            glFlush();
        }
        assert_eq!(gl.draws().len(), 1);
        assert_eq!((gl.flushes(), gl.finishes()), (2, 1));
    }

    #[test]
    fn finish_emits_a_colour_only_clear_with_default_depth_stencil() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.25);
            glClear(GL_COLOR_BUFFER_BIT);
            glFinish();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        let d = &draws[0].draw;
        assert!(d.do_clear && !d.do_clear_depth && !d.do_clear_stencil);
        assert_eq!(d.clear_rgba, [0.0; 4]);
        assert_eq!(d.clear_depth, 1.0, "no depth clear requested: the neutral value");
        assert_eq!(d.clear_stencil, 0);
    }

    #[test]
    fn flush_without_a_current_context_still_reaches_the_backend() {
        let gl = TestGl::new(8, 8);
        let ctx = gl.ctx() as *mut GlContext;
        // SAFETY: the context is restored before the guard is dropped.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            glFlush();
            glFinish();
            context::gl_context_set_current(ctx);
        }
        assert_eq!((gl.flushes(), gl.finishes()), (2, 1));
    }

    /* ---- glGetError ---------------------------------------------------------- */

    #[test]
    fn get_error_returns_and_clears() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glGetError(), GL_NO_ERROR);
            glDepthFunc(0);
            assert_eq!(glGetError(), GL_INVALID_ENUM);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        drop(gl);
    }

    #[test]
    fn get_error_keeps_the_first_error() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glLineWidth(0.0); /* INVALID_VALUE */
            glCullFace(0); /* INVALID_ENUM, dropped */
            assert_eq!(glGetError(), GL_INVALID_VALUE);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
        drop(gl);
    }

    #[test]
    fn get_error_without_a_context_is_no_error() {
        let gl = TestGl::new(8, 8);
        let ctx = gl.ctx() as *mut GlContext;
        // SAFETY: the context is restored before the guard is dropped.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            assert_eq!(glGetError(), GL_NO_ERROR);
            context::gl_context_set_current(ctx);
        }
    }

    /* ---- capabilities -------------------------------------------------------- */

    const CAPS: [GLenum; 9] = [
        GL_DEPTH_TEST,
        GL_BLEND,
        GL_CULL_FACE,
        GL_SCISSOR_TEST,
        GL_STENCIL_TEST,
        GL_DITHER,
        GL_POLYGON_OFFSET_FILL,
        GL_SAMPLE_ALPHA_TO_COVERAGE,
        GL_SAMPLE_COVERAGE,
    ];

    #[test]
    fn capabilities_default_to_disabled_except_dither() {
        let gl = TestGl::new(8, 8);
        for cap in CAPS {
            // SAFETY: plain GL calls.
            let on = unsafe { glIsEnabled(cap) };
            assert_eq!(on, (cap == GL_DITHER) as GLboolean, "cap 0x{cap:x}");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn enable_and_disable_each_capability_independently() {
        let gl = TestGl::new(8, 8);
        for cap in CAPS {
            // SAFETY: plain GL calls.
            unsafe {
                for other in CAPS {
                    glDisable(other);
                }
                glEnable(cap);
                for other in CAPS {
                    assert_eq!(glIsEnabled(other), (other == cap) as GLboolean, "0x{other:x} after enabling 0x{cap:x}");
                }
            }
            assert_eq!(geti1(cap), 1);
            assert_eq!(getb(cap)[0], 1);
            assert_eq!(getf(cap)[0], 1.0);
            // SAFETY: plain GL calls.
            unsafe { glDisable(cap) };
            assert_eq!(geti1(cap), 0);
            assert_eq!(getb(cap)[0], 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn enable_records_into_the_context_fields() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glEnable(GL_BLEND);
            glEnable(GL_CULL_FACE);
            glEnable(GL_SCISSOR_TEST);
            glEnable(GL_STENCIL_TEST);
            glDisable(GL_DITHER);
        }
        let c = gl.ctx();
        assert!(c.depth_test && c.blend && c.cull_face && c.scissor_test && c.stencil_test && !c.dither);
    }

    #[test]
    fn enable_disable_is_enabled_reject_unknown_capabilities() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_TEXTURE_2D);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glDisable(0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(glIsEnabled(GL_VIEWPORT), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
    }

    /* ---- depth, cull, line width, polygon offset ----------------------------- */

    #[test]
    fn depth_func_accepts_the_eight_comparisons() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_DEPTH_FUNC), GL_LESS as GLint);
        for f in [GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS] {
            // SAFETY: plain GL calls.
            unsafe { glDepthFunc(f) };
            assert_eq!(geti1(GL_DEPTH_FUNC), f as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn depth_func_rejects_neighbouring_enums() {
        let gl = TestGl::new(8, 8);
        for f in [GL_NEVER - 1, GL_ALWAYS + 1, 0, GL_KEEP] {
            // SAFETY: plain GL calls.
            unsafe { glDepthFunc(f) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{f:x}");
        }
        assert_eq!(gl.ctx().depth_func, GL_LESS);
    }

    #[test]
    fn depth_mask_converts_any_nonzero_to_true() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getb(GL_DEPTH_WRITEMASK)[0], 1);
        // SAFETY: plain GL calls.
        unsafe { glDepthMask(0) };
        assert_eq!(getb(GL_DEPTH_WRITEMASK)[0], 0);
        assert_eq!(geti1(GL_DEPTH_WRITEMASK), 0);
        // SAFETY: plain GL calls.
        unsafe { glDepthMask(0x80) };
        assert_eq!(getb(GL_DEPTH_WRITEMASK)[0], 1);
        assert_eq!(geti1(GL_DEPTH_WRITEMASK), 1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn cull_face_and_front_face() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_CULL_FACE_MODE), GL_BACK as GLint);
        assert_eq!(geti1(GL_FRONT_FACE), GL_CCW as GLint);
        for m in [GL_FRONT, GL_FRONT_AND_BACK, GL_BACK] {
            // SAFETY: plain GL calls.
            unsafe { glCullFace(m) };
            assert_eq!(geti1(GL_CULL_FACE_MODE), m as GLint);
        }
        // SAFETY: plain GL calls.
        unsafe { glFrontFace(GL_CW) };
        assert_eq!(geti1(GL_FRONT_FACE), GL_CW as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);

        // SAFETY: plain GL calls.
        unsafe { glCullFace(GL_CW) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glFrontFace(GL_BACK) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().cull_mode, GL_BACK);
        assert_eq!(gl.ctx().front_face, GL_CW);
    }

    #[test]
    fn line_width_must_be_positive() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(GL_LINE_WIDTH)[0], 1.0);
        // SAFETY: plain GL calls.
        unsafe { glLineWidth(2.5) };
        assert_eq!(getf(GL_LINE_WIDTH)[0], 2.5);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for w in [0.0, -0.0, -1.0] {
            // SAFETY: plain GL calls.
            unsafe { glLineWidth(w) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "{w}");
        }
        assert_eq!(gl.ctx().line_width, 2.5);
    }

    #[test]
    fn float_state_rounds_to_nearest_in_get_integerv() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glLineWidth(2.4);
            glPolygonOffset(2.5, -2.5);
        }
        assert_eq!(geti1(GL_LINE_WIDTH), 2);
        assert_eq!(geti1(GL_POLYGON_OFFSET_FACTOR), 3);
        assert_eq!(geti1(GL_POLYGON_OFFSET_UNITS), -3);
        // SAFETY: plain GL calls.
        unsafe { glPolygonOffset(-0.4, 7.6) };
        assert_eq!(geti1(GL_POLYGON_OFFSET_FACTOR), 0);
        assert_eq!(geti1(GL_POLYGON_OFFSET_UNITS), 8);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn polygon_offset_is_recorded_unclamped() {
        let gl = TestGl::new(8, 8);
        assert_eq!((getf(GL_POLYGON_OFFSET_FACTOR)[0], getf(GL_POLYGON_OFFSET_UNITS)[0]), (0.0, 0.0));
        // SAFETY: plain GL calls.
        unsafe { glPolygonOffset(-1.5, 300.0) };
        assert_eq!(getf(GL_POLYGON_OFFSET_FACTOR)[0], -1.5);
        assert_eq!(getf(GL_POLYGON_OFFSET_UNITS)[0], 300.0);
        assert_eq!(getb(GL_POLYGON_OFFSET_FACTOR)[0], 1, "non-zero float -> GL_TRUE");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn line_and_point_size_ranges() {
        let gl = TestGl::new(8, 8);
        let lw = getf(GL_ALIASED_LINE_WIDTH_RANGE);
        assert_eq!(lw[2], -777.0, "two values");
        assert!(lw[0] <= 1.0 && lw[1] >= 1.0, "the range includes width 1 (spec minimum)");
        let ps = getf(GL_ALIASED_POINT_SIZE_RANGE);
        assert!(ps[0] <= 1.0 && ps[1] >= 1.0);
        /* the maximum is what the GPU draws at (POINT_SIZE_MAX) */
        let want = if cfg!(feature = "gpu-v3d") {
            512.0
        } else if cfg!(feature = "gpu-vivante") {
            64.0
        } else {
            511.0
        };
        assert_eq!(ps[..2], [1.0, want]);
        assert_eq!(geti(GL_ALIASED_LINE_WIDTH_RANGE)[..2], [1, 16]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- sample coverage, hints, colour mask --------------------------------- */

    #[test]
    fn sample_coverage_clamps_value_and_converts_invert() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(GL_SAMPLE_COVERAGE_VALUE)[0], 1.0);
        assert_eq!(getb(GL_SAMPLE_COVERAGE_INVERT)[0], 0);
        // SAFETY: plain GL calls.
        unsafe { glSampleCoverage(1.5, 7) };
        assert_eq!(getf(GL_SAMPLE_COVERAGE_VALUE)[0], 1.0);
        assert_eq!(getb(GL_SAMPLE_COVERAGE_INVERT)[0], 1);
        assert_eq!(geti1(GL_SAMPLE_COVERAGE_INVERT), 1);
        // SAFETY: plain GL calls.
        unsafe { glSampleCoverage(-2.0, 0) };
        assert_eq!(getf(GL_SAMPLE_COVERAGE_VALUE)[0], 0.0);
        assert_eq!(getb(GL_SAMPLE_COVERAGE_INVERT)[0], 0);
        // SAFETY: plain GL calls.
        unsafe { glSampleCoverage(0.5, 0) };
        assert_eq!(gl.ctx().sample_coverage_value, 0.5);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn hint_records_the_mipmap_hint() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_GENERATE_MIPMAP_HINT), GL_DONT_CARE as GLint);
        for m in [GL_FASTEST, GL_NICEST, GL_DONT_CARE] {
            // SAFETY: plain GL calls.
            unsafe { glHint(GL_GENERATE_MIPMAP_HINT, m) };
            assert_eq!(geti1(GL_GENERATE_MIPMAP_HINT), m as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn hint_rejects_bad_mode_and_bad_target() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glHint(GL_GENERATE_MIPMAP_HINT, GL_LINEAR) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glHint(GL_TEXTURE_2D, GL_NICEST) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().generate_mipmap_hint, GL_DONT_CARE);
    }

    #[test]
    fn hint_accepts_the_fragment_shader_derivative_hint() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_DONT_CARE as GLint);
        // SAFETY: plain GL calls.
        unsafe { glHint(GL_FRAGMENT_SHADER_DERIVATIVE_HINT, GL_NICEST) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(geti1(GL_FRAGMENT_SHADER_DERIVATIVE_HINT), GL_NICEST as GLint);
    }

    #[test]
    fn color_mask_converts_to_booleans_per_channel() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getb(GL_COLOR_WRITEMASK), [1, 1, 1, 1]);
        // SAFETY: plain GL calls.
        unsafe { glColorMask(1, 0, 0xFF, 0) };
        assert_eq!(getb(GL_COLOR_WRITEMASK), [1, 0, 1, 0]);
        assert_eq!(geti(GL_COLOR_WRITEMASK), [1, 0, 1, 0]);
        assert_eq!(getf(GL_COLOR_WRITEMASK), [1.0, 0.0, 1.0, 0.0]);
        assert_eq!(gl.ctx().color_writemask, [1.0, 0.0, 1.0, 0.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- blending ------------------------------------------------------------ */

    #[test]
    fn blend_defaults() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_BLEND_SRC_RGB), GL_ONE as GLint);
        assert_eq!(geti1(GL_BLEND_SRC_ALPHA), GL_ONE as GLint);
        assert_eq!(geti1(GL_BLEND_DST_RGB), GL_ZERO as GLint);
        assert_eq!(geti1(GL_BLEND_DST_ALPHA), GL_ZERO as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_RGB), GL_FUNC_ADD as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_ALPHA), GL_FUNC_ADD as GLint);
        assert_eq!(getf(GL_BLEND_COLOR), [0.0; 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn blend_func_sets_rgb_and_alpha_alike() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA) };
        assert_eq!(geti1(GL_BLEND_SRC_RGB), GL_SRC_ALPHA as GLint);
        assert_eq!(geti1(GL_BLEND_SRC_ALPHA), GL_SRC_ALPHA as GLint);
        assert_eq!(geti1(GL_BLEND_DST_RGB), GL_ONE_MINUS_SRC_ALPHA as GLint);
        assert_eq!(geti1(GL_BLEND_DST_ALPHA), GL_ONE_MINUS_SRC_ALPHA as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn blend_func_separate_records_four_factors() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendFuncSeparate(GL_SRC_ALPHA_SATURATE, GL_DST_COLOR, GL_CONSTANT_ALPHA, GL_ONE) };
        let c = gl.ctx();
        assert_eq!(
            (c.blend_src, c.blend_dst, c.blend_src_a, c.blend_dst_a),
            (GL_SRC_ALPHA_SATURATE, GL_DST_COLOR, GL_CONSTANT_ALPHA, GL_ONE)
        );
        assert_eq!(geti1(GL_BLEND_SRC_RGB), GL_SRC_ALPHA_SATURATE as GLint);
        assert_eq!(geti1(GL_BLEND_DST_RGB), GL_DST_COLOR as GLint);
        assert_eq!(geti1(GL_BLEND_SRC_ALPHA), GL_CONSTANT_ALPHA as GLint);
        assert_eq!(geti1(GL_BLEND_DST_ALPHA), GL_ONE as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn blend_func_invalid_factor_in_any_position_changes_nothing() {
        let gl = TestGl::new(8, 8);
        let bad = GL_FUNC_ADD;
        let ok = GL_SRC_ALPHA;
        for args in [[bad, ok, ok, ok], [ok, bad, ok, ok], [ok, ok, bad, ok], [ok, ok, ok, bad]] {
            // SAFETY: plain GL calls.
            unsafe { glBlendFuncSeparate(args[0], args[1], args[2], args[3]) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "{args:x?}");
        }
        // SAFETY: plain GL calls.
        unsafe { glBlendFunc(GL_ONE, 0xBAD) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        let c = gl.ctx();
        assert_eq!((c.blend_src, c.blend_dst, c.blend_src_a, c.blend_dst_a), (GL_ONE, GL_ZERO, GL_ONE, GL_ZERO));
    }

    /// ES 2.0 4.1.6 takes SRC_ALPHA_SATURATE as a source factor only; ES 3.0
    /// 4.1.7 (table 4.2) as either. The rule follows the reported version;
    /// this pinned ES 2.0's for both until 2026-09-24 (dEQP-GLES3
    /// state_query.integers.blend_dst_*).
    #[test]
    fn blend_func_takes_src_alpha_saturate_as_destination_per_version() {
        let gl = TestGl::new(8, 8);
        let want = if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_NO_ERROR };
        // SAFETY: plain GL calls.
        unsafe { glBlendFunc(GL_ONE, GL_SRC_ALPHA_SATURATE) };
        assert_eq!(gl.error(), want);
        // SAFETY: plain GL calls.
        unsafe { glBlendFuncSeparate(GL_ONE, GL_ONE, GL_ONE, GL_SRC_ALPHA_SATURATE) };
        assert_eq!(gl.error(), want);
    }

    #[test]
    fn blend_equation_and_separate() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquation(GL_FUNC_SUBTRACT) };
        assert_eq!(geti1(GL_BLEND_EQUATION_RGB), GL_FUNC_SUBTRACT as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_ALPHA), GL_FUNC_SUBTRACT as GLint);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquationSeparate(GL_FUNC_REVERSE_SUBTRACT, GL_FUNC_ADD) };
        assert_eq!(geti1(GL_BLEND_EQUATION_RGB), GL_FUNC_REVERSE_SUBTRACT as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_ALPHA), GL_FUNC_ADD as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn blend_equation_invalid_mode_changes_nothing() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquation(GL_ONE) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquationSeparate(GL_FUNC_SUBTRACT, GL_SRC_ALPHA) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquationSeparate(0, GL_FUNC_SUBTRACT) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        let c = gl.ctx();
        assert_eq!((c.blend_eq, c.blend_eq_a), (GL_FUNC_ADD, GL_FUNC_ADD));
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn blend_equation_accepts_min_and_max() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendEquationSeparate(GL_MIN, GL_MAX) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(geti1(GL_BLEND_EQUATION_RGB), GL_MIN as GLint);
        assert_eq!(geti1(GL_BLEND_EQUATION_ALPHA), GL_MAX as GLint);
    }

    #[test]
    fn blend_color_is_clamped() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glBlendColor(2.0, -2.0, 0.5, 1.0) };
        assert_eq!(getf(GL_BLEND_COLOR), [1.0, 0.0, 0.5, 1.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- stencil ------------------------------------------------------------- */

    #[test]
    fn stencil_defaults() {
        let gl = TestGl::new(8, 8);
        for (func, r, mask, wmask) in [
            (GL_STENCIL_FUNC, GL_STENCIL_REF, GL_STENCIL_VALUE_MASK, GL_STENCIL_WRITEMASK),
            (GL_STENCIL_BACK_FUNC, GL_STENCIL_BACK_REF, GL_STENCIL_BACK_VALUE_MASK, GL_STENCIL_BACK_WRITEMASK),
        ] {
            assert_eq!(geti1(func), GL_ALWAYS as GLint);
            assert_eq!(geti1(r), 0);
            assert_eq!(geti1(mask) as u32, 0xFFFF_FFFF, "all ones");
            assert_eq!(geti1(wmask) as u32, 0xFFFF_FFFF);
        }
        for op in [
            GL_STENCIL_FAIL,
            GL_STENCIL_PASS_DEPTH_FAIL,
            GL_STENCIL_PASS_DEPTH_PASS,
            GL_STENCIL_BACK_FAIL,
            GL_STENCIL_BACK_PASS_DEPTH_FAIL,
            GL_STENCIL_BACK_PASS_DEPTH_PASS,
        ] {
            assert_eq!(geti1(op), GL_KEEP as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn stencil_func_sets_both_faces() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilFunc(GL_EQUAL, 5, 0x0F) };
        assert_eq!(
            (geti1(GL_STENCIL_FUNC), geti1(GL_STENCIL_REF), geti1(GL_STENCIL_VALUE_MASK)),
            (GL_EQUAL as GLint, 5, 15)
        );
        assert_eq!(
            (geti1(GL_STENCIL_BACK_FUNC), geti1(GL_STENCIL_BACK_REF), geti1(GL_STENCIL_BACK_VALUE_MASK)),
            (GL_EQUAL as GLint, 5, 15)
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn stencil_func_separate_touches_only_the_named_face() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glStencilFuncSeparate(GL_FRONT, GL_LESS, 1, 0x11);
            glStencilFuncSeparate(GL_BACK, GL_GREATER, 2, 0x22);
        }
        let c = gl.ctx();
        assert_eq!((c.stencil_func, c.stencil_ref, c.stencil_mask), (GL_LESS, 1, 0x11));
        assert_eq!((c.stencil_back_func, c.stencil_back_ref, c.stencil_back_mask), (GL_GREATER, 2, 0x22));
        // SAFETY: plain GL calls.
        unsafe { glStencilFuncSeparate(GL_FRONT_AND_BACK, GL_NEVER, 3, 0x33) };
        let c = gl.ctx();
        assert_eq!((c.stencil_func, c.stencil_ref, c.stencil_mask), (GL_NEVER, 3, 0x33));
        assert_eq!((c.stencil_back_func, c.stencil_back_ref, c.stencil_back_mask), (GL_NEVER, 3, 0x33));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn stencil_ref_is_clamped_to_the_8_bit_buffer() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilFunc(GL_ALWAYS, 300, 0xFF) };
        assert_eq!(geti1(GL_STENCIL_REF), 255);
        // SAFETY: plain GL calls.
        unsafe { glStencilFuncSeparate(GL_BACK, GL_ALWAYS, -4, 0xFF) };
        assert_eq!(geti1(GL_STENCIL_BACK_REF), 0);
        assert_eq!(geti1(GL_STENCIL_REF), 255);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn stencil_func_rejects_bad_face_and_func() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilFuncSeparate(GL_CW, GL_LESS, 1, 1) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glStencilFuncSeparate(GL_FRONT, GL_KEEP, 1, 1) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        // SAFETY: plain GL calls.
        unsafe { glStencilFunc(GL_ALWAYS + 1, 1, 1) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        let c = gl.ctx();
        assert_eq!((c.stencil_func, c.stencil_ref, c.stencil_mask), (GL_ALWAYS, 0, 0xFFFF_FFFF));
    }

    #[test]
    fn stencil_op_and_separate() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilOp(GL_ZERO, GL_INCR, GL_DECR_WRAP) };
        for (f, b, want) in [
            (GL_STENCIL_FAIL, GL_STENCIL_BACK_FAIL, GL_ZERO),
            (GL_STENCIL_PASS_DEPTH_FAIL, GL_STENCIL_BACK_PASS_DEPTH_FAIL, GL_INCR),
            (GL_STENCIL_PASS_DEPTH_PASS, GL_STENCIL_BACK_PASS_DEPTH_PASS, GL_DECR_WRAP),
        ] {
            assert_eq!(geti1(f), want as GLint);
            assert_eq!(geti1(b), want as GLint);
        }
        // SAFETY: plain GL calls.
        unsafe {
            glStencilOpSeparate(GL_FRONT, GL_REPLACE, GL_INVERT, GL_INCR_WRAP);
            glStencilOpSeparate(GL_BACK, GL_KEEP, GL_DECR, GL_REPLACE);
        }
        let c = gl.ctx();
        assert_eq!((c.stencil_sfail, c.stencil_dpfail, c.stencil_dppass), (GL_REPLACE, GL_INVERT, GL_INCR_WRAP));
        assert_eq!(
            (c.stencil_back_sfail, c.stencil_back_dpfail, c.stencil_back_dppass),
            (GL_KEEP, GL_DECR, GL_REPLACE)
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn stencil_op_rejects_bad_face_and_ops() {
        let gl = TestGl::new(8, 8);
        let k = GL_KEEP;
        // SAFETY: plain GL calls.
        unsafe { glStencilOpSeparate(0, k, k, k) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        for ops in [[GL_ONE, k, k], [k, GL_ONE, k], [k, k, GL_ONE]] {
            // SAFETY: plain GL calls.
            unsafe { glStencilOp(ops[0], ops[1], ops[2]) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "{ops:x?}");
        }
        let c = gl.ctx();
        assert_eq!((c.stencil_sfail, c.stencil_dpfail, c.stencil_dppass), (GL_KEEP, GL_KEEP, GL_KEEP));
    }

    #[test]
    fn stencil_mask_and_separate() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilMask(0xF0) };
        assert_eq!((geti1(GL_STENCIL_WRITEMASK), geti1(GL_STENCIL_BACK_WRITEMASK)), (0xF0, 0xF0));
        // SAFETY: plain GL calls.
        unsafe {
            glStencilMaskSeparate(GL_FRONT, 0x01);
            glStencilMaskSeparate(GL_BACK, 0x02);
        }
        assert_eq!((geti1(GL_STENCIL_WRITEMASK), geti1(GL_STENCIL_BACK_WRITEMASK)), (1, 2));
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL calls.
        unsafe { glStencilMaskSeparate(GL_CCW, 0xFF) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!((gl.ctx().stencil_writemask, gl.ctx().stencil_back_writemask), (1, 2));
    }

    /* ---- glPixelStorei ------------------------------------------------------- */

    #[test]
    fn pixel_store_alignment_accepts_1_2_4_8() {
        let gl = TestGl::new(8, 8);
        assert_eq!((geti1(GL_UNPACK_ALIGNMENT), geti1(GL_PACK_ALIGNMENT)), (4, 4));
        for a in [1, 2, 8, 4] {
            // SAFETY: plain GL calls.
            unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, a) };
            assert_eq!(geti1(GL_UNPACK_ALIGNMENT), a);
            assert_eq!(geti1(GL_PACK_ALIGNMENT), 4, "pack alignment is separate state");
        }
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_PACK_ALIGNMENT, 1) };
        assert_eq!((geti1(GL_UNPACK_ALIGNMENT), geti1(GL_PACK_ALIGNMENT)), (4, 1));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn pixel_store_alignment_rejects_other_values() {
        let gl = TestGl::new(8, 8);
        for a in [0, 3, 5, 16, -1] {
            // SAFETY: plain GL calls.
            unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, a) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "unpack {a}");
            // SAFETY: plain GL calls.
            unsafe { glPixelStorei(GL_PACK_ALIGNMENT, a) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "pack {a}");
        }
        assert_eq!((gl.ctx().unpack_alignment, gl.ctx().pack_alignment), (4, 4));
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn pixel_store_row_length_is_recorded() {
        let gl = TestGl::new(8, 8);
        assert_eq!(gl.ctx().unpack_row_length, 0);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 37) };
        assert_eq!(gl.ctx().unpack_row_length, 37);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, -1) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(gl.ctx().unpack_row_length, 37);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 0) };
        assert_eq!(gl.ctx().unpack_row_length, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn pixel_store_row_length_can_be_queried() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_UNPACK_ROW_LENGTH, 37) };
        assert_eq!(geti1(GL_UNPACK_ROW_LENGTH), 37);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn pixel_store_unmodelled_es3_parameters_are_accepted() {
        let gl = TestGl::new(8, 8);
        for p in [
            GL_UNPACK_SKIP_ROWS,
            GL_UNPACK_SKIP_PIXELS,
            GL_UNPACK_SKIP_IMAGES,
            GL_UNPACK_IMAGE_HEIGHT,
            GL_PACK_ROW_LENGTH,
            GL_PACK_SKIP_ROWS,
            GL_PACK_SKIP_PIXELS,
        ] {
            // SAFETY: plain GL calls.
            unsafe {
                glPixelStorei(p, 0);
                glPixelStorei(p, 2);
            }
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{p:x}");
            // SAFETY: plain GL calls.
            unsafe { glPixelStorei(p, -1) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "0x{p:x}");
        }
    }

    #[test]
    fn pixel_store_unknown_pname_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glPixelStorei(GL_VIEWPORT, 4) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    /* ---- strings ------------------------------------------------------------- */

    #[test]
    fn get_string_values() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let (vendor, renderer, version, glsl) = unsafe {
            (
                gl_str(glGetString(GL_VENDOR)),
                gl_str(glGetString(GL_RENDERER)),
                gl_str(glGetString(GL_VERSION)),
                gl_str(glGetString(GL_SHADING_LANGUAGE_VERSION)),
            )
        };
        assert!(!vendor.is_empty());
        assert!(!renderer.is_empty());
        /* the formats the ES spec prescribes (6.1.6) */
        let (want_version, want_glsl) = if crate::REPORT_ES2 {
            ("OpenGL ES 2.0", "OpenGL ES GLSL ES 1.00")
        } else {
            ("OpenGL ES 3.0", "OpenGL ES GLSL ES 3.00")
        };
        assert!(version.starts_with(want_version), "{version}");
        assert!(glsl.starts_with(want_glsl), "{glsl}");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn version_string_agrees_with_the_integer_queries() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let version = unsafe { gl_str(glGetString(GL_VERSION)) };
        let want = std::format!("OpenGL ES {}.{}", geti1(GL_MAJOR_VERSION), geti1(GL_MINOR_VERSION));
        assert!(version.starts_with(&want), "{version} vs {want}");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// An ES 2.0 report must not answer the ES 3.0 version queries: a client
    /// that gets a number back believes it whatever the string said (this is
    /// how dEQP and Qt decide which API they have).
    #[test]
    #[cfg(feature = "report-es2")]
    fn es2_report_refuses_the_es3_version_queries() {
        for pname in [GL_MAJOR_VERSION, GL_MINOR_VERSION, GL_NUM_EXTENSIONS] {
            let gl = TestGl::new(8, 8);
            let mut v: GLint = 0x5A5A;
            // SAFETY: plain GL calls; `v` holds one value.
            unsafe { glGetIntegerv(pname, &mut v) };
            assert_eq!(gl.error(), GL_INVALID_ENUM, "pname {pname:#x}");
            assert_eq!(v, 0x5A5A, "pname {pname:#x} wrote a value");
        }
    }

    #[test]
    fn get_string_unknown_name_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glGetString(GL_VIEWPORT) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn extension_list_matches_the_indexed_names() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        let all = unsafe { gl_str(glGetString(GL_EXTENSIONS)) };
        let listed: Vec<&str> = all.split(' ').collect();
        assert_eq!(listed.len(), EXTENSION_COUNT);
        for (i, name) in listed.iter().enumerate() {
            // SAFETY: plain GL calls.
            let indexed = unsafe { gl_str(glGetStringi(GL_EXTENSIONS, i as GLuint)) };
            assert_eq!(&indexed, name);
            assert!(name.starts_with("GL_"), "{name}");
        }
        assert!(listed.contains(&"GL_OES_vertex_array_object"), "vao.rs exports the OES entry points");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// dEQP-GLES2.functional.debug_marker: `supported` reads the string,
    /// `invalid` makes these six calls and `random` mixes pushes, pops and
    /// inserts of arbitrary bytes with length 0 (NUL-terminated), the exact
    /// length, or a length that excludes the NUL. None may raise an error.
    #[test]
    fn debug_marker_is_claimed_and_never_raises_an_error() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; the markers are never read.
        unsafe {
            let all = gl_str(glGetString(GL_EXTENSIONS));
            assert!(all.split(' ').any(|e| e == "GL_EXT_debug_marker"), "{all}");
            /* es2fDebugMarkerTests.cpp InvalidCase */
            glPushGroupMarkerEXT(-1, c"foo".as_ptr());
            glInsertEventMarkerEXT(-1, c"foo".as_ptr());
            glPushGroupMarkerEXT(0, core::ptr::null());
            glInsertEventMarkerEXT(0, core::ptr::null());
            glPushGroupMarkerEXT(-1, core::ptr::null());
            glInsertEventMarkerEXT(-1, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            /* RandomCase's shapes, and pops past the last push */
            let bytes: [GLchar; 4] = [0x41, 0x7f, 0x20, 0];
            glPushGroupMarkerEXT(0, bytes.as_ptr());
            glPushGroupMarkerEXT(3, bytes.as_ptr());
            glInsertEventMarkerEXT(4, bytes.as_ptr());
            for _ in 0..10 {
                glPopGroupMarkerEXT();
            }
            assert_eq!(gl.error(), GL_NO_ERROR);
            /* an error already recorded is left for glGetError */
            glGetString(GL_VIEWPORT);
            glPopGroupMarkerEXT();
            glInsertEventMarkerEXT(-1, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
    }

    #[test]
    fn get_stringi_index_out_of_range_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let n = EXTENSION_COUNT as GLuint;
        // SAFETY: plain GL calls.
        unsafe {
            assert!(glGetStringi(GL_EXTENSIONS, n).is_null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert!(glGetStringi(GL_EXTENSIONS, GLuint::MAX).is_null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn get_stringi_other_name_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { assert!(glGetStringi(GL_VENDOR, 0).is_null()) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    /* ---- implementation limits and bindings ---------------------------------- */

    /// The budgets the link packs against (`shaderc::packing`) are the
    /// values the queries answer with: one constant each, read by both.
    #[test]
    fn the_link_budgets_are_the_advertised_limits() {
        use crate::shaderc::packing::Budget;
        let _gl = TestGl::new(8, 8);
        for (budget, q) in [
            (Budget::VertexUniforms, GL_MAX_VERTEX_UNIFORM_VECTORS),
            (Budget::FragmentUniforms, GL_MAX_FRAGMENT_UNIFORM_VECTORS),
            (Budget::Varyings, GL_MAX_VARYING_VECTORS),
        ] {
            assert_eq!(geti1(q), budget.limit().1 as GLint, "{budget:?}");
        }
    }

    /// The ES 3.0 limits this GPU reports, pinned per GPU as min(hardware,
    /// implemented). A value below ES 3.0's minimum (Tables 6.28-6.34, read
    /// from the 3.0.6 text) is a listed gap in tools/cts/RESULTS.md, "ES 3.0
    /// conformance gaps"; raising one moves both this pin and that row.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn the_es3_limits_are_what_this_gpu_carries_and_programs() {
        let _gl = TestGl::new(8, 8);
        let vivante = cfg!(feature = "gpu-vivante");
        /* GC7000: 15 varyings reach the fragment stage (16 io rows, one the
         * position); PowerVR: 8 until the USC's limit is measured; V3D: 8,
         * ES 2.0's minimum, until more are measured on the board */
        let vectors = if vivante { 15 } else { 8 };
        assert_eq!(geti1(GL_MAX_VARYING_VECTORS), vectors);
        for q in [GL_MAX_VARYING_COMPONENTS, GL_MAX_VERTEX_OUTPUT_COMPONENTS, GL_MAX_FRAGMENT_INPUT_COMPONENTS] {
            assert_eq!(geti1(q), vectors * 4, "{q:#x}");
        }
        /* every GPU: three (every colour attachment); the query itself is
         * ES 3.0's (see es2_has_no_draw_buffer_queries) */
        if !crate::REPORT_ES2 {
            assert_eq!(geti1(GL_MAX_DRAW_BUFFERS), 3);
        }
        assert_eq!(geti1(GL_MAX_COLOR_ATTACHMENTS), 3);
        /* Uniform blocks, the same on both GPUs (flattened into the default
         * block): twelve a stage, 24 a program, 24 binding points, and one
         * block no larger than a stage's default block (6144 bytes, below
         * ES 3.0's 16384). The combined components stay the default block's
         * 1536, which the blocks share -- below ES 3.0's formula minimum. */
        assert_eq!(geti1(GL_MAX_VERTEX_UNIFORM_BLOCKS), 12);
        assert_eq!(geti1(GL_MAX_FRAGMENT_UNIFORM_BLOCKS), 12);
        assert_eq!(geti1(GL_MAX_COMBINED_UNIFORM_BLOCKS), 24);
        assert_eq!(geti1(GL_MAX_UNIFORM_BUFFER_BINDINGS), 24);
        assert_eq!(geti1(GL_MAX_UNIFORM_BLOCK_SIZE), 6144);
        assert_eq!(geti1(GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS), 1536);
        assert_eq!(geti1(GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS), 1536);
        assert_eq!(geti1(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT), 4);
    }

    /// ES 2.0 core has no GL_MAX_DRAW_BUFFERS and no GL_DRAW_BUFFERi: an ES
    /// 2.0 report refuses both with GL_INVALID_ENUM and leaves the output
    /// untouched (dEQP shaders.fragdata.* reads the first to learn how many
    /// draw buffers exist), and an ES 3.0 build answers them. The shader's
    /// gl_MaxDrawBuffers follows (lower.rs `the_implementation_constants`).
    #[test]
    fn draw_buffer_queries_are_es3_only() {
        let gl = TestGl::new(8, 8);
        let mut pnames = std::vec![GL_MAX_DRAW_BUFFERS];
        pnames.extend((0..context::max_draw_buffers() as GLenum + 1).map(|i| GL_DRAW_BUFFER0 + i));
        for p in pnames {
            let mut v: GLint = -777;
            // SAFETY: one GLint out.
            unsafe { glGetIntegerv(p, &mut v) };
            let in_range = p == GL_MAX_DRAW_BUFFERS || p < GL_DRAW_BUFFER0 + context::max_draw_buffers() as GLenum;
            if crate::REPORT_ES2 {
                assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{p:x} in an ES 2.0 report");
                assert_eq!(v, -777, "0x{p:x}: nothing written");
            } else if in_range {
                assert_eq!(gl.error(), GL_NO_ERROR, "0x{p:x}");
                assert_ne!(v, -777, "0x{p:x}");
            } else {
                /* the first name past the draw buffers is no pname at all */
                assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{p:x} past GL_MAX_DRAW_BUFFERS");
            }
        }
        /* the attachment points stay: ES 2.0 reaches them through
         * GL_NV_fbo_color_attachments, which the list claims */
        assert_eq!(geti1(GL_MAX_COLOR_ATTACHMENTS), context::max_color_attachments() as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn limits_match_the_context_capacities() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_MAX_VERTEX_ATTRIBS) as usize, gl.ctx().attribs.len());
        assert_eq!(geti1(GL_MAX_VERTEX_ATTRIBS) as usize, gl.ctx().current_attrib.len());
        /* the units are the combined limit; a fragment shader's samplers
         * are fewer (ES 2.0 6.2: each stage under its own limit) */
        assert_eq!(geti1(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS) as usize, gl.ctx().bound_texture_2d.len());
        /* 32 on every GPU: a unit is GL state only, the backends take
         * compacted per-stage bindings (V3D and PowerVR 16 + 16, GC7000
         * 16 + 16), so no GPU table bounds it */
        assert_eq!(geti1(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS), 32, "ES 3.0's minimum (Table 6.32)");
        /* 16 on every GPU: each backend's fragment table is GLES_MAX_TEX_BINDS */
        assert_eq!(geti1(GL_MAX_TEXTURE_IMAGE_UNITS), 16, "ES 3.0 asks for sixteen (upstream c470df8)");
        assert_eq!(geti1(GL_MAX_TEXTURE_IMAGE_UNITS) as usize, crate::backend::GLES_MAX_TEX_BINDS);
        /* the vertex stage has descriptors of its own (upstream 0a57cc6) */
        assert_eq!(geti1(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS) as usize, crate::backend::GLES_VS_TEX_UNITS);
        /* 16 on every GPU (backend.rs); the GC7000's was 8 until 2026-10-01 */
        assert_eq!(geti1(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS), 16);
        /* colour outputs one draw writes, per GPU; attachment points below */
        if !crate::REPORT_ES2 {
            assert_eq!(geti1(GL_MAX_DRAW_BUFFERS) as usize, context::max_draw_buffers());
        }
        assert_eq!(geti1(GL_MAX_COLOR_ATTACHMENTS) as usize, context::max_color_attachments());
        /* the blocks are sized at link time, capped at this limit */
        assert_eq!(geti1(GL_MAX_VERTEX_UNIFORM_COMPONENTS) as usize, MAX_UNIFORM_DWORDS);
        assert_eq!(geti1(GL_MAX_FRAGMENT_UNIFORM_COMPONENTS) as usize, MAX_UNIFORM_DWORDS);
        assert_eq!(geti1(GL_MAX_VERTEX_UNIFORM_VECTORS) * 4, geti1(GL_MAX_VERTEX_UNIFORM_COMPONENTS));
        assert_eq!(geti1(GL_MAX_FRAGMENT_UNIFORM_VECTORS) * 4, geti1(GL_MAX_FRAGMENT_UNIFORM_COMPONENTS));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The GC7000's texture unit limits: sixteen vertex units, 16 + 16
    /// within the combined 32.
    #[test]
    #[cfg(feature = "gpu-vivante")]
    fn the_gc7000_answers_sixteen_vertex_texture_units() {
        let gl = TestGl::new(8, 8);
        let limits =
            [GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, GL_MAX_TEXTURE_IMAGE_UNITS, GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS];
        assert_eq!(limits.map(geti1), [16, 16, 32]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn max_texture_size_fits_the_mip_level_capacity() {
        let gl = TestGl::new(8, 8);
        /* a full chain of a 2^k texture has k + 1 levels */
        let max = geti1(GL_MAX_TEXTURE_SIZE) as u32;
        assert!(max.is_power_of_two());
        assert_eq!(max.trailing_zeros() as usize + 1, context::CAP_MIP_LEVELS);
        assert_eq!(geti(GL_MAX_VIEWPORT_DIMS), [max as GLint, max as GLint, UNSET, UNSET]);
        assert_eq!(geti1(GL_MAX_RENDERBUFFER_SIZE), max as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn compressed_format_list_has_the_advertised_length() {
        let gl = TestGl::new(8, 8);
        let n = geti1(GL_NUM_COMPRESSED_TEXTURE_FORMATS);
        let f = geti(GL_COMPRESSED_TEXTURE_FORMATS);
        assert_eq!(n, 3);
        assert_eq!(
            f,
            [GL_ETC1_RGB8_OES as GLint, GL_COMPRESSED_RGB8_ETC2 as GLint, GL_COMPRESSED_RGBA8_ETC2_EAC as GLint, UNSET]
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn program_binary_formats_is_an_empty_list() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_NUM_PROGRAM_BINARY_FORMATS), 0);
        assert_eq!(geti(GL_PROGRAM_BINARY_FORMATS), [UNSET; 4], "nothing written");
        assert_eq!(geti1(GL_NUM_SHADER_BINARY_FORMATS), 0);
        assert_eq!(getb(GL_SHADER_COMPILER)[0], 1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn framebuffer_bits_of_the_rgba8_d24s8_target() {
        let gl = TestGl::new(8, 8);
        for p in [GL_RED_BITS, GL_GREEN_BITS, GL_BLUE_BITS, GL_ALPHA_BITS, GL_STENCIL_BITS] {
            assert_eq!(geti1(p), 8);
        }
        assert_eq!(geti1(GL_DEPTH_BITS), 24);
        assert!(geti1(GL_SUBPIXEL_BITS) >= 4, "spec minimum");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sample_queries_follow_the_default_surface() {
        let gl = TestGl::new(8, 8);
        assert_eq!((geti1(GL_SAMPLE_BUFFERS), geti1(GL_SAMPLES)), (0, 0));
        gl.ctx().default_samples = 4;
        assert_eq!((geti1(GL_SAMPLE_BUFFERS), geti1(GL_SAMPLES)), (1, 4));
        /* not the surface's: an FBO has sample buffers only when complete
         * with one-sample renderbuffers (tests/es3_gl_core.rs) */
        gl.ctx().bound_framebuffer = 1;
        assert_eq!((geti1(GL_SAMPLE_BUFFERS), geti1(GL_SAMPLES)), (0, 0));
        gl.ctx().bound_framebuffer = 0;
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn binding_queries_report_the_context_bindings() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti1(GL_ACTIVE_TEXTURE), GL_TEXTURE0 as GLint);
        if !crate::REPORT_ES2 {
            assert_eq!(geti1(GL_DRAW_BUFFER0), GL_BACK as GLint);
        }
        assert_eq!(geti1(GL_READ_BUFFER), GL_BACK as GLint);
        for p in [
            GL_ARRAY_BUFFER_BINDING,
            GL_ELEMENT_ARRAY_BUFFER_BINDING,
            GL_TEXTURE_BINDING_2D,
            GL_CURRENT_PROGRAM,
            GL_FRAMEBUFFER_BINDING,
            GL_READ_FRAMEBUFFER_BINDING,
            GL_RENDERBUFFER_BINDING,
            GL_VERTEX_ARRAY_BINDING,
        ] {
            assert_eq!(geti1(p), 0, "0x{p:x}");
        }
        {
            let c = gl.ctx();
            c.bound_array_buffer = 3;
            c.bound_element_buffer = 4;
            c.active_texture = 2;
            c.bound_texture_2d[2] = 9;
            c.cur_program = 5;
            c.bound_framebuffer = 6;
            c.bound_renderbuffer = 7;
            c.bound_vertex_array = 8;
        }
        assert_eq!(geti1(GL_ARRAY_BUFFER_BINDING), 3);
        assert_eq!(geti1(GL_ELEMENT_ARRAY_BUFFER_BINDING), 4);
        assert_eq!(geti1(GL_ACTIVE_TEXTURE), (GL_TEXTURE0 + 2) as GLint);
        assert_eq!(geti1(GL_TEXTURE_BINDING_2D), 9, "the active unit's binding");
        assert_eq!(geti1(GL_CURRENT_PROGRAM), 5);
        assert_eq!(geti1(GL_FRAMEBUFFER_BINDING), 6);
        assert_eq!(geti1(GL_READ_FRAMEBUFFER_BINDING), 6);
        assert_eq!(geti1(GL_RENDERBUFFER_BINDING), 7);
        assert_eq!(geti1(GL_VERTEX_ARRAY_BINDING), 8);
        if !crate::REPORT_ES2 {
            assert_eq!(geti1(GL_DRAW_BUFFER0), GL_COLOR_ATTACHMENT0 as GLint);
        }
        assert_eq!(geti1(GL_READ_BUFFER), GL_COLOR_ATTACHMENT0 as GLint);
        /* undo the white-box bindings: destroy must not see objects that do not exist */
        let c = gl.ctx();
        c.bound_framebuffer = 0;
        c.cur_program = 0;
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- the query entry points themselves ----------------------------------- */

    #[test]
    fn queries_with_a_null_pointer_do_nothing() {
        let gl = TestGl::new(8, 8);
        // SAFETY: null is explicitly tolerated.
        unsafe {
            glGetIntegerv(GL_VIEWPORT, core::ptr::null_mut());
            glGetFloatv(GL_VIEWPORT, core::ptr::null_mut());
            glGetBooleanv(GL_VIEWPORT, core::ptr::null_mut());
            glGetIntegerv(0xBAD, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_integerv_unknown_pname_is_invalid_enum_and_writes_nothing() {
        let gl = TestGl::new(8, 8);
        assert_eq!(geti(0xBAD), [UNSET; 4]);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn get_floatv_and_booleanv_unknown_pname_is_invalid_enum_and_writes_nothing() {
        /* the C library (and the port before this) wrote one zero: a failing command has no effect */
        let gl = TestGl::new(8, 8);
        assert_eq!(getf(0xBAD), [-777.0; 4]);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(getb(0xBAD), [0xEE; 4]);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(getf(GL_PROGRAM_BINARY_FORMATS), [-777.0; 4], "an empty list writes nothing either");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_floatv_converts_integer_state() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe { glStencilFunc(GL_ALWAYS, 7, 0xFF) };
        assert_eq!(getf(GL_STENCIL_REF), [7.0, -777.0, -777.0, -777.0]);
        assert_eq!(getf(GL_DEPTH_FUNC)[0], GL_LESS as GLfloat);
        assert_eq!(getf(GL_MAX_VIEWPORT_DIMS), [4096.0, 4096.0, -777.0, -777.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_booleanv_converts_zero_and_nonzero() {
        let gl = TestGl::new(8, 8);
        assert_eq!(getb(GL_DEPTH_RANGE), [0, 1, 0xEE, 0xEE]);
        assert_eq!(getb(GL_STENCIL_REF)[0], 0);
        assert_eq!(getb(GL_DEPTH_FUNC)[0], 1, "a non-zero enum is GL_TRUE");
        // SAFETY: plain GL calls.
        unsafe { glViewport(0, 3, 0, 5) };
        assert_eq!(getb(GL_VIEWPORT), [0, 1, 0, 1]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn setters_without_a_current_context_are_ignored() {
        let gl = TestGl::new(8, 8);
        let ctx = gl.ctx() as *mut GlContext;
        let mut v = [UNSET; 4];
        // SAFETY: the context is restored before the guard is dropped.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            glViewport(1, 1, 1, 1);
            glEnable(GL_BLEND);
            glDepthFunc(0);
            glClear(GL_COLOR_BUFFER_BIT);
            glGetIntegerv(GL_VIEWPORT, v.as_mut_ptr());
            assert_eq!(glIsEnabled(GL_DITHER), GL_FALSE_B);
            context::gl_context_set_current(ctx);
        }
        assert_eq!(v, [UNSET; 4]);
        assert_eq!(gl.ctx().viewport, [0, 0, 8, 8]);
        assert!(!gl.ctx().blend);
        assert!(!draw::pending_clear().color);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
    /* ---- upstream 7e76640 -------------------------------------------------------------------------- */

    /// ES 3.0 6.1's buffer, sampler and transform-feedback bindings through
    /// glGetIntegerv (dEQP-GLES3 state_query.integers.*_binding), and
    /// GL_SRC_ALPHA_SATURATE as a destination factor, which ES 3.0 4.1.7
    /// allows and ES 2.0 4.1.6 does not (state_query.integers.blend_dst_*).
    /// The bindings were GL_INVALID_ENUM until 2026-09-24.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn es3_bindings_and_the_saturate_destination_factor_are_queryable() {
        use crate::gl::buffer::{glBindBuffer, glGenBuffers};
        let gl = TestGl::new(8, 8);
        let mut b: GLuint = 0;
        let mut s: GLuint = 0;
        let mut tf: GLuint = 0;
        // SAFETY: plain GL calls; each out pointer is one name.
        unsafe {
            glGenBuffers(1, &mut b);
            for (target, pname) in [
                (GL_COPY_READ_BUFFER, GL_COPY_READ_BUFFER_BINDING),
                (GL_COPY_WRITE_BUFFER, GL_COPY_WRITE_BUFFER_BINDING),
                (GL_PIXEL_PACK_BUFFER, GL_PIXEL_PACK_BUFFER_BINDING),
                (GL_PIXEL_UNPACK_BUFFER, GL_PIXEL_UNPACK_BUFFER_BINDING),
                (GL_UNIFORM_BUFFER, GL_UNIFORM_BUFFER_BINDING),
            ] {
                assert_eq!(geti1(pname), 0, "{pname:#x} initially");
                glBindBuffer(target, b);
                assert_eq!(geti1(pname) as GLuint, b, "{pname:#x}");
                glBindBuffer(target, 0);
            }
            crate::gl::es3::glGenSamplers(1, &mut s);
            crate::gl::es3::glBindSampler(0, s);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(geti1(GL_SAMPLER_BINDING) as GLuint, s);
            crate::gl::es3::glGenTransformFeedbacks(1, &mut tf);
            crate::gl::es3::glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
            assert_eq!(geti1(GL_TRANSFORM_FEEDBACK_BINDING) as GLuint, tf);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glBlendFunc(GL_ONE, GL_SRC_ALPHA_SATURATE);
            if crate::REPORT_ES2 {
                assert_eq!(gl.error(), GL_INVALID_ENUM, "a source factor only in ES 2.0");
            } else {
                assert_eq!(gl.error(), GL_NO_ERROR);
                assert_eq!(geti1(GL_BLEND_DST_RGB), GL_SRC_ALPHA_SATURATE as GLint);
            }
        }
    }

    /// The two ES 3.0 caps are recorded and queryable through glIsEnabled
    /// and glGetIntegerv (ES 3.0 6.1.1; dEQP-GLES3 state_query.boolean.
    /// primitive_restart_fixed_index, rasterizer_discard). Until 2026-09-24
    /// both were accepted and dropped, glIsEnabled refusing them with
    /// GL_INVALID_ENUM (upstream's quirk, pinned here as such); the draw
    /// honours the restart (`draw::tests::primitive_restart_*`), and the
    /// discard waits for transform feedback.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn es3_caps_are_recorded_and_queryable() {
        let gl = TestGl::new(8, 8);
        for cap in [GL_PRIMITIVE_RESTART_FIXED_INDEX, GL_RASTERIZER_DISCARD] {
            // SAFETY: plain GL calls.
            unsafe {
                assert_eq!(glIsEnabled(cap), GL_FALSE_B, "initially off");
                glEnable(cap);
                assert_eq!(gl.error(), GL_NO_ERROR);
                assert_eq!(glIsEnabled(cap), 1);
                assert_eq!(geti1(cap), 1);
                glDisable(cap);
                assert_eq!(glIsEnabled(cap), GL_FALSE_B);
                assert_eq!(geti1(cap), 0);
            }
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn cube_and_array_texture_bindings_are_queryable_per_unit() {
        let gl = TestGl::new(8, 8);
        let mut t = [0 as GLuint; 2];
        // SAFETY: valid out pointers, plain GL calls.
        unsafe {
            crate::gl::texture::glGenTextures(2, t.as_mut_ptr());
            crate::gl::texture::glActiveTexture(GL_TEXTURE2);
            crate::gl::texture::glBindTexture(GL_TEXTURE_CUBE_MAP, t[0]);
            crate::gl::texture::glBindTexture(GL_TEXTURE_2D_ARRAY, t[1]);
        }
        assert_eq!(geti(GL_TEXTURE_BINDING_CUBE_MAP)[0], t[0] as GLint);
        assert_eq!(geti(GL_TEXTURE_BINDING_2D_ARRAY)[0], t[1] as GLint);
        assert_eq!(geti(GL_TEXTURE_BINDING_2D)[0], 0);
        // SAFETY: plain GL call.
        unsafe { crate::gl::texture::glActiveTexture(GL_TEXTURE0) };
        assert_eq!(geti(GL_TEXTURE_BINDING_CUBE_MAP)[0], 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// dEQP `es2fIntegerStateQueryTests` reads every stencil mask through all
    /// three getters. The masks are UNSIGNED state, and their default is all
    /// ones: glGetIntegerv hands back the bit pattern (-1), but glGetFloatv
    /// and glGetInteger64v must widen it as unsigned. A signed widening gave
    /// -1.0, and the suite asserts the default is at least (1 << bits) - 1.
    #[test]
    fn the_stencil_masks_widen_as_unsigned() {
        const MASKS: [GLenum; 4] =
            [GL_STENCIL_VALUE_MASK, GL_STENCIL_WRITEMASK, GL_STENCIL_BACK_VALUE_MASK, GL_STENCIL_BACK_WRITEMASK];
        let gl = TestGl::new(8, 8);
        for pname in MASKS {
            let mut f: GLfloat = 0.0;
            let mut i64v: GLint64 = 0;
            // SAFETY: plain GL calls; each out pointer holds one value.
            unsafe {
                glGetFloatv(pname, &mut f);
                glGetInteger64v(pname, &mut i64v);
            }
            assert_eq!(f, 4294967295.0, "glGetFloatv of {pname:#x}");
            assert_eq!(i64v, 4294967295, "glGetInteger64v of {pname:#x}");
            /* glGetIntegerv keeps the bit pattern, which is what GL says */
            assert_eq!(geti1(pname), -1, "glGetIntegerv of {pname:#x}");
            assert!(f >= 255.0, "at least (1 << stencil bits) - 1");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* a mask the application set reads back as itself in all three */
        // SAFETY: plain GL calls.
        unsafe { glStencilMask(0x0f) };
        let mut f: GLfloat = 0.0;
        // SAFETY: `f` holds one value.
        unsafe { glGetFloatv(GL_STENCIL_WRITEMASK, &mut f) };
        assert_eq!(f, 15.0);
        assert_eq!(geti1(GL_STENCIL_WRITEMASK), 15);

        /* and a signed piece of state next door is untouched */
        assert_eq!(geti1(GL_STENCIL_REF), 0);
    }
}

#[cfg(test)]
mod gl_test_reset_state_tests {
    use super::*;
    use crate::gl::buffer::glBindBuffer;
    use crate::gl::framebuffer::{glBindFramebuffer, glBindRenderbuffer};
    use crate::gl::program::glUseProgram;
    use crate::gl::texture::{glActiveTexture, glBindTexture};
    use crate::gl::vao::glBindVertexArrayOES;
    use crate::gl::vertex::glDisableVertexAttribArray;
    use crate::testutil::TestGl;

    /// gl_test.c's reset_state, call for call (driver/tools/pvrlab/gl_test.c):
    /// every case on every board starts from it, and the GL core must answer
    /// every query it makes without an error.
    ///
    /// This could NOT see the GL_INVALID_ENUM the boards showed: that was
    /// gl_context.h redefining the C token GL_MAX_VERTEX_ATTRIBS to 16, so
    /// the C suite asked for pname 16 while this Rust test asks for 0x8869.
    /// That defect is guarded where it lived, by gl_internal.h's static
    /// assertion, which tools/check.sh compiles with gl_test.c.
    #[test]
    fn gl_tests_reset_state_raises_no_error() {
        let gl = TestGl::new(64, 64);
        // SAFETY: plain GL calls on the current context, with live pointers.
        unsafe {
            let (mut units, mut attribs) = (0, 0);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, 64, 64);
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_BLEND);
            glDepthMask(GL_TRUE as GLboolean);
            glDepthFunc(GL_LESS);
            glFrontFace(GL_CCW);
            glCullFace(GL_BACK);
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glColorMask(1, 1, 1, 1);
            glStencilMask(0xff);
            glDisable(GL_STENCIL_TEST);
            glClearDepthf(1.0);
            glClearStencil(0);
            assert_eq!(glGetError(), GL_NO_ERROR, "the fixed-function part");
            glUseProgram(0);
            glBindVertexArrayOES(0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "the bindings");
            glGetIntegerv(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, &mut units);
            assert_eq!(glGetError(), GL_NO_ERROR, "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS");
            for u in 0..units.min(32) {
                glActiveTexture(GL_TEXTURE0 + u as GLenum);
                glBindTexture(GL_TEXTURE_2D, 0);
                glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
            }
            glActiveTexture(GL_TEXTURE0);
            assert_eq!(glGetError(), GL_NO_ERROR, "the texture units");
            glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut attribs);
            assert_eq!(glGetError(), GL_NO_ERROR, "GL_MAX_VERTEX_ATTRIBS");
            for a in 0..attribs.min(32) {
                glDisableVertexAttribArray(a as GLuint);
            }
            glStencilFuncSeparate(GL_FRONT_AND_BACK, GL_ALWAYS, 0, 0xff);
            glStencilOpSeparate(GL_FRONT_AND_BACK, GL_KEEP, GL_KEEP, GL_KEEP);
            glBlendFuncSeparate(GL_ONE, GL_ZERO, GL_ONE, GL_ZERO);
            glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_ADD);
            glBlendColor(0.0, 0.0, 0.0, 0.0);
            glDisable(GL_POLYGON_OFFSET_FILL);
            glPolygonOffset(0.0, 0.0);
            glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
            glDisable(GL_SAMPLE_COVERAGE);
            glEnable(GL_DITHER);
            glLineWidth(1.0);
            glPixelStorei(GL_PACK_ALIGNMENT, 4);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
            glDepthRangef(0.0, 1.0);
            assert_eq!(glGetError(), GL_NO_ERROR, "the rest");
        }
        drop(gl);
    }
}
