// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Framebuffer objects, renderbuffers and glReadPixels (`gl_framebuffer.c`).
//!
//! A framebuffer object is only a set of attachment points; the render target of
//! a draw is resolved from the bound framebuffer at draw time ([`fb_target`],
//! called from `gl::draw`):
//!   - framebuffer 0: the default (window) colour surface owned by the context,
//!   - otherwise: the colour attachment's storage (a texture's linear RGBA8888
//!     texel store, or a colour renderbuffer's).
//!
//! Rendering into a texture needs no copy or layout change on this GPU: the PBE
//! writes linear RGBA8888 and the TPU samples the same bytes at the same VA
//! (docs/gpu-recipes/rtt_fbo_recipe.md). The backend starts a new frame when
//! the target changes, so a draw that samples a texture rendered by an earlier
//! frame is ordered after that frame's fragment phase.
//!
//! Depth and stencil attachments own no memory: the ISP keeps depth/stencil on
//! chip for the duration of a frame (one kick), which is all GLES 2.0 apps that
//! clear them every frame need. They are tracked only for completeness checks
//! and the attachment queries.
//!
//! Row order: a window surface is stored top-down (row 0 = top of the
//! window, the scan-out order), so draws into it flip GL's bottom-up window y
//! (`GlesDraw::flip_y`). A pbuffer is not scanned out and is stored bottom-up,
//! as an FBO is (`GlContext::default_y_up`). A texture is stored bottom-up in GL terms (texel row 0 =
//! t = 0), so draws into a framebuffer texture do NOT flip; glReadPixels flips
//! only for a top-down default surface. This keeps a rendered-to texture sampling the
//! same way as an uploaded one.
#![allow(non_snake_case)]

use core::ffi::c_void;

use super::consts::*;
use super::context::{
    self, gl_trace, tex_pitch, GlAttachment, GlContext, GlFramebuffer, GlRenderbuffer, CAP_COLOR_ATTACHMENTS,
    CAP_MIP_LEVELS, MAX_CONTEXTS, MAX_FRAMEBUFFERS, MAX_RENDERBUFFERS, MAX_TEXTURES, RGBA8_BPP,
};
use super::names::{self, Kind};
use super::types::*;
use super::{draw, texture};
use crate::backend::{
    self, gles_surf_bytes_per_pixel, gles_surf_rt_class, gles_texfmt_bytes, GLES_SURF_R16F, GLES_SURF_R32F,
    GLES_SURF_RGB565, GLES_SURF_RGBA16F, GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_TEXFMT_R16F, GLES_TEXFMT_R32F,
    GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8,
};
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::Global;

/// The "said it once" flags of the two entry points that log (function-local
/// `static bool warned` in C).
struct Warned {
    blit: bool,
}

static WARNED: Global<Warned> = Global::zeroed();

const NO_ATTACHMENT: GlAttachment = GlAttachment { type_: GL_NONE, name: 0, level: 0, layer: 0 };

const FRAMEBUFFER_ZERO: GlFramebuffer = GlFramebuffer {
    used: false,
    bound: false,
    color: NO_ATTACHMENT,
    color_n: [NO_ATTACHMENT; CAP_COLOR_ATTACHMENTS - 1],
    depth: NO_ATTACHMENT,
    stencil: NO_ATTACHMENT,
    draw_buf: [0; CAP_COLOR_ATTACHMENTS],
    num_draw_buf: 0,
    read_none: false,
    read_index: 0,
};

const RENDERBUFFER_ZERO: GlRenderbuffer = GlRenderbuffer {
    used: false,
    bound: false,
    internalformat: 0,
    width: 0,
    height: 0,
    mem: OsalGpuMem::ZERO,
    external: false,
    orphan: false,
    serial: 0,
    storage_gen: 0,
};

/// Per context slot and texture name: the kick that last rendered into the
/// texture (stamped when a draw resolves it as its target). Texture writes
/// wait for that kick (`texture_target_use`): the GPU may still be
/// writing the texels. A table here rather than a `GlTexture` field keeps
/// the C view of the struct unchanged; a stale stamp of a completed kick is
/// harmless.
static TARGET_USE: Global<[[u32; MAX_TEXTURES]; MAX_CONTEXTS]> = Global::zeroed();

/// Per context slot and texture name, for a depth texture a draw renders
/// depth into: the level it rendered (the backend keeps one depth plane per
/// object, not per level), and the render kick whose depth the texels
/// already hold (`texture::resolve_depth`; 0: none). The depth a draw
/// writes stays in the backend's depth/stencil buffer until then.
#[derive(Clone, Copy)]
pub struct DepthTarget {
    pub level: u8,
    pub resolved: u32,
}
static DEPTH_TARGET: Global<[[DepthTarget; MAX_TEXTURES]; MAX_CONTEXTS]> = Global::zeroed();

/// Per context slot: `backend::gpu_work_lost()` as this context's last read
/// of the framebuffer (glReadPixels, glCopyTex*) found it. A loss the read
/// did not see itself -- a glFlush or glFinish that submitted the work and
/// dropped the backend's error, as dEQP's random-shader cases do (draw,
/// glFlush, glReadPixels) -- moves the counter, and the next read withholds
/// (`read_pixels_sync`). Kept here rather than in `GlContext` so that the
/// struct's C view does not change.
static LOSS_SEEN: Global<[u32; MAX_CONTEXTS]> = Global::zeroed();

/// A context slot starts: losses from before it existed are not its own.
pub(super) fn reset_loss_seen(index: u8) {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { LOSS_SEEN.get()[index as usize % MAX_CONTEXTS] = backend::gpu_work_lost() };
}

/// Per context slot: glDrawBuffers({GL_NONE}) on the default framebuffer.
/// ES 3.0 4.2.1 keeps draw buffer state "for each framebuffer", the default
/// one included, whose draw buffer 0 is GL_BACK or GL_NONE; GL_NONE "will
/// inhibit that fragment color from being written". Kept here rather than in
/// `GlContext` so that the struct's C view does not change (as `LOSS_SEEN`).
static DEFAULT_DRAW_NONE: Global<[bool; MAX_CONTEXTS]> = Global::zeroed();

/// A context slot starts with GL_BACK as the default framebuffer's draw buffer.
pub(super) fn reset_default_draw_buffer(index: u8) {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { DEFAULT_DRAW_NONE.get()[index as usize % MAX_CONTEXTS] = false };
}

fn default_draw_none(c: &GlContext) -> bool {
    // SAFETY: single-threaded by contract; a plain read.
    unsafe { DEFAULT_DRAW_NONE.get()[c.index as usize % MAX_CONTEXTS] }
}

/// True when the bound draw framebuffer writes no colour at all through its
/// draw buffers: the default framebuffer with draw buffer GL_NONE, or an
/// object whose every draw buffer is GL_NONE or selects an attachment with
/// no image ("then that fragment color is not written", ES 3.0 4.2.1). The
/// draw and the clear then write no colour channel, as with a colour mask of
/// all zeros (depth and stencil are still written), which every backend
/// honours; the draw's slot 0 stays the frame's own target.
pub fn colour_writes_off(c: &GlContext) -> bool {
    if c.bound_framebuffer == 0 {
        return default_draw_none(c);
    }
    let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) else { return false };
    fb.num_draw_buf != 0
        && fb.draw_buf[0] < 0
        && (1..CAP_COLOR_ATTACHMENTS).all(|i| fb.draw_buf[i] < 0 || fb.color_n[i - 1].type_ == GL_NONE)
}

/// Draw buffer 0 of the bound framebuffer object is GL_NONE: the draw's
/// slot 0 (attachment 0, the frame's own target) must not be written
/// (`draw::resolve_targets`).
pub fn draw_buffer0_none(c: &GlContext) -> bool {
    c.bound_framebuffer != 0
        && c.framebuffer_ref(c.bound_framebuffer).is_some_and(|fb| fb.num_draw_buf != 0 && fb.draw_buf[0] < 0)
}

/// GL_DRAW_BUFFERi of the bound draw framebuffer (ES 3.0 4.2.1), `i` below
/// GL_MAX_DRAW_BUFFERS.
///
/// The default framebuffer: GL_BACK or GL_NONE for buffer 0, GL_NONE after.
/// A framebuffer object after glDrawBuffers: what it recorded. A framebuffer
/// object glDrawBuffers never touched writes output `i` to attachment `i`
/// ([`fb_target_n`]), so that is what is answered, although ES 3.0 says the
/// initial value past buffer 0 is GL_NONE (see GL_CORE_NOTES.md, "Draw
/// buffers").
pub fn draw_buffer(c: &GlContext, i: usize) -> GLenum {
    if c.bound_framebuffer == 0 {
        return if i == 0 && !default_draw_none(c) { GL_BACK } else { GL_NONE };
    }
    /* an object with no state of its own yet answers the initial mapping */
    let fb = c.framebuffer_ref(c.bound_framebuffer).filter(|fb| fb.num_draw_buf != 0);
    let Some(fb) = fb else { return GL_COLOR_ATTACHMENT0 + i as GLenum };
    match fb.draw_buf.get(i) {
        Some(&b) if b >= 0 => GL_COLOR_ATTACHMENT0 + b as GLenum,
        _ => GL_NONE,
    }
}

fn stamp_target_use(c: &GlContext, loc: MemLoc) {
    if let MemLoc::Texture(i) = loc {
        // SAFETY: single-threaded by contract; no other reference is live.
        let t = unsafe { TARGET_USE.get() };
        t[c.index as usize % MAX_CONTEXTS][i] = backend::kick_id();
    }
}

/// Before a draw renders into the bound framebuffer: each colour attachment
/// that is a texture takes the pixel engine's channel order
/// ([`texture::take_pe_order`]), so its texels are all in the order the
/// render writes. Nothing to do for the default surface or a renderbuffer,
/// which only the pixel engine writes.
pub fn take_pe_order_targets(c: &mut GlContext) {
    if c.bound_framebuffer == 0 || !backend::rt_rgba8888_is_bgra() {
        return;
    }
    let mut slots = [usize::MAX; CAP_COLOR_ATTACHMENTS];
    if let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) {
        for (k, a) in core::iter::once(&fb.color).chain(fb.color_n.iter()).enumerate() {
            if a.type_ != GL_TEXTURE || k >= slots.len() {
                continue;
            }
            if let Some(Storage { mem: Some(MemLoc::Texture(i)), internalformat, .. }) = attachment_storage(c, a) {
                if is_color_format(internalformat) {
                    slots[k] = i;
                }
            }
        }
    }
    for i in slots.into_iter().filter(|&i| i != usize::MAX) {
        texture::take_pe_order(c, i);
    }
}

/// A draw is being recorded with `a`, a depth or depth-stencil texture, at
/// its depth attachment: it is stamped as a render target
/// ([`texture_target_use`]) and the level kept ([`DepthTarget`]).
pub fn stamp_depth_target(c: &GlContext, a: &GlAttachment) {
    let ti = a.name as usize;
    if a.type_ != GL_TEXTURE || ti >= MAX_TEXTURES {
        return;
    }
    stamp_target_use(c, MemLoc::Texture(ti));
    // SAFETY: single-threaded by contract; no other reference is live.
    let d = unsafe { DEPTH_TARGET.get() };
    d[c.index as usize % MAX_CONTEXTS][ti].level = a.level.clamp(0, u8::MAX as GLint) as u8;
}

/// Texture `ti`'s [`DepthTarget`] in `c`.
pub fn depth_target(c: &GlContext, ti: usize) -> DepthTarget {
    // SAFETY: single-threaded by contract; no other reference is live.
    let d = unsafe { DEPTH_TARGET.get() };
    d[c.index as usize % MAX_CONTEXTS].get(ti).copied().unwrap_or(DepthTarget { level: 0, resolved: 0 })
}

/// The texels of texture `ti` hold the depth render kick `kick` left.
pub fn set_depth_resolved(c: &GlContext, ti: usize, kick: u32) {
    // SAFETY: single-threaded by contract; no other reference is live.
    let d = unsafe { DEPTH_TARGET.get() };
    if let Some(e) = d[c.index as usize % MAX_CONTEXTS].get_mut(ti) {
        e.resolved = kick;
    }
}

/// The kick that last rendered into texture `ti` of `c` (0: none).
pub fn texture_target_use(c: &GlContext, ti: usize) -> u32 {
    // SAFETY: single-threaded by contract; no other reference is live.
    let t = unsafe { TARGET_USE.get() };
    t[c.index as usize % MAX_CONTEXTS].get(ti).copied().unwrap_or(0)
}

/* --------------------------------------------------------------------------- */
/* attachment resolution */

fn attachment_slot(fb: &mut GlFramebuffer, attachment: GLenum) -> Option<&mut GlAttachment> {
    if attachment > GL_COLOR_ATTACHMENT0 && attachment < GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum {
        return Some(&mut fb.color_n[(attachment - GL_COLOR_ATTACHMENT0) as usize - 1]);
    }
    match attachment {
        GL_COLOR_ATTACHMENT0 => Some(&mut fb.color),
        GL_DEPTH_ATTACHMENT => Some(&mut fb.depth),
        GL_STENCIL_ATTACHMENT => Some(&mut fb.stencil),
        _ => None,
    }
}

/// An attachment point glFramebufferTexture2D / glFramebufferRenderbuffer
/// accept: one with a slot, or the packed depth/stencil point.
fn is_attachment_point(attachment: GLenum) -> bool {
    (attachment > GL_COLOR_ATTACHMENT0 && attachment < GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum)
        || matches!(
            attachment,
            GL_COLOR_ATTACHMENT0 | GL_DEPTH_ATTACHMENT | GL_STENCIL_ATTACHMENT | GL_DEPTH_STENCIL_ATTACHMENT
        )
}

fn is_fb_target(target: GLenum) -> bool {
    target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER || target == GL_READ_FRAMEBUFFER
}

/// Set (name != 0) or clear an attachment point; GL_DEPTH_STENCIL_ATTACHMENT
/// (ES 3.0, a packed depth/stencil renderbuffer) is both the depth and the
/// stencil point.
fn set_attachment(fb: &mut GlFramebuffer, attachment: GLenum, type_: GLenum, name: GLuint, level: GLint, layer: GLint) {
    let a = if name == 0 { NO_ATTACHMENT } else { GlAttachment { type_, name, level, layer } };

    if attachment == GL_DEPTH_STENCIL_ATTACHMENT {
        fb.depth = a;
        fb.stencil = a;
    } else if let Some(slot) = attachment_slot(fb, attachment) {
        *slot = a;
    }
}

/// Where a memory descriptor lives inside the context (the C code passes
/// `struct osal_gpu_mem *` around; the table index keeps the borrow checker
/// out of it and still resolves to the real descriptor).
#[derive(Clone, Copy)]
enum MemLoc {
    DefaultSurface,
    Texture(usize),
    Renderbuffer(usize),
}

fn mem_at(c: &GlContext, loc: MemLoc) -> &OsalGpuMem {
    match loc {
        MemLoc::DefaultSurface => &c.default_surface,
        MemLoc::Texture(i) => &c.objs_ref().textures[i].mem,
        MemLoc::Renderbuffer(i) => &c.objs_ref().renderbuffers[i].mem,
    }
}

fn mem_at_mut(c: &mut GlContext, loc: MemLoc) -> &mut OsalGpuMem {
    match loc {
        MemLoc::DefaultSurface => &mut c.default_surface,
        MemLoc::Texture(i) => &mut c.objs().textures[i].mem,
        MemLoc::Renderbuffer(i) => &mut c.objs().renderbuffers[i].mem,
    }
}

struct Storage {
    /// None for depth/stencil renderbuffers, which own no memory.
    mem: Option<MemLoc>,
    w: u32,
    h: u32,
    internalformat: GLenum,
    /// Byte offset of the attached level/face/layer inside `mem`.
    offset: u32,
}

/// Does `mem` hold `w` x `h` pixels of `bpp` bytes from `offset` on, at the
/// PBE's pair-padded row pitch (the last row needs only `w` pixels)?
fn storage_holds(mem: &OsalGpuMem, offset: u32, w: u32, h: u32, bpp: u32) -> bool {
    if w == 0 || h == 0 {
        return false;
    }
    let pitch = tex_pitch(w) as u64;
    let need = ((h as u64 - 1) * pitch + w as u64) * bpp as u64 + offset as u64;
    need <= mem.size as u64
}

/// Storage + size behind an attachment (no `mem` for depth/stencil renderbuffers,
/// which own none). None when the attached object no longer exists or has no
/// storage yet, or is a texture the PBE cannot render into (compressed,
/// RGB565 / external formats whose texels are not RGBA8 or float, a 3D
/// texture's tiled slices), the attached level/layer is not there, or the
/// storage is smaller than the PBE's rows (an external image).
fn attachment_storage(c: &GlContext, a: &GlAttachment) -> Option<Storage> {
    if a.type_ == GL_TEXTURE {
        let t = c.texture_ref(a.name)?;

        /* A depth texture attachment: the render writes depth through the
         * backend's own depth/stencil buffer (keyed by this texture; it
         * reaches the texels at texture::resolve_depth), so what the
         * framebuffer needs from it is its size and that it counts as a
         * depth format. Its storage is the same RGBA8-sized memory, which
         * still has to hold the level. */
        if matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL)
            && !t.mem.impl_.is_null()
            && t.width != 0
            && t.height != 0
            && a.level >= 0
            && (a.level as u32) < t.chain_levels as u32
            && (a.level as usize) < CAP_MIP_LEVELS
        {
            let lw = (t.width >> a.level).max(1);
            let lh = (t.height >> a.level).max(1);
            let offset = t.level_off[a.level as usize];

            if !storage_holds(&t.mem, offset, lw, lh, RGBA8_BPP) {
                return None;
            }
            return Some(Storage {
                mem: Some(MemLoc::Texture(a.name as usize)),
                w: lw,
                h: lh,
                internalformat: if t.format == GL_DEPTH_STENCIL { GL_DEPTH24_STENCIL8 } else { GL_DEPTH_COMPONENT24 },
                offset,
            });
        }
        if t.mem.impl_.is_null()
            || t.width == 0
            || t.height == 0
            || t.block_bytes != 0
            || !matches!(t.texfmt, GLES_TEXFMT_RGBA8 | GLES_TEXFMT_RGBA16F | GLES_TEXFMT_R16F | GLES_TEXFMT_R32F)
            || t.grid_cols > 1
            || a.level < 0
            || a.level as u32 >= t.chain_levels as u32
            || a.level as usize >= CAP_MIP_LEVELS
            || a.layer < 0
            || a.layer as u32 >= t.layers.max(1) as u32
        {
            return None;
        }
        /* the attached level of the attached face/layer: a sub-rectangle
         * of the stacked storage, at the level's pair-padded pitch */
        let lw = (t.width >> a.level).max(1);
        let lh = (t.height >> a.level).max(1);
        let internalformat = match t.texfmt {
            GLES_TEXFMT_RGBA16F => GL_RGBA16F,
            GLES_TEXFMT_R16F => GL_R16F,
            GLES_TEXFMT_R32F => GL_R32F,
            _ => GL_RGBA,
        };
        /* A 1-wide level of a PowerVR chain has one-texel rows where the
         * TPU samples it, and the PBE writes rows a texel pair apart: more
         * than one row of it cannot be a render target without drawing the
         * wrong picture, so it is incomplete instead. One row (a cube face's
         * 1x1 level) is the same either way. */
        if lh > 1 && t.level_pitch(lw) != tex_pitch(lw) {
            return None;
        }
        /* checked: a layer past what the storage holds is incomplete */
        let offset = (a.layer as u64 * lh as u64 * t.level_pitch(lw) as u64 * gles_texfmt_bytes(t.texfmt) as u64)
            .checked_add(t.level_off[a.level as usize] as u64)
            .and_then(|o| u32::try_from(o).ok())?;
        if !storage_holds(&t.mem, offset, lw, lh, gles_texfmt_bytes(t.texfmt)) {
            return None;
        }
        return Some(Storage { mem: Some(MemLoc::Texture(a.name as usize)), w: lw, h: lh, internalformat, offset });
    }
    if a.type_ == GL_RENDERBUFFER {
        let rb = c.renderbuffer_ref(a.name)?;

        if rb.width == 0 || rb.height == 0 {
            return None;
        }
        /* a colour renderbuffer's storage (its own or an image's) must hold
         * the rows the PBE writes, in the surface its format maps to */
        let bpp = if is_color_format(rb.internalformat) {
            gles_surf_bytes_per_pixel(renderbuffer_surf_format(rb.internalformat))
        } else {
            RGBA8_BPP
        };
        if !rb.mem.impl_.is_null() && !storage_holds(&rb.mem, 0, rb.width, rb.height, bpp) {
            return None;
        }
        return Some(Storage {
            mem: (!rb.mem.impl_.is_null()).then_some(MemLoc::Renderbuffer(a.name as usize)),
            w: rb.width,
            h: rb.height,
            internalformat: rb.internalformat,
            offset: 0,
        });
    }
    None
}

fn is_color_format(f: GLenum) -> bool {
    /* float targets: RGBA8 storage for now. GL_BGRA_EXT is here because a
     * BGRA texture's store IS RGBA8 -- the swap happens once, on upload -- so
     * the PBE writes it like any other colour texture and it is renderable.
     * It is not in is_renderbuffer_color_format: GL_EXT_texture_format_BGRA8888
     * adds a texture format, not renderbuffer storage, which is named by a
     * sized format. */
    matches!(
        f,
        GL_BGRA_EXT
            | GL_BGRA8_EXT
            | GL_RGBA
            | GL_RGBA4
            | GL_RGB5_A1
            | GL_RGB565
            | GL_RGBA8
            | GL_RGB8
            | GL_SRGB8_ALPHA8
            | GL_R8
            | GL_RG8
            | GL_RGBA16F
            | GL_RGB16F
            | GL_RG16F
            | GL_R16F
            | GL_R32F
    )
}

/// The colour formats renderbuffer storage is named by. GL names renderbuffer
/// storage with a SIZED format; the unsized base formats (GL_RGBA and the
/// rest) belong to textures, and a texture attachment reports one, so
/// [`is_color_format`] above keeps them and this predicate does not. Folding
/// the two together would make every render-to-texture framebuffer
/// incomplete.
fn is_renderbuffer_color_format(f: GLenum) -> bool {
    /* GL_SRGB8_ALPHA8 is renderbuffer storage in ES 3.0 and in ES 2.0 only
     * with GL_EXT_sRGB or GL_EXT_sRGB_write_control. This surface claims
     * neither -- there is no sRGB encode or decode anywhere in it -- so on
     * the ES 2.0 surface the name is not renderbuffer storage, and a
     * framebuffer built on one is not complete. glTexImage2D already refuses
     * the same name for the same reason. */
    if crate::REPORT_ES2 && f == GL_SRGB8_ALPHA8 {
        return false;
    }
    is_color_format(f) && f != GL_RGBA
}

/// The surface (`GLES_SURF_*`) a colour renderbuffer of this format is stored
/// and rendered as.
///
/// The half-float names share one surface: GL_RGB16F and GL_RG16F keep four
/// half-float channels like GL_RGBA16F and the channels the format does not
/// have are ignored, exactly as GL_RGB565 keeps RGBA8. Everything with eight
/// bits or fewer a channel is RGBA8888.
///
/// The renderbuffer used to be RGBA8888 whatever was asked for, float names
/// included, so a GL_RGBA16F renderbuffer was rendered and read back as
/// eight-bit unorm while `GL_RENDERBUFFER_INTERNAL_FORMAT` said otherwise.
fn renderbuffer_surf_format(internalformat: GLenum) -> u8 {
    match internalformat {
        GL_RGBA16F | GL_RGB16F | GL_RG16F => GLES_SURF_RGBA16F,
        GL_R16F => GLES_SURF_R16F,
        GL_R32F => GLES_SURF_R32F,
        _ => GLES_SURF_RGBA8888,
    }
}

fn is_depth_format(f: GLenum) -> bool {
    matches!(
        f,
        GL_DEPTH_COMPONENT16
            | GL_DEPTH_COMPONENT24
            | GL_DEPTH_COMPONENT32F
            | GL_DEPTH24_STENCIL8
            | GL_DEPTH32F_STENCIL8
    )
}

fn is_stencil_format(f: GLenum) -> bool {
    matches!(f, GL_STENCIL_INDEX8 | GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8)
}

fn check_status(c: &GlContext, fb: &GlFramebuffer) -> GLenum {
    let (mut cw, mut ch) = (0u32, 0u32);
    let mut any = false;

    if fb.color.type_ != GL_NONE {
        match attachment_storage(c, &fb.color) {
            Some(s) if s.mem.is_some() && is_color_format(s.internalformat) => {
                cw = s.w;
                ch = s.h;
                any = true;
            }
            _ => return GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
        }
    }
    /* Colour attachments 1..: each is attachment complete as attachment 0
     * is (ES 2.0.25 4.4.5 with GL_NV_fbo_color_attachments; ES 3.0.6
     * 4.4.4.1: "If attachment is COLOR_ATTACHMENTi, then image must have a
     * color-renderable internal format"). The same-size rule is ES 2.0's
     * ("All attached images have the same width and height.
     * FRAMEBUFFER_INCOMPLETE_DIMENSIONS"); ES 3.0 4.4.4.2 says that token
     * "cannot be generated by an OpenGL ES 3.0 implementation", so the ES
     * 3.0 surface does not apply it here. Before, these attachments were
     * never looked at, and an incomplete, unrenderable or other-sized one
     * gave GL_FRAMEBUFFER_COMPLETE. */
    for a in fb.color_n.iter().filter(|a| a.type_ != GL_NONE) {
        match attachment_storage(c, a) {
            Some(s) if s.mem.is_some() && is_color_format(s.internalformat) => {
                if crate::REPORT_ES2 && any && (s.w != cw || s.h != ch) {
                    return GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
                }
                if !any {
                    (cw, ch) = (s.w, s.h);
                }
                any = true;
            }
            _ => return GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
        }
    }
    if fb.depth.type_ != GL_NONE {
        match attachment_storage(c, &fb.depth) {
            Some(s) if is_depth_format(s.internalformat) => {
                if any && (s.w != cw || s.h != ch) {
                    return GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
                }
                cw = s.w;
                ch = s.h;
                any = true;
            }
            _ => return GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
        }
    }
    if fb.stencil.type_ != GL_NONE {
        match attachment_storage(c, &fb.stencil) {
            Some(s) if is_stencil_format(s.internalformat) => {
                if any && (s.w != cw || s.h != ch) {
                    return GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
                }
                any = true;
            }
            _ => return GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
        }
    }
    if !any {
        return GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT;
    }
    /* No colour attachment 0. A framebuffer with no colour attachment at
     * all (glmark2's shadow pass) is the backend's question: complete where
     * it draws without one (backend::DEPTH_ONLY_TARGETS, which each backend
     * declares), GL_FRAMEBUFFER_UNSUPPORTED where it does not (ES 2.0 4.4.5).
     * One with colour only on attachments 1.. stays unsupported. */
    if fb.color.type_ == GL_NONE && !(crate::backend::DEPTH_ONLY_TARGETS && is_depth_stencil_only(fb)) {
        return GL_FRAMEBUFFER_UNSUPPORTED;
    }
    GL_FRAMEBUFFER_COMPLETE
}

/// No colour attachment at all, and a depth or stencil one.
fn is_depth_stencil_only(fb: &GlFramebuffer) -> bool {
    fb.color.type_ == GL_NONE
        && fb.color_n.iter().all(|a| a.type_ == GL_NONE)
        && (fb.depth.type_ != GL_NONE || fb.stencil.type_ != GL_NONE)
}

/// The bound framebuffer is complete and has no colour attachment
/// ([`is_depth_stencil_only`]): a draw into it goes to the backend with no
/// colour target (backend::DEPTH_ONLY_TARGETS). Its size is the depth (else
/// stencil) attachment's.
pub fn fb_depth_stencil_only_size(c: &GlContext) -> Option<(u32, u32)> {
    if !crate::backend::DEPTH_ONLY_TARGETS || c.bound_framebuffer == 0 {
        return None;
    }
    let fb = c.framebuffer_ref(c.bound_framebuffer)?;
    if !is_depth_stencil_only(fb) || check_status(c, fb) != GL_FRAMEBUFFER_COMPLETE {
        return None;
    }
    let a = if fb.depth.type_ != GL_NONE { &fb.depth } else { &fb.stencil };
    attachment_storage(c, a).map(|s| (s.w, s.h)).filter(|&(w, h)| w != 0 && h != 0)
}

/// The surface format (`GLES_SURF_*`) of the bound framebuffer's colour target.
pub fn fb_target_format(c: &GlContext) -> u8 {
    if c.bound_framebuffer == 0 {
        return c.default_format;
    }
    if let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) {
        if fb.color.type_ == GL_TEXTURE {
            match c.texture_ref(fb.color.name).map(|t| (t.texfmt, t.bgra)) {
                Some((GLES_TEXFMT_RGBA16F, _)) => return GLES_SURF_RGBA16F,
                Some((GLES_TEXFMT_R16F, _)) => return GLES_SURF_R16F,
                Some((GLES_TEXFMT_R32F, _)) => return GLES_SURF_R32F,
                /* an XRGB8888 image's texels are B,G,R,X */
                Some((GLES_TEXFMT_RGBA8, true)) => return GLES_SURF_XRGB8888,
                _ => {}
            }
        }
        if fb.color.type_ == GL_RENDERBUFFER {
            if let Some(rb) = c.renderbuffer_ref(fb.color.name) {
                if is_color_format(rb.internalformat) {
                    return renderbuffer_surf_format(rb.internalformat);
                }
            }
        }
    }
    GLES_SURF_RGBA8888
}

/// How many alpha bits the bound colour target really has.
///
/// The storage behind every colour attachment is RGBA8 whatever the format
/// asked for (`renderbuffer_storage`, and a GL_RGB texture keeps RGBA8
/// texels), so the alpha channel is there in memory even when the format the
/// application named has no alpha at all. GL says such a buffer reads 1
/// (ES 2.0 4.3.1: "if the framebuffer does not have an alpha channel ... A is
/// 1.0"), and dEQP's reference renderer gives exactly that -- so what is
/// stored has to say 1 as well, or every alpha byte of the image is wrong.
///
/// 0 means "no alpha, it reads 1"; 1, 4 and 8 are the widths ES 2.0's colour
/// formats actually have. The two callers are the clear (which rounds the
/// clear colour's alpha to this width) and the draw (which takes the alpha
/// channel out of the write mask when this is 0).
pub fn fb_target_alpha_bits(c: &GlContext) -> u8 {
    if c.bound_framebuffer == 0 {
        /* the default surface: RGB565 and XRGB8888 carry no alpha */
        return match c.default_format {
            GLES_SURF_RGB565 | GLES_SURF_XRGB8888 => 0,
            _ => 8,
        };
    }
    let Some(fb) = c.framebuffer_ref(c.bound_framebuffer) else { return 8 };
    match fb.color.type_ {
        /* a texture names its format unsized: GL_RGB has no alpha, and
         * GL_LUMINANCE / GL_ALPHA are not renderable, so they cannot get here */
        GL_TEXTURE => match c.texture_ref(fb.color.name).map(|t| t.format) {
            /* GL_LUMINANCE and GL_EXT_texture_rg's two are here for the same
             * reason as GL_RGB: they have no alpha channel, so the buffer
             * reads 1. (Their green and blue read 0, which the PBE's swizzle
             * could do as well and does not yet; nothing in ES 2.0's
             * conformance renders into one -- the extension makes them
             * renderable, it does not require it.) */
            Some(GL_RGB) | Some(GL_RED) | Some(GL_RG) | Some(GL_LUMINANCE) => 0,
            _ => 8,
        },
        GL_RENDERBUFFER => match c.renderbuffer_ref(fb.color.name).map(|rb| rb.internalformat) {
            Some(GL_RGB565) | Some(GL_RGB8) | Some(GL_RGB16F) | Some(GL_RG16F) | Some(GL_R16F) | Some(GL_R32F)
            | Some(GL_R8) | Some(GL_RG8) => 0,
            Some(GL_RGB5_A1) => 1,
            Some(GL_RGBA4) => 4,
            _ => 8,
        },
        _ => 8,
    }
}

/// Whether the bound target has a depth buffer and whether it has a stencil
/// buffer. A framebuffer object has each only where something is attached;
/// the default surface has both (this implementation always gives a window
/// surface a D24S8, and glGetIntegerv answers 24 and 8 to match).
///
/// It decides more than a query: ES 2.0 4.1.4 and 4.1.6 say that with no
/// stencil buffer the stencil test always passes and nothing is written, and
/// with no depth buffer the depth test always passes. Enabling GL_DEPTH_TEST
/// while rendering into an FBO that has no depth attachment must therefore
/// change nothing -- and it did, because the backend keeps a depth/stencil
/// buffer for the target whether or not GL says one exists. dEQP's
/// fbo.render.stencil is built on exactly this distinction (it expects a
/// stencil of 2, not 1, when the configuration has no depth buffer).
pub fn fb_target_depth_stencil(c: &GlContext) -> (bool, bool) {
    if c.bound_framebuffer == 0 {
        return (true, true);
    }
    match c.framebuffer_ref(c.bound_framebuffer) {
        Some(fb) => (fb.depth.type_ != GL_NONE, fb.stencil.type_ != GL_NONE),
        None => (false, false),
    }
}

/// `v` rounded to what `bits` of a unorm channel can hold. `bits` 0 is "the
/// channel is not there": it reads 1.
pub fn quantise_unorm(v: f32, bits: u8) -> f32 {
    if bits == 0 {
        return 1.0;
    }
    if bits >= 8 {
        return v;
    }
    let steps = ((1u32 << bits) - 1) as f32;
    /* `as u32` and not `f32::floor`: the crate is no_std and has no libm.
     * The value is non-negative here, so the cast truncates towards zero,
     * which is the floor. */
    ((v.clamp(0.0, 1.0) * steps + 0.5) as u32) as f32 / steps
}

/// The render target (`gl_fb_target`): its descriptor (by location), size
/// and the byte offset of the attached level/face/layer. None when there is
/// no default surface or the bound framebuffer is incomplete (said in the
/// log: a client that draws into such a framebuffer only sees
/// INVALID_FRAMEBUFFER_OPERATION).
fn target_loc(c: &GlContext) -> Option<(MemLoc, u32, u32, u32)> {
    if c.bound_framebuffer == 0 {
        if !c.default_surface_valid {
            return None;
        }
        return Some((MemLoc::DefaultSurface, c.target_w, c.target_h, 0));
    }

    let fb = c.framebuffer_ref(c.bound_framebuffer);
    let status = fb.map(|fb| check_status(c, fb));

    if status != Some(GL_FRAMEBUFFER_COMPLETE) {
        glog!(
            "gl_fb_target: framebuffer {} {} (status 0x{:x})",
            c.bound_framebuffer,
            if fb.is_some() { "incomplete" } else { "unknown" },
            status.unwrap_or(0)
        );
        return None;
    }
    /* complete with no colour attachment, where the backend draws without
     * one (backend::DEPTH_ONLY_TARGETS): no colour target, and nothing is
     * wrong with it; the draw takes fb_depth_stencil_only_size instead */
    if crate::backend::DEPTH_ONLY_TARGETS && fb.is_some_and(is_depth_stencil_only) {
        return None;
    }
    let s = fb.and_then(|fb| attachment_storage(c, &fb.color));
    let Some(s) = s else {
        glog!("gl_fb_target: framebuffer {} colour attachment has no storage", c.bound_framebuffer);
        return None;
    };
    /* check_status: a complete framebuffer's colour attachment has memory */
    Some((s.mem?, s.w, s.h, s.offset))
}

/// The colour buffer a draw renders into (a copy of its memory descriptor),
/// its size and the byte offset of the attached level/face/layer: the bound
/// framebuffer's colour attachment, or the default surface when framebuffer 0
/// is bound. None when incomplete.
///
/// A draw resolves its target through this: a texture target is stamped
/// with the kick being recorded (see `TARGET_USE`).
/// The row pitch in pixels of the target at `loc`, `tw` wide: the default
/// surface's own ([`GlContext::default_pitch_px`]: a window may be a view
/// into a wider scan-out buffer), else the pair-padded `tex_pitch(tw)` every
/// GL-core storage uses.
fn loc_pitch(c: &GlContext, loc: MemLoc, tw: u32) -> u32 {
    match loc {
        MemLoc::DefaultSurface => c.default_pitch_px(),
        _ => tex_pitch(tw),
    }
}

pub fn fb_target(c: &GlContext) -> Option<(OsalGpuMem, u32, u32, u32)> {
    target_loc(c).map(|(loc, w, h, off)| {
        stamp_target_use(c, loc);
        (*mem_at(c, loc), w, h, off)
    })
}

/// Colour attachment `n` (n >= 1) of the bound framebuffer: its memory,
/// size and `GLES_SURF_*` format, or `None` when there is none. Attachment 0
/// is [`fb_target`].
///
/// Which attachment an output writes is what glDrawBuffers said; the default
/// is output `i` to attachment `i`.
pub fn fb_target_n(c: &GlContext, n: u8) -> Option<(u64, u32, u32, u8)> {
    if n == 0 || n as usize >= CAP_COLOR_ATTACHMENTS || c.bound_framebuffer == 0 {
        return None;
    }
    let fb = c.framebuffer_ref(c.bound_framebuffer)?;

    if fb.num_draw_buf != 0 && (n >= fb.num_draw_buf || fb.draw_buf[n as usize] < 0) {
        return None;
    }
    let a = &fb.color_n[n as usize - 1];

    if a.type_ == GL_NONE {
        return None;
    }
    let st = attachment_storage(c, a)?;
    /* as attachment 0: only a format the PBE can write, and storage that
     * holds the rows it writes (attachment_storage checked the size) */
    if !is_color_format(st.internalformat) {
        return None;
    }
    let loc = st.mem?;
    let mem = mem_at(c, loc);

    if mem.impl_.is_null() {
        return None;
    }
    stamp_target_use(c, loc);
    let format = match st.internalformat {
        GL_RGBA16F => GLES_SURF_RGBA16F,
        GL_R16F => GLES_SURF_R16F,
        GL_R32F => GLES_SURF_R32F,
        _ => GLES_SURF_RGBA8888,
    };
    Some((mem.gpu + st.offset as u64, st.w, st.h, format))
}

/// Is there a render target (the check glCopyTex*Image2D makes; nothing is
/// stamped)?
pub fn fb_target_ok(c: &GlContext) -> bool {
    target_loc(c).is_some()
}

/// True when the bound draw target has storage to render into. A draw into
/// one that has not is GL_INVALID_FRAMEBUFFER_OPERATION (ES 2.0 4.4.5), and
/// that outranks every other error a draw can raise -- including the missing
/// program. It is the same question [`crate::gl::draw`]'s resolve_target
/// asks, so the two cannot disagree.
pub fn fb_target_drawable(c: &GlContext) -> bool {
    matches!(fb_target(c), Some((_, w, h, _)) if w != 0 && h != 0) || fb_depth_stencil_only_size(c).is_some()
}

/// true when the bound target is stored top-down: the default surface,
/// unless it is a pbuffer ([`GlContext::default_y_up`]), which is drawn y-up
/// like an FBO so gl_FragCoord, dFdy and point sprites need no mirroring.
pub fn fb_flip_y(c: &GlContext) -> bool {
    c.bound_framebuffer == 0 && !c.default_y_up
}

/* Deleting an attached object breaks the attachment in the framebuffer that
 * is bound and in no other (ES 2.0 4.4.3 for a renderbuffer, 3.7.13 for a
 * texture: "as if FramebufferRenderbuffer had been called ... for each
 * attachment point to which this image was attached in the currently bound
 * framebuffer"). A framebuffer that is not bound keeps the attachment, name
 * and all, which is what dEQP's fbo.api.delete_*_attached_to_not_bound_fbo
 * reads back and what fbo.render.recreate_*.no_rebind_* then renders into
 * after re-creating an object under the same name. Detaching everywhere is
 * how this read until 2026-09-22 and it cost both. */
fn detach_from_bound(c: &mut GlContext, type_: GLenum, name: GLuint) {
    let bound = c.bound_framebuffer;
    let Some(fb) = c.framebuffer(bound) else { return };
    for slot in [&mut fb.color, &mut fb.depth, &mut fb.stencil].into_iter().chain(fb.color_n.iter_mut()) {
        if slot.type_ == type_ && slot.name == name {
            *slot = NO_ATTACHMENT;
        }
    }
}

/// Every attachment point of a framebuffer.
fn attachment_points(fb: &GlFramebuffer) -> impl Iterator<Item = &GlAttachment> {
    [&fb.color, &fb.depth, &fb.stencil].into_iter().chain(fb.color_n.iter())
}

/// How many attachment points, over every framebuffer object of the
/// context, hold `name` of `type_`.
fn attachment_refs(c: &GlContext, type_: GLenum, name: GLuint) -> usize {
    c.framebuffers
        .iter()
        .filter(|fb| fb.used)
        .map(|fb| attachment_points(fb).filter(|a| a.type_ == type_ && a.name == name).count())
        .sum()
}

/* The orphan rule. Deleting an object detaches it from the framebuffer that
 * is bound and from no other (ES 2.0 3.7.13 / 4.4.3; `detach_from_bound`),
 * so a framebuffer that is not bound keeps reading the deleted object's
 * IMAGE. Objects live in fixed pools indexed by name, and until 2026-09-24
 * the delete freed the slot at once: the next glGen* handed the same name
 * out, the new object's storage landed in that slot, and the framebuffer
 * read the new object instead (dEQP-GLES2 lifetime.attach.deleted_input /
 * deleted_output.{renderbuffer,texture}_framebuffer, all four, on the
 * i.MX8MP). So an object deleted while another framebuffer still holds it
 * becomes an orphan: `used` stays (the name is not re-handed), `orphan`
 * says the name is gone (glIs* false, attaching it refused), and the slot
 * is freed once the last attachment point lets go -- an attachment
 * replaced, or its framebuffer deleted (`attachment_released`). Binding an
 * orphan's name again is the ES 2.0 "a bind creates the object" case: a
 * fresh object under the name, while the framebuffers that held the
 * orphan keep ITS image. The pool has one slot per name, so the orphan
 * moves to a free slot first (`orphan_moved`) and their attachments follow
 * it there. That is the shape dEQP drives most: ES 2.0 needs no glGen*, so
 * lifetime.attach.deleted_{input,output}.* reuse the deleted name for the
 * new object (glsLifetimeTests.cpp replaceName) and require the
 * framebuffer to keep showing, and writing, the deleted image; and
 * fbo.render.recreate_*.no_rebind_* delete, bind the same name, respecify,
 * and bind the framebuffer without attaching again, which renders into the
 * kept image and reads the same picture as dEQP's reference (it clears the
 * recreated buffer first). Detaching the name there (62fc61fa, until
 * 2026-09-24) left the no_rebind framebuffers incomplete. With no free
 * slot, the name's slot is freed and the framebuffers hold the new object
 * by name, as dEQP's reference context does (it looks an attachment up by
 * name at each use). */

/// Point every attachment of `c`'s framebuffers that holds `from` of `type_`
/// at `to`: the orphan's record moved there (the orphan rule above).
fn orphan_moved(c: &mut GlContext, type_: GLenum, from: GLuint, to: GLuint) {
    for fb in c.framebuffers.iter_mut().filter(|fb| fb.used) {
        for slot in [&mut fb.color, &mut fb.depth, &mut fb.stencil].into_iter().chain(fb.color_n.iter_mut()) {
            if slot.type_ == type_ && slot.name == from {
                slot.name = to;
            }
        }
    }
    if type_ == GL_TEXTURE {
        // SAFETY: single-threaded by contract; no other reference is live.
        let t = unsafe { TARGET_USE.get() };
        let row = &mut t[c.index as usize % MAX_CONTEXTS];
        if (from as usize) < MAX_TEXTURES && (to as usize) < MAX_TEXTURES {
            row[to as usize] = row[from as usize];
            row[from as usize] = 0;
            // SAFETY: as above.
            let d = unsafe { DEPTH_TARGET.get() };
            let row = &mut d[c.index as usize % MAX_CONTEXTS];
            row[to as usize] = row[from as usize];
            row[from as usize] = DepthTarget { level: 0, resolved: 0 };
        }
    }
}

/// The texture orphan `from` moved to the free slot `to` (glBindTexture of
/// its name): the framebuffers that held it follow it.
pub fn texture_orphan_moved(c: &mut GlContext, from: GLuint, to: GLuint) {
    orphan_moved(c, GL_TEXTURE, from, to);
}

/// The deleted texture `texture` (glDeleteTextures): detached from the bound
/// framebuffer; true when it is still held elsewhere and must stay an orphan
/// rather than be freed (ES 2.0 3.7.13; the orphan rule above).
pub fn texture_deleted(c: &mut GlContext, texture: GLuint) -> bool {
    detach_from_bound(c, GL_TEXTURE, texture);
    attachment_refs(c, GL_TEXTURE, texture) != 0
}

/// Detach a deleted renderbuffer from the bound framebuffer (ES 2.0 4.4.3);
/// true when another framebuffer still holds it (the orphan rule above).
fn renderbuffer_deleted(c: &mut GlContext, renderbuffer: GLuint) -> bool {
    detach_from_bound(c, GL_RENDERBUFFER, renderbuffer);
    attachment_refs(c, GL_RENDERBUFFER, renderbuffer) != 0
}

/// Free a renderbuffer's slot: its storage, its binding, its name.
fn free_renderbuffer(c: &mut GlContext, h: GLuint) {
    let Some(rb) = c.renderbuffer(h) else { return };
    crate::backend::attachment_gone(context::ds_object_id(rb.serial, false));
    context::release_storage(&mut rb.mem, &mut rb.external);
    if c.bound_renderbuffer == h {
        c.bound_renderbuffer = 0;
    }
    c.objs().renderbuffers[h as usize] = RENDERBUFFER_ZERO;
}

/// An attachment point let go of `name` of `type_` (replaced, or its
/// framebuffer deleted): an orphan nothing holds any more is freed.
fn attachment_released(c: &mut GlContext, type_: GLenum, name: GLuint) {
    if name == 0 || attachment_refs(c, type_, name) != 0 {
        return;
    }
    match type_ {
        GL_RENDERBUFFER if c.renderbuffer_ref(name).is_some_and(|rb| rb.orphan) => free_renderbuffer(c, name),
        GL_TEXTURE if c.texture_ref(name).is_some_and(|t| t.orphan) => super::texture::free_texture(c, name),
        _ => {}
    }
}

/// What an attachment point holds, as `attachment_released` wants it; the
/// packed depth/stencil point is two.
fn held_by(fb: &mut GlFramebuffer, attachment: GLenum) -> [(GLenum, GLuint); 2] {
    if attachment == GL_DEPTH_STENCIL_ATTACHMENT {
        [(fb.depth.type_, fb.depth.name), (fb.stencil.type_, fb.stencil.name)]
    } else {
        match attachment_slot(fb, attachment) {
            Some(a) => [(a.type_, a.name), (GL_NONE, 0)],
            None => [(GL_NONE, 0), (GL_NONE, 0)],
        }
    }
}

/// `set_attachment` on the bound framebuffer, releasing what the point held.
fn replace_attachment(c: &mut GlContext, attachment: GLenum, type_: GLenum, name: GLuint, level: GLint, layer: GLint) {
    let bound = c.bound_framebuffer;
    let Some(fb) = c.framebuffer(bound) else { return };
    let old = held_by(fb, attachment);
    set_attachment(fb, attachment, type_, name, level, layer);
    for (t, n) in old {
        attachment_released(c, t, n);
    }
}

/* ---- the C names of the above (gl_internal.h): the on-silicon test suite and
 * the samples' glue call them. */

/// The render target of the current context: a pointer to the real descriptor
/// inside the context, null (and `*w` = `*h` = 0) when there is none;
/// `offset` (may be null) receives the byte offset of the attached
/// level/face/layer inside that storage.
#[no_mangle]
pub unsafe extern "C" fn gl_fb_target(w: *mut u32, h: *mut u32, offset: *mut u32) -> *mut OsalGpuMem {
    // SAFETY: the C contract: `w` and `h` point at writable uint32_t, `offset`
    // is null or does.
    unsafe {
        w.write_unaligned(0);
        h.write_unaligned(0);
        if !offset.is_null() {
            offset.write_unaligned(0);
        }
    }
    let Some(c) = context::current() else {
        return core::ptr::null_mut();
    };
    let Some((loc, tw, th, toff)) = target_loc(c) else {
        return core::ptr::null_mut();
    };
    stamp_target_use(c, loc); /* the caller renders into it */
    // SAFETY: as above.
    unsafe {
        w.write_unaligned(tw);
        h.write_unaligned(th);
        if !offset.is_null() {
            offset.write_unaligned(toff);
        }
    }
    mem_at_mut(c, loc)
}

/// Colour attachment `n` (n >= 1) of the bound framebuffer: false when there
/// is none. Attachment 0 is [`gl_fb_target`].
///
/// # Safety
/// `gpu`, `w`, `h` and `format` must each point at a writable value of their
/// type (the C contract).
#[no_mangle]
pub unsafe extern "C" fn gl_fb_target_n(n: u8, gpu: *mut u64, w: *mut u32, h: *mut u32, format: *mut u8) -> bool {
    let Some(c) = context::current() else {
        return false;
    };
    let Some((g, tw, th, f)) = fb_target_n(c, n) else {
        return false;
    };
    // SAFETY: the C contract above.
    unsafe {
        gpu.write_unaligned(g);
        w.write_unaligned(tw);
        h.write_unaligned(th);
        format.write_unaligned(f);
    }
    true
}

/// `enum gles_surf_format` of the bound target (RGBA8888 without a context).
#[no_mangle]
pub unsafe extern "C" fn gl_fb_target_format() -> u8 {
    context::current().map_or(GLES_SURF_RGBA8888, |c| fb_target_format(c))
}

#[no_mangle]
pub unsafe extern "C" fn gl_fb_flip_y() -> bool {
    context::current().is_none_or(|c| fb_flip_y(c))
}

#[no_mangle]
pub unsafe extern "C" fn gl_fb_texture_deleted(texture: GLuint) {
    if let Some(c) = context::current() {
        texture_deleted(c, texture);
    }
}

#[no_mangle]
pub unsafe extern "C" fn gl_fb_renderbuffer_deleted(renderbuffer: GLuint) {
    if let Some(c) = context::current() {
        renderbuffer_deleted(c, renderbuffer);
    }
}

/* --------------------------------------------------------------------------- */
/* framebuffer objects */

#[no_mangle]
pub unsafe extern "C" fn glGenFramebuffers(n: GLsizei, framebuffers: *mut GLuint) {
    gl_trace!(glGenFramebuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glGenFramebuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if framebuffers.is_null() {
        return;
    }
    let n = n as usize;
    let mut made = 0;

    for h in 1..MAX_FRAMEBUFFERS {
        if made >= n {
            break;
        }
        if !c.framebuffers[h].used {
            c.framebuffers[h] = FRAMEBUFFER_ZERO;
            c.framebuffers[h].used = true;
            // SAFETY: the application passes room for `n` names; GLuint
            // arrays are not assumed aligned.
            unsafe { framebuffers.add(made).write_unaligned(h as GLuint) };
            made += 1;
        }
    }
    if made < n {
        /* the fixed pool is full: the rest are 0, and a client that binds
         * name 0 hits the default framebuffer instead (upstream 7219228) */
        c.set_error(GL_OUT_OF_MEMORY, "glGenFramebuffers");
    }
    for i in made..n {
        // SAFETY: as above.
        unsafe { framebuffers.add(i).write_unaligned(0) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteFramebuffers(n: GLsizei, framebuffers: *const GLuint) {
    gl_trace!(glDeleteFramebuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glDeleteFramebuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if framebuffers.is_null() {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: the application passes `n` names (n >= 0 checked above),
        // not necessarily aligned.
        let Some(h) = names::slot_of(c, Kind::Framebuffer, unsafe { framebuffers.add(i).read_unaligned() }) else {
            continue;
        };
        if c.framebuffer_ref(h).is_none() {
            continue;
        }
        names::forget(c, Kind::Framebuffer, h);
        if c.bound_framebuffer == h {
            c.bound_framebuffer = 0; /* GL: rebinds the default */
        }
        if c.read_fb_split && c.bound_read_framebuffer == h {
            c.bound_read_framebuffer = 0;
        }
        let held: [(GLenum, GLuint); CAP_COLOR_ATTACHMENTS + 2] = {
            let fb = &c.framebuffers[h as usize];
            let mut held = [(GL_NONE, 0); CAP_COLOR_ATTACHMENTS + 2];
            for (dst, a) in held.iter_mut().zip(attachment_points(fb)) {
                *dst = (a.type_, a.name);
            }
            held
        };
        c.framebuffers[h as usize] = FRAMEBUFFER_ZERO;
        for (t, n) in held {
            attachment_released(c, t, n); /* an orphan it alone still held */
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glIsFramebuffer(framebuffer: GLuint) -> GLboolean {
    gl_trace!(glIsFramebuffer);
    /* a generated name is a framebuffer object only once bound (ES 2.0 4.4.1) */
    match context::current()
        .and_then(|c| names::slot_of(c, Kind::Framebuffer, framebuffer).and_then(|s| c.framebuffer_ref(s)))
    {
        Some(fb) if fb.bound => GL_TRUE_B,
        _ => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBindFramebuffer(target: GLenum, framebuffer: GLuint) {
    gl_trace!(glBindFramebuffer);
    let Some(c) = context::current() else { return };
    if !is_fb_target(target) {
        c.set_error(GL_INVALID_ENUM, "glBindFramebuffer");
        return;
    }
    /* the slot the name lives in; a name past the pool borrows one */
    let Some(framebuffer) = names::bind_slot(c, Kind::Framebuffer, framebuffer, |s| !c.framebuffers[s].used) else {
        c.set_error(GL_OUT_OF_MEMORY, "glBindFramebuffer");
        return;
    };
    if framebuffer != 0 {
        let fb = &mut c.framebuffers[framebuffer as usize];
        if !fb.used {
            /* a name not from glGenFramebuffers: created on first bind */
            *fb = FRAMEBUFFER_ZERO;
            fb.used = true;
        }
        fb.bound = true;
    }
    /* ES 3.0 4.4.1: GL_FRAMEBUFFER binds both, the other two one each */
    match target {
        GL_READ_FRAMEBUFFER => {
            if !c.read_fb_split {
                c.read_fb_split = true;
            }
            c.bound_read_framebuffer = framebuffer;
        }
        GL_DRAW_FRAMEBUFFER => {
            if !c.read_fb_split {
                c.read_fb_split = true;
                c.bound_read_framebuffer = c.bound_framebuffer;
            }
            c.bound_framebuffer = framebuffer;
        }
        _ => {
            c.read_fb_split = false;
            c.bound_read_framebuffer = framebuffer;
            c.bound_framebuffer = framebuffer;
        }
    }
}

/// The READ framebuffer (ES 3.0 4.4.1): what glReadPixels and glCopyTex*
/// read, and what a GL_READ_FRAMEBUFFER target names.
pub fn read_framebuffer(c: &GlContext) -> GLuint {
    if c.read_fb_split {
        c.bound_read_framebuffer
    } else {
        c.bound_framebuffer
    }
}

/// Run `f` with the READ framebuffer where the rest of the core looks for
/// "the" framebuffer (`bound_framebuffer`), when it is another one than the
/// draw framebuffer: `f` is the entry point itself, called again, and finds
/// the two equal. A clear still pending for the draw framebuffer goes out
/// first. Returns None when the bindings are one (the caller goes on).
pub fn as_read<R>(c: &mut GlContext, f: impl FnOnce() -> R) -> Option<R> {
    let read = read_framebuffer(c);
    if read == c.bound_framebuffer {
        return None;
    }
    super::draw::flush_pending_clear(c);
    let (draw, split) = (c.bound_framebuffer, c.read_fb_split);
    c.bound_framebuffer = read;
    c.read_fb_split = false;
    let r = f();
    if let Some(c) = context::current() {
        c.bound_framebuffer = draw;
        c.bound_read_framebuffer = read;
        c.read_fb_split = split;
    }
    Some(r)
}

#[no_mangle]
pub unsafe extern "C" fn glFramebufferTexture2D(
    target: GLenum,
    attachment: GLenum,
    textarget: GLenum,
    texture: GLuint,
    level: GLint,
) {
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if as_read(c, || unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, textarget, texture, level) })
                .is_some()
            {
                return;
            }
        }
    }
    gl_trace!(glFramebufferTexture2D);
    const FN: &str = "glFramebufferTexture2D";
    let Some(c) = context::current() else { return };
    let face = (GL_TEXTURE_CUBE_MAP_POSITIVE_X..=GL_TEXTURE_CUBE_MAP_NEGATIVE_Z).contains(&textarget);

    if !is_fb_target(target) || (textarget != GL_TEXTURE_2D && !face) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    if c.bound_framebuffer == 0 {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }

    let bound = c.bound_framebuffer;

    if c.framebuffer_ref(bound).is_none() || !is_attachment_point(attachment) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    /* A name glGenTextures handed out is not a texture until it has been
     * bound, and attaching one that is not is GL_INVALID_OPERATION
     * (ES 2.0 4.4.3): `texture_object`, not `texture_ref`. */
    let texture = match (texture, names::slot_of(c, Kind::Texture, texture)) {
        (0, _) => 0,
        (_, Some(s)) if c.texture_object(s).is_some() => s,
        _ => {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
    };
    if level < 0 || level >= CAP_MIP_LEVELS as GLint {
        c.set_error(GL_INVALID_VALUE, FN);
        return;
    }
    /* `textarget` must name the kind of texture it was given: a cube map face
     * of a 2D texture, or GL_TEXTURE_2D of a cube map, is
     * GL_INVALID_OPERATION (ES 2.0 4.4.3). */
    if let Some(t) = c.texture_object(texture) {
        let is_cube = t.target == GL_TEXTURE_CUBE_MAP;
        if is_cube != face {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
    }
    let layer = if face { textarget.wrapping_sub(GL_TEXTURE_CUBE_MAP_POSITIVE_X) as GLint } else { 0 };
    debug_assert_eq!(bound, c.bound_framebuffer);
    replace_attachment(c, attachment, GL_TEXTURE, texture, level, layer);
    note_level_attached(c, texture, level);
}

/// Rendering into a level fills it: the sampler may then walk down to it
/// (Qt prefilters a light probe level by level). Detaching (`texture` 0)
/// fills nothing.
fn note_level_attached(c: &mut GlContext, texture: GLuint, level: GLint) {
    if let Some(t) = c.texture(texture) {
        if level >= 0 && (t.levels_written as GLint) < level + 1 {
            t.levels_written = (level + 1) as u8;
        }
    }
}

/// OpenGL ES 3.0: one layer of a 2D texture array (or face of a cube map) as
/// the attachment.
#[no_mangle]
pub unsafe extern "C" fn glFramebufferTextureLayer(
    target: GLenum,
    attachment: GLenum,
    texture: GLuint,
    level: GLint,
    layer: GLint,
) {
    const FN: &str = "glFramebufferTextureLayer";
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if as_read(c, || unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, attachment, texture, level, layer) })
                .is_some()
            {
                return;
            }
        }
    }
    gl_trace!(glFramebufferTextureLayer);
    let Some(c) = context::current() else { return };
    if !is_fb_target(target) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    if c.bound_framebuffer == 0 {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }

    let bound = c.bound_framebuffer;

    if c.framebuffer_ref(bound).is_none() || !is_attachment_point(attachment) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    // As glFramebufferTexture2D: a name with no object behind it.
    let texture = match (texture, names::slot_of(c, Kind::Texture, texture)) {
        (0, _) => 0,
        (_, Some(s)) if c.texture_object(s).is_some() => s,
        _ => {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
    };
    /* ES 3.0 4.4.2: a 3D or a 2D array texture, nothing else (dEQP
     * negative_api.buffer.framebuffer_texture_layer hands a 2D one) */
    if texture != 0 && !c.texture_ref(texture).is_some_and(|t| matches!(t.target, GL_TEXTURE_3D | GL_TEXTURE_2D_ARRAY))
    {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    if level < 0 || level >= CAP_MIP_LEVELS as GLint || !(0..=255).contains(&layer) {
        c.set_error(GL_INVALID_VALUE, FN);
        return;
    }
    if let Some(fb) = c.framebuffer(bound) {
        set_attachment(fb, attachment, GL_TEXTURE, texture, level, layer);
    }
    note_level_attached(c, texture, level);
}

#[no_mangle]
pub unsafe extern "C" fn glFramebufferRenderbuffer(
    target: GLenum,
    attachment: GLenum,
    renderbuffertarget: GLenum,
    renderbuffer: GLuint,
) {
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if as_read(c, || unsafe {
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, renderbuffertarget, renderbuffer)
            })
            .is_some()
            {
                return;
            }
        }
    }
    gl_trace!(glFramebufferRenderbuffer);
    const FN: &str = "glFramebufferRenderbuffer";
    let Some(c) = context::current() else { return };
    if !is_fb_target(target) || renderbuffertarget != GL_RENDERBUFFER {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    if c.bound_framebuffer == 0 {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }

    let bound = c.bound_framebuffer;

    if c.framebuffer_ref(bound).is_none() || !is_attachment_point(attachment) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    /* As glFramebufferTexture2D: a generated but never bound name is no
     * renderbuffer object yet, and attaching it is GL_INVALID_OPERATION. */
    let renderbuffer = match (renderbuffer, names::slot_of(c, Kind::Renderbuffer, renderbuffer)) {
        (0, _) => 0,
        (_, Some(s)) if c.renderbuffer_object(s).is_some() => s,
        _ => {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
    };
    debug_assert_eq!(bound, c.bound_framebuffer);
    replace_attachment(c, attachment, GL_RENDERBUFFER, renderbuffer, 0, 0);
}

#[no_mangle]
pub unsafe extern "C" fn glCheckFramebufferStatus(target: GLenum) -> GLenum {
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if let Some(r) = as_read(c, || unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }) {
                return r;
            }
        }
    }
    gl_trace!(glCheckFramebufferStatus);
    let Some(c) = context::current() else { return 0 };
    /* ES 3.0: the draw and read targets name the one binding (see glBindFramebuffer) */
    if !is_fb_target(target) {
        c.set_error(GL_INVALID_ENUM, "glCheckFramebufferStatus");
        return 0;
    }
    if c.bound_framebuffer == 0 {
        return GL_FRAMEBUFFER_COMPLETE;
    }

    match c.framebuffer_ref(c.bound_framebuffer) {
        Some(fb) => check_status(c, fb),
        None => GL_FRAMEBUFFER_UNSUPPORTED,
    }
}

/// Every `pname` `glGetFramebufferAttachmentParameteriv` accepts in ES 3.0,
/// whether or not this library can answer it for an attached object. It says
/// "this enum is a framebuffer attachment query", which is what separates
/// INVALID_OPERATION (a good enum the state cannot answer) from INVALID_ENUM
/// (not one of these at all).
fn is_attachment_pname(pname: GLenum) -> bool {
    matches!(
        pname,
        GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE
            | GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME
            | GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL
            | GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE
            | GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER
            | GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING
            | GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE
            | GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE
            | GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE
            | GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE
            | GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE
            | GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE
            | GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE
    )
}

/// What an attachment's image is, for the ES 3.0 size/type/encoding queries:
/// (red, green, blue, alpha, depth, stencil bits, component type, encoding).
struct AttachmentBits {
    rgba: [GLint; 4],
    depth: GLint,
    stencil: GLint,
    component_type: GLenum,
    encoding: GLenum,
}

impl AttachmentBits {
    const NONE: AttachmentBits =
        AttachmentBits { rgba: [0; 4], depth: 0, stencil: 0, component_type: GL_NONE, encoding: GL_LINEAR };

    fn color(rgba: [GLint; 4], component_type: GLenum) -> AttachmentBits {
        AttachmentBits { rgba, component_type, ..AttachmentBits::NONE }
    }
}

/// The bits of a renderbuffer of `internalformat` (0: no storage yet, every
/// size 0 -- ES 3.0 6.1.13 for an unspecified image).
fn renderbuffer_bits(internalformat: GLenum) -> AttachmentBits {
    let un = GL_UNSIGNED_NORMALIZED;
    match internalformat {
        GL_RGBA4 => AttachmentBits::color([4, 4, 4, 4], un),
        GL_RGB5_A1 => AttachmentBits::color([5, 5, 5, 1], un),
        GL_RGB565 => AttachmentBits::color([5, 6, 5, 0], un),
        GL_RGB8 | GL_RGB => AttachmentBits::color([8, 8, 8, 0], un),
        GL_RGBA8 | GL_RGBA | GL_BGRA_EXT | GL_BGRA8_EXT => AttachmentBits::color([8, 8, 8, 8], un),
        GL_SRGB8_ALPHA8 => AttachmentBits { encoding: GL_SRGB, ..AttachmentBits::color([8, 8, 8, 8], un) },
        GL_R8 => AttachmentBits::color([8, 0, 0, 0], un),
        GL_RG8 => AttachmentBits::color([8, 8, 0, 0], un),
        GL_RGBA16F => AttachmentBits::color([16, 16, 16, 16], GL_FLOAT),
        GL_R16F => AttachmentBits::color([16, 0, 0, 0], GL_FLOAT),
        GL_RG16F => AttachmentBits::color([16, 16, 0, 0], GL_FLOAT),
        GL_R32F => AttachmentBits::color([32, 0, 0, 0], GL_FLOAT),
        GL_DEPTH_COMPONENT16 => AttachmentBits { depth: 16, component_type: un, ..AttachmentBits::NONE },
        GL_DEPTH_COMPONENT24 => AttachmentBits { depth: 24, component_type: un, ..AttachmentBits::NONE },
        GL_DEPTH_COMPONENT32F => AttachmentBits { depth: 32, component_type: GL_FLOAT, ..AttachmentBits::NONE },
        GL_DEPTH24_STENCIL8 | GL_DEPTH_STENCIL => {
            AttachmentBits { depth: 24, stencil: 8, component_type: un, ..AttachmentBits::NONE }
        }
        GL_DEPTH32F_STENCIL8 => {
            AttachmentBits { depth: 32, stencil: 8, component_type: GL_FLOAT, ..AttachmentBits::NONE }
        }
        GL_STENCIL_INDEX8 => AttachmentBits { stencil: 8, component_type: GL_UNSIGNED_INT, ..AttachmentBits::NONE },
        _ => AttachmentBits::NONE,
    }
}

/// The bits of a texture level's image, from the format and type it was
/// specified with (the store does not keep the sized name; an RGB565 or
/// RGBA4 texture reads as 8 bits a channel, which is what it is stored at).
fn texture_bits(t: &context::GlTexture) -> AttachmentBits {
    if t.mem.impl_.is_null() && t.width == 0 {
        return AttachmentBits::NONE;
    }
    let float = matches!(t.type_, GL_HALF_FLOAT | GL_HALF_FLOAT_OES | GL_FLOAT);
    let (bits, ct) = if float {
        (if t.type_ == GL_FLOAT && t.texfmt != GLES_TEXFMT_RGBA16F { 32 } else { 16 }, GL_FLOAT)
    } else {
        (8, GL_UNSIGNED_NORMALIZED)
    };
    match t.format {
        GL_RGBA | GL_BGRA_EXT => AttachmentBits::color([bits; 4], ct),
        GL_RGB => AttachmentBits::color([bits, bits, bits, 0], ct),
        GL_RG => AttachmentBits::color([bits, bits, 0, 0], ct),
        GL_RED | GL_LUMINANCE => AttachmentBits::color([bits, 0, 0, 0], ct),
        GL_ALPHA => AttachmentBits::color([0, 0, 0, bits], ct),
        GL_LUMINANCE_ALPHA => AttachmentBits::color([bits, 0, 0, bits], ct),
        GL_DEPTH_COMPONENT => {
            AttachmentBits { depth: 24, component_type: GL_UNSIGNED_NORMALIZED, ..AttachmentBits::NONE }
        }
        GL_DEPTH_STENCIL => {
            AttachmentBits { depth: 24, stencil: 8, component_type: GL_UNSIGNED_NORMALIZED, ..AttachmentBits::NONE }
        }
        _ => AttachmentBits::NONE,
    }
}

/// The answer to one of the ES 3.0 image pnames for `b`, None for another pname.
fn bits_pname(b: &AttachmentBits, pname: GLenum) -> Option<GLint> {
    Some(match pname {
        GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE => b.rgba[0],
        GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE => b.rgba[1],
        GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE => b.rgba[2],
        GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE => b.rgba[3],
        GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE => b.depth,
        GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE => b.stencil,
        GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE => b.component_type as GLint,
        GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING => b.encoding as GLint,
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn glGetFramebufferAttachmentParameteriv(
    target: GLenum,
    attachment: GLenum,
    pname: GLenum,
    params: *mut GLint,
) {
    gl_trace!(glGetFramebufferAttachmentParameteriv);
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if as_read(c, || unsafe {
                glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, pname, params)
            })
            .is_some()
            {
                return;
            }
        }
    }
    const FN: &str = "glGetFramebufferAttachmentParameteriv";
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if !is_fb_target(target) {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    let es3 = !crate::REPORT_ES2;
    let put = |v: GLint| {
        // SAFETY: a non-null `params` points at a writable GLint (the GL
        // contract), not necessarily aligned.
        unsafe { params.write_unaligned(v) }
    };
    if c.bound_framebuffer == 0 {
        /* ES 3.0 6.1.13: the default framebuffer's images are named
         * GL_BACK, GL_DEPTH and GL_STENCIL, their type is
         * GL_FRAMEBUFFER_DEFAULT, and their sizes are the surface's (the
         * *_BITS queries). An attachment point name is GL_INVALID_OPERATION.
         * ES 2.0 has none of it: every query here is GL_INVALID_OPERATION. */
        if !es3 || !matches!(attachment, GL_BACK | GL_DEPTH | GL_STENCIL) {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
        let b = match attachment {
            GL_BACK => AttachmentBits::color([8, 8, 8, 8], GL_UNSIGNED_NORMALIZED),
            GL_DEPTH => AttachmentBits { depth: 24, component_type: GL_UNSIGNED_NORMALIZED, ..AttachmentBits::NONE },
            _ => AttachmentBits { stencil: 8, component_type: GL_UNSIGNED_INT, ..AttachmentBits::NONE },
        };
        match pname {
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE => put(GL_FRAMEBUFFER_DEFAULT as GLint),
            _ => match bits_pname(&b, pname) {
                Some(v) => put(v),
                None => c.set_error(GL_INVALID_ENUM, FN),
            },
        }
        return;
    }

    let bound = c.bound_framebuffer;
    let Some(fb) = c.framebuffer_ref(bound) else {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    };
    let a = match attachment {
        GL_COLOR_ATTACHMENT0 => fb.color,
        GL_DEPTH_ATTACHMENT => fb.depth,
        GL_STENCIL_ATTACHMENT => fb.stencil,
        /* ES 3.0 6.1.13: the packed point is queryable when depth and
         * stencil name the same object */
        GL_DEPTH_STENCIL_ATTACHMENT => {
            let (d, s) = (fb.depth, fb.stencil);
            if (d.type_, d.name, d.level, d.layer) != (s.type_, s.name, s.level, s.layer) {
                c.set_error(GL_INVALID_OPERATION, FN);
                return;
            }
            d
        }
        /* a colour attachment past 0 that exists as a point: ES 3.0, and
         * ES 2.0 with GL_NV_fbo_color_attachments ("Accepted by the
         * <attachment> parameter of ... GetFramebufferAttachmentParameteriv:
         * COLOR_ATTACHMENTi_NV"), which this GL claims on both surfaces */
        n if n > GL_COLOR_ATTACHMENT0 && n < GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum => {
            fb.color_n[(n - GL_COLOR_ATTACHMENT0) as usize - 1]
        }
        /* GL_BACK and friends name the default framebuffer's images */
        GL_BACK | GL_DEPTH | GL_STENCIL if es3 => {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FN);
            return;
        }
    };

    /* GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE is the face the
     * attachment names, or 0 when the texture is not a cube map. The face is
     * kept in `a.layer`, which an array texture uses for its layer instead
     * (glFramebufferTextureLayer), so it is only a face when the texture
     * really is a cube map. */
    /* (`a.name` is a RENDERBUFFER name for a renderbuffer attachment, and the
     * two name spaces are separate, so the kind is checked first) */
    let tex_target = if a.type_ == GL_TEXTURE { c.texture_ref(a.name).map_or(0, |t| t.target) } else { 0 };
    let cube_face = tex_target == GL_TEXTURE_CUBE_MAP;

    /* An attachment point with nothing attached. The two versions answer
     * differently, and the difference is not cosmetic:
     *
     *   ES 2.0 (4.4.5): only OBJECT_TYPE may be asked. Anything else,
     *   OBJECT_NAME included, is INVALID_ENUM.
     *
     *   ES 3.0 (6.1.13): OBJECT_NAME is legal and answers 0, and any OTHER
     *   attachment pname is INVALID_OPERATION -- not INVALID_ENUM, because
     *   the enum is perfectly valid; it is the state that makes it
     *   unanswerable. A pname that is not an attachment query at all stays
     *   INVALID_ENUM in both.
     */
    if a.type_ == GL_NONE && es3 && pname != GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE {
        if !is_attachment_pname(pname) {
            c.set_error(GL_INVALID_ENUM, FN);
        } else if pname == GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME {
            put(0);
        } else {
            c.set_error(GL_INVALID_OPERATION, FN);
        }
        return;
    }

    let v = match pname {
        GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE => a.type_ as GLint,
        GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME => {
            if a.type_ == GL_NONE {
                c.set_error(GL_INVALID_ENUM, FN);
                return;
            }
            /* the attachment keeps the slot; the application knows it by its name */
            let kind = if a.type_ == GL_TEXTURE { Kind::Texture } else { Kind::Renderbuffer };
            names::name_of(c, kind, a.name) as GLint
        }
        GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL | GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE => {
            if a.type_ != GL_TEXTURE {
                c.set_error(GL_INVALID_ENUM, FN);
                return;
            }
            if pname == GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL {
                a.level
            } else if cube_face {
                GL_TEXTURE_CUBE_MAP_POSITIVE_X as GLint + a.layer
            } else {
                0
            }
        }
        /* ES 3.0: the layer of an array or 3D texture attachment, 0 otherwise */
        GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER if es3 => {
            if a.type_ != GL_TEXTURE {
                c.set_error(GL_INVALID_ENUM, FN);
                return;
            }
            if matches!(tex_target, GL_TEXTURE_2D_ARRAY | GL_TEXTURE_3D) {
                a.layer
            } else {
                0
            }
        }
        /* ES 3.0: the attached image's sizes, component type and encoding.
         * GL_DEPTH_STENCIL_ATTACHMENT has no one component type (6.1.13). */
        p if es3 && bits_pname(&AttachmentBits::NONE, p).is_some() => {
            if p == GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE && attachment == GL_DEPTH_STENCIL_ATTACHMENT {
                c.set_error(GL_INVALID_OPERATION, FN);
                return;
            }
            let b = match a.type_ {
                GL_TEXTURE => c.texture_ref(a.name).map_or(AttachmentBits::NONE, texture_bits),
                _ => c.renderbuffer_ref(a.name).map_or(AttachmentBits::NONE, |rb| renderbuffer_bits(rb.internalformat)),
            };
            /* a depth or stencil point reads only its own component */
            let b = match attachment {
                GL_DEPTH_ATTACHMENT => AttachmentBits { rgba: [0; 4], stencil: 0, ..b },
                GL_STENCIL_ATTACHMENT => AttachmentBits { rgba: [0; 4], depth: 0, ..b },
                _ => b,
            };
            bits_pname(&b, p).unwrap_or(0)
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FN);
            return;
        }
    };
    put(v);
}

/* --------------------------------------------------------------------------- */
/* renderbuffers */

#[no_mangle]
pub unsafe extern "C" fn glGenRenderbuffers(n: GLsizei, renderbuffers: *mut GLuint) {
    gl_trace!(glGenRenderbuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glGenRenderbuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if renderbuffers.is_null() {
        return;
    }
    let n = n as usize;
    let mut made = 0;

    for h in 1..MAX_RENDERBUFFERS {
        if made >= n {
            break;
        }
        if !c.objs().renderbuffers[h].used {
            c.objs().renderbuffers[h] = RENDERBUFFER_ZERO;
            c.objs().renderbuffers[h].used = true;
            c.objs().renderbuffers[h].serial = context::next_object_stamp();
            // SAFETY: the application passes room for `n` names; GLuint
            // arrays are not assumed aligned.
            unsafe { renderbuffers.add(made).write_unaligned(h as GLuint) };
            made += 1;
        }
    }
    if made < n {
        /* the fixed pool is full (upstream 7219228) */
        c.set_error(GL_OUT_OF_MEMORY, "glGenRenderbuffers");
    }
    for i in made..n {
        // SAFETY: as above.
        unsafe { renderbuffers.add(i).write_unaligned(0) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteRenderbuffers(n: GLsizei, renderbuffers: *const GLuint) {
    gl_trace!(glDeleteRenderbuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glDeleteRenderbuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if renderbuffers.is_null() {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: the application passes `n` names (n >= 0 checked above),
        // not necessarily aligned.
        let Some(h) = names::slot_of(c, Kind::Renderbuffer, unsafe { renderbuffers.add(i).read_unaligned() }) else {
            continue;
        };
        if c.renderbuffer(h).is_none() {
            continue;
        }
        names::forget(c, Kind::Renderbuffer, h);
        if renderbuffer_deleted(c, h) {
            /* still held by a framebuffer that is not bound: the image
             * stays, the name goes (the orphan rule above) */
            if c.bound_renderbuffer == h {
                c.bound_renderbuffer = 0;
            }
            if let Some(rb) = c.renderbuffer(h) {
                rb.orphan = true;
            }
        } else {
            free_renderbuffer(c, h);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glIsRenderbuffer(renderbuffer: GLuint) -> GLboolean {
    gl_trace!(glIsRenderbuffer);
    /* a generated name is a renderbuffer object only once bound */
    match context::current()
        .and_then(|c| names::slot_of(c, Kind::Renderbuffer, renderbuffer).and_then(|s| c.renderbuffer_object(s)))
    {
        Some(_) => GL_TRUE_B,
        None => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBindRenderbuffer(target: GLenum, renderbuffer: GLuint) {
    gl_trace!(glBindRenderbuffer);
    let Some(c) = context::current() else { return };
    if target != GL_RENDERBUFFER {
        c.set_error(GL_INVALID_ENUM, "glBindRenderbuffer");
        return;
    }
    /* the slot the name lives in; a name past the pool borrows one */
    let Some(renderbuffer) =
        names::bind_slot(c, Kind::Renderbuffer, renderbuffer, |s| !c.objs_ref().renderbuffers[s].used)
    else {
        c.set_error(GL_OUT_OF_MEMORY, "glBindRenderbuffer");
        return;
    };
    if renderbuffer != 0 {
        if c.objs().renderbuffers[renderbuffer as usize].orphan {
            /* the deleted name bound again: a new object under the name;
             * the orphan moves to a free slot and the framebuffers that
             * hold it follow (the orphan rule) */
            let free = (1..MAX_RENDERBUFFERS).find(|&i| !c.objs().renderbuffers[i].used);
            if let Some(to) = free {
                let objs = c.objs();
                objs.renderbuffers[to] =
                    core::mem::replace(&mut objs.renderbuffers[renderbuffer as usize], RENDERBUFFER_ZERO);
                orphan_moved(c, GL_RENDERBUFFER, renderbuffer, to as GLuint);
                if c.bound_renderbuffer == to as GLuint {
                    c.bound_renderbuffer = 0;
                }
            } else {
                glog!("renderbuffer {}: no free slot for its orphan; the framebuffers follow the name", renderbuffer);
                free_renderbuffer(c, renderbuffer);
            }
        }
        let rb = &mut c.objs().renderbuffers[renderbuffer as usize];
        if !rb.used {
            *rb = RENDERBUFFER_ZERO;
            rb.used = true;
            rb.serial = context::next_object_stamp();
        }
        rb.bound = true;
    }
    c.bound_renderbuffer = renderbuffer;
}

/// glRenderbufferStorage on the current context (glRenderbufferStorageMultisample
/// lands here too, so errors name glRenderbufferStorage as they do in C).
/// A format glRenderbufferStorage takes: colour-, depth- or stencil-renderable.
fn is_renderable_format(f: GLenum) -> bool {
    is_renderbuffer_color_format(f) || is_depth_format(f) || is_stencil_format(f)
}

/// glRenderbufferStorage's argument rules (ES 3.0 4.4.2), shared with the
/// multisample form: the target, then the format, then the size.
fn renderbuffer_storage_args_ok(
    c: &mut GlContext,
    target: GLenum,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
    fn_name: &str,
) -> bool {
    if target != GL_RENDERBUFFER {
        c.set_error(GL_INVALID_ENUM, fn_name);
        return false;
    }
    if !is_renderable_format(internalformat) {
        c.set_error(GL_INVALID_ENUM, fn_name);
        return false;
    }
    if width < 0 || height < 0 || width > 4096 || height > 4096 {
        c.set_error(GL_INVALID_VALUE, fn_name);
        return false;
    }
    true
}

fn renderbuffer_storage(c: &mut GlContext, target: GLenum, internalformat: GLenum, width: GLsizei, height: GLsizei) {
    const FN: &str = "glRenderbufferStorage";
    if !renderbuffer_storage_args_ok(c, target, internalformat, width, height, FN) {
        return;
    }

    let bound = c.bound_renderbuffer;
    let Some(rb) = c.renderbuffer(bound) else {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    };

    context::release_storage(&mut rb.mem, &mut rb.external);
    if is_color_format(internalformat) && width > 0 && height > 0 {
        /* Linear, in the surface the format maps to: RGBA8888 for everything
         * with eight bits or fewer a channel (the PBE writes that layout
         * whatever the narrower format asked for), the float surfaces for the
         * half- and single-float names, which are wider than RGBA8 and were
         * under-allocated while every renderbuffer was RGBA8. */
        let bytes = context::surface_bytes(
            width as u32,
            height as u32,
            gles_surf_bytes_per_pixel(renderbuffer_surf_format(internalformat)),
        );

        if osal::gpu_alloc(context::heap_data(), bytes, OSAL_MEM_DATA, &mut rb.mem) != 0 {
            c.set_error(GL_OUT_OF_MEMORY, FN);
            return;
        }
        /* pool memory: a read-back before the first draw shows no earlier
         * user's pixels (one pass over the new storage) */
        let cleared = context::clear_storage(&mut rb.mem);
        rb.internalformat = internalformat;
        rb.width = width as u32;
        rb.height = height as u32;
        rb.storage_gen = context::respecified(rb.serial, false);
        if !cleared {
            c.set_error(GL_OUT_OF_MEMORY, FN);
        }
        return;
    }
    rb.internalformat = internalformat;
    rb.width = width as u32;
    rb.height = height as u32;
    rb.storage_gen = context::respecified(rb.serial, false);
}

#[no_mangle]
pub unsafe extern "C" fn glRenderbufferStorage(
    target: GLenum,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
) {
    gl_trace!(glRenderbufferStorage);
    let Some(c) = context::current() else { return };
    renderbuffer_storage(c, target, internalformat, width, height);
}

#[no_mangle]
pub unsafe extern "C" fn glGetRenderbufferParameteriv(target: GLenum, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetRenderbufferParameteriv);
    const FN: &str = "glGetRenderbufferParameteriv";
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    if target != GL_RENDERBUFFER {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }

    let Some(rb) = c.renderbuffer_ref(c.bound_renderbuffer) else {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    };
    let f = rb.internalformat;

    let v = match pname {
        GL_RENDERBUFFER_WIDTH => rb.width as GLint,
        GL_RENDERBUFFER_HEIGHT => rb.height as GLint,
        GL_RENDERBUFFER_INTERNAL_FORMAT => (if f != 0 { f } else { GL_RGBA4 }) as GLint,
        GL_RENDERBUFFER_RED_SIZE
        | GL_RENDERBUFFER_GREEN_SIZE
        | GL_RENDERBUFFER_BLUE_SIZE
        | GL_RENDERBUFFER_ALPHA_SIZE => {
            if is_color_format(f) {
                8
            } else {
                0
            }
        }
        /* ES 3.0: single-sampled storage only (glRenderbufferStorageMultisample
         * refuses more than one sample) */
        GL_RENDERBUFFER_SAMPLES if !crate::REPORT_ES2 => 0,
        GL_RENDERBUFFER_DEPTH_SIZE => match f {
            GL_DEPTH_COMPONENT16 => 16,
            GL_DEPTH_COMPONENT24 | GL_DEPTH24_STENCIL8 => 24,
            GL_DEPTH_COMPONENT32F | GL_DEPTH32F_STENCIL8 => 32,
            _ => 0,
        },
        GL_RENDERBUFFER_STENCIL_SIZE => {
            if is_stencil_format(f) {
                8
            } else {
                0
            }
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FN);
            return;
        }
    };
    // SAFETY: a non-null `params` points at a writable GLint (the GL
    // contract), not necessarily aligned.
    unsafe { params.write_unaligned(v) };
}

/* --------------------------------------------------------------------------- */
/* glReadPixels */

#[no_mangle]
pub unsafe extern "C" fn glReadPixels(
    x: GLint,
    y: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *mut c_void,
) {
    gl_trace!(glReadPixels);
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if let Some(c) = context::current() {
        if as_read(c, || unsafe { glReadPixels(x, y, width, height, format, type_, pixels) }).is_some() {
            return;
        }
        /* and its glReadBuffer attachment */
        if as_read_attachment(c, || unsafe { glReadPixels(x, y, width, height, format, type_, pixels) }).is_some() {
            return;
        }
    }
    let Some(c) = context::current() else { return };
    /* With PIXEL_PACK_BUFFER bound, `pixels` is an offset into that buffer
     * rather than a client address (ES 3.0 4.3.1). */
    /* ES 3.0 4.3.2: nothing to read with the read buffer GL_NONE */
    let none = if c.bound_framebuffer == 0 {
        c.default_read_none
    } else {
        c.framebuffer_ref(c.bound_framebuffer).is_some_and(|fb| fb.read_none)
    };
    if none {
        c.set_error(GL_INVALID_OPERATION, "glReadPixels");
        return;
    }
    let Some(pixels) = super::buffer::pixel_buffer_resolve(c, true, pixels, 0, "glReadPixels") else {
        return;
    };
    /* ES 3.0 4.3.2's pack state: rows GL_PACK_ROW_LENGTH pixels apart
     * when it is set, and the first pixel written after the skipped rows
     * and pixels (both honoured since 2026-09-24) */
    let pack_align = c.pack_alignment as usize;
    let row_len = c.pack_row_length.max(0) as usize;
    let out_bpp = if type_ == GL_FLOAT { 16 } else { 4 };
    let a = pack_align.max(1);
    let stride = (if row_len > 0 { row_len } else { width.max(0) as usize } * out_bpp).div_ceil(a) * a;
    let skip = stride * c.pack_skip_rows.max(0) as usize + out_bpp * c.pack_skip_pixels.max(0) as usize;
    let pixels =
        if pixels.is_null() { pixels as *mut c_void } else { (pixels as *mut u8).wrapping_add(skip) as *mut c_void };
    // SAFETY: the caller's contract, forwarded (see `read_pixels`).
    unsafe { read_pixels(c, x, y, width, height, format, type_, pixels, true, pack_align, row_len) };
}

/// Everything recorded so far reaches memory, and the target comes back into
/// the CPU's view. Both cost real time -- the invalidate walks the whole
/// surface -- so a caller reading one frame many times over (a
/// framebuffer-to-texture copy is hundreds of rows) does this once and then
/// calls [`read_pixels`] with `sync` false per chunk. Through glReadPixels
/// each of those rows would submit the frame, wait for the GPU and
/// invalidate the surface again.
///
/// False when the GPU work did not complete (a submit that failed, a wait
/// that timed out or found its kick thrown away by a reset): the target then
/// holds a picture that was never drawn, and the caller must not hand it
/// out. The backend has counted it (`backend::gpu_work_lost`). That covers a
/// loss this read's own flush and finish met, and a loss since this
/// context's previous read that an earlier call submitted and dropped
/// (glFlush and glFinish return nothing): see `LOSS_SEEN`. Only the first
/// read after a loss withholds; the counter is not tied to a target.
#[must_use]
pub fn read_pixels_sync(c: &mut GlContext) -> bool {
    let Some(ops) = osal::ops() else { return true };

    draw::flush_pending_clear(c);
    /* both, whatever the first says: the wait still retires what did run */
    let submitted = backend::flush() == 0;
    let finished = backend::finish() == 0;
    let lost = backend::gpu_work_lost();
    // SAFETY: single-threaded by contract; no other reference is live.
    let seen = core::mem::replace(unsafe { &mut LOSS_SEEN.get()[c.index as usize % MAX_CONTEXTS] }, lost);
    if !(submitted && finished) || lost != seen {
        return false;
    }
    let Some((loc, tw, th, toff)) = target_loc(c) else {
        return true;
    };
    let bpp = gles_surf_bytes_per_pixel(fb_target_format(c)) as usize;
    let m: *mut OsalGpuMem = mem_at_mut(c, loc);
    // SAFETY: `m` points at a descriptor inside the current context.
    let cpu = unsafe { (*m).cpu } as *const u8;

    if !cpu.is_null() {
        if let Some(invd) = ops.cache_invd {
            // SAFETY: the target's CPU mapping covers the attached level's
            // pitch x height pixels from `toff` on.
            unsafe { invd(cpu.add(toff as usize) as *mut c_void, loc_pitch(c, loc, tw) as usize * th as usize * bpp) };
        }
    }
    true
}

/// The rows of glReadPixels, without the flush, finish and invalidate
/// [`read_pixels_sync`] does (glCopyTexSubImage2D reads the framebuffer
/// through it too, once per chunk, having synced once). `pixels` is trusted
/// as glReadPixels trusts it: null, or room for `height` rows of `width` RGBA
/// pixels (16-byte float pixels for RGBA/FLOAT), each row starting at a
/// multiple of `pack_align`. Errors are raised as glReadPixels.
///
/// `pack_align` is GL_PACK_ALIGNMENT (1, 2, 4 or 8) for glReadPixels, and 1
/// for glCopyTexSubImage2D: the pack state describes transfers into the
/// *application's* memory, and a copy between two GL objects never touches it
/// (ES 2.0 4.3.1). Rows are padded, never their contents: the bytes between
/// `width * bpp` and the next multiple of the alignment belong to the caller
/// and are not written.
///
/// # Safety
/// `pixels` must be null or valid for `height` rows of `width * 4` (FLOAT:
/// 16) bytes at the `pack_align` stride.
pub unsafe fn read_pixels(
    c: &mut GlContext,
    x: GLint,
    y: GLint,
    width: GLsizei,
    height: GLsizei,
    format: GLenum,
    type_: GLenum,
    pixels: *mut c_void,
    sync: bool,
    pack_align: usize,
    pack_row_len: usize,
) {
    const FN: &str = "glReadPixels";
    let Some(ops) = osal::ops() else { return };
    if width < 0 || height < 0 {
        c.set_error(GL_INVALID_VALUE, FN);
        return;
    }
    /* the target's real format: a float FBO attachment is read back either as
     * RGBA/UNSIGNED_BYTE (clamped) or as RGBA/FLOAT (ES 3.0's read type for
     * float colour buffers) */
    let fmt = fb_target_format(c);
    let float_target = gles_surf_rt_class(fmt) != 0;

    if format != GL_RGBA || (type_ != GL_UNSIGNED_BYTE && !(float_target && type_ == GL_FLOAT)) {
        /* GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE is RGBA/UNSIGNED_BYTE: the
         * only pair every GLES2 implementation must accept, and the layout
         * the colour buffers are stored in. Known enums in another
         * combination are GL_INVALID_OPERATION (ES 2.0 4.3.1, ES 3.0 4.3.2),
         * unknown ones GL_INVALID_ENUM. */
        let known = texture::is_pixel_format_enum(format) && texture::is_pixel_type_enum(type_);
        c.set_error(if known { GL_INVALID_OPERATION } else { GL_INVALID_ENUM }, FN);
        return;
    }
    if pixels.is_null() || width == 0 || height == 0 {
        return;
    }

    let Some((loc, tw, th, toff)) = target_loc(c) else {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FN);
        return;
    };
    let mut io_failed = false;

    /* Unlike the C, the sync comes after the argument checks and the empty
     * cases: a glReadPixels the spec has nothing to read for does not stall
     * the GPU (the C flushes and finishes first, whatever the arguments). */
    /* Lost GPU work leaves the rows unwritten: ES 2.0/3.0 have no error for
     * it, and a picture that was never drawn is worse than none (the backend
     * counted it and logged the first). */
    if sync && !read_pixels_sync(c) {
        return;
    }
    let flip = fb_flip_y(c);
    /* The scan-out surface is B,G,R,X; on a GPU whose pixel engine has no
     * RGBA8888 render format, an RGBA8888 target is B,G,R,A as well. Both are
     * handed back to the application as R,G,B,A below. */
    /* ...and a texture target is B,G,R,A only once a draw has rendered into
     * it (`pe_order`: its texels were converted then); one the CPU alone
     * wrote is still R,G,B,A */
    let texture_cpu_order =
        matches!(target_loc(c), Some((MemLoc::Texture(i), ..)) if !c.objs_ref().textures[i].pe_order);
    let bgra = fmt == GLES_SURF_XRGB8888
        || (fmt == GLES_SURF_RGBA8888 && backend::rt_rgba8888_is_bgra() && !texture_cpu_order);
    /* The scan-out surface's fourth byte is the X of XRGB8888 and means
     * nothing; an RGBA8888 target stored B,G,R,A carries the alpha the pixel
     * engine blended and wrote, and keeps it. This was a divergence when the
     * port made it; upstream took it at 9e33d7b. */
    let opaque = fmt == GLES_SURF_XRGB8888;
    let bpp = gles_surf_bytes_per_pixel(fmt) as usize;
    let out_bpp = if type_ == GL_FLOAT { 16 } else { 4 };
    let dst = pixels as *mut u8;
    /* the pixels of one row, or GL_PACK_ROW_LENGTH of them when it is set */
    let row_bytes = if pack_row_len > 0 { pack_row_len } else { width as usize } * out_bpp;
    /* ...and where the next row starts: GL_PACK_ALIGNMENT rounds the stride
     * up (ES 2.0 4.3.1). With RGBA/UNSIGNED_BYTE a row is already a multiple
     * of 4, so only alignment 8 with an odd width ever pads -- which is
     * exactly the pair dEQP fails on (`read_pixels.*_align_8`). */
    let a = pack_align.max(1);
    let row_stride = row_bytes.div_ceil(a) * a;
    let pitch = loc_pitch(c, loc, tw) as usize;
    let m: *mut OsalGpuMem = mem_at_mut(c, loc);
    // SAFETY: `m` points at a descriptor inside the current context.
    let cpu = unsafe { (*m).cpu } as *const u8;

    for j in 0..height {
        let gy = y.wrapping_add(j); /* GL window row (bottom-up) */

        /* rows/columns outside the target are left undefined (GL) */
        if gy < 0 || gy as u32 >= th {
            continue;
        }

        let row = if flip { th - 1 - gy as u32 } else { gy as u32 };
        let x0 = if x < 0 { 0 } else { x };
        let mut x1 = x.wrapping_add(width);

        if x1 > tw as GLint {
            x1 = tw as GLint;
        }
        if x1 <= x0 {
            continue;
        }

        let count = (x1 - x0) as usize;
        /* the target's pair-padded pitch, inside the attached level/layer */
        let src_off = toff as usize + (row as usize * pitch + x0 as usize) * bpp;
        let n = count * bpp;
        // SAFETY: the application's buffer holds `height` rows of `width`
        // pixels of out_bpp bytes; this is the part of row j covering columns
        // x0 .. x1 (u8: no alignment requirement).
        let o = unsafe {
            core::slice::from_raw_parts_mut(
                dst.add(j as usize * row_stride + x0.wrapping_sub(x) as usize * out_bpp),
                count * out_bpp,
            )
        };

        if float_target {
            io_failed |= !read_float_row(ops, m, cpu, src_off, fmt, count, o, type_);
            continue;
        }

        if !cpu.is_null() {
            // SAFETY: row < th and x1 <= tw, so the span lies inside the
            // target's CPU mapping (pitch x th pixels of bpp bytes).
            o[..n].copy_from_slice(unsafe { core::slice::from_raw_parts(cpu.add(src_off), n) });
        } else if let Some(gpu_read) = ops.gpu_read {
            // SAFETY: `m` is a live descriptor and `o` has room for n bytes.
            if unsafe { gpu_read(m, src_off, o.as_mut_ptr() as *mut c_void, n) } != 0 {
                /* the platform could not read the pixels: hand back zeros
                 * rather than whatever the buffer held, and say so */
                o.fill(0);
                io_failed = true;
                continue;
            }
        } else {
            o.fill(0);
            io_failed = true;
            continue;
        }
        if fmt == GLES_SURF_RGB565 {
            /* expand the 16-bit pixels in place, last pixel first: the
             * 4-byte destination of pixel i starts at or after its source */
            for i in (0..n / 2).rev() {
                let v = o[i * 2] as u32 | (o[i * 2 + 1] as u32) << 8;
                let (r5, g6, b5) = ((v >> 11) & 0x1f, (v >> 5) & 0x3f, v & 0x1f);

                o[i * 4] = ((r5 << 3) | (r5 >> 2)) as u8;
                o[i * 4 + 1] = ((g6 << 2) | (g6 >> 4)) as u8;
                o[i * 4 + 2] = ((b5 << 3) | (b5 >> 2)) as u8;
                o[i * 4 + 3] = 0xff;
            }
        }
        if bgra {
            /* the surface stores B,G,R,X or B,G,R,A; hand back R,G,B,A */
            for px in o[..n].as_chunks_mut::<4>().0 {
                px.swap(0, 2);
                if opaque {
                    px[3] = 0xff;
                }
            }
        }
    }
    if io_failed {
        /* no GL error fits a failed read-back exactly; OUT_OF_MEMORY is the
         * "the command could not be carried out" one (state unchanged) */
        c.set_error(GL_OUT_OF_MEMORY, FN);
    }
}

/// IEEE half -> float.
pub(crate) fn half_to_float(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let mut exp = ((h >> 10) & 0x1f) as u32;
    let mut mant = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            /* subnormal: normalise */
            exp = 113;
            while mant & 0x400 == 0 {
                mant <<= 1;
                exp -= 1;
            }
            mant &= 0x3ff;
            sign | (exp << 23) | (mant << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        sign | ((exp + 112) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

/// One row of a float colour target (RGBA16F / R16F / R32F texels) as RGBA
/// floats or clamped RGBA8, 32 pixels at a time from GPU memory. `o` has room
/// for `npix` output pixels (16 bytes for FLOAT, else 4). False when the
/// platform failed a read (those pixels come back as zero texels).
#[allow(clippy::too_many_arguments)]
fn read_float_row(
    ops: &osal::OsalOps,
    m: *mut OsalGpuMem,
    cpu: *const u8,
    mut src_off: usize,
    fmt: u8,
    mut npix: usize,
    o: &mut [u8],
    type_: GLenum,
) -> bool {
    let bpp = gles_surf_bytes_per_pixel(fmt) as usize;
    let mut stage = [0u8; 32 * 8];
    let mut oi = 0;
    let mut ok = true;

    while npix > 0 {
        let k = npix.min(32);
        let n = k * bpp;

        if !cpu.is_null() {
            // SAFETY: the caller's span lies inside the target's CPU mapping.
            stage[..n].copy_from_slice(unsafe { core::slice::from_raw_parts(cpu.add(src_off), n) });
        } else if let Some(gpu_read) = ops.gpu_read {
            // SAFETY: `m` is a live descriptor and `stage` has room for n bytes.
            if unsafe { gpu_read(m, src_off, stage.as_mut_ptr() as *mut c_void, n) } != 0 {
                stage[..n].fill(0);
                ok = false;
            }
        } else {
            stage[..n].fill(0);
            ok = false;
        }
        for px in stage[..n].chunks_exact(bpp) {
            let mut c = [0.0f32, 0.0, 0.0, 1.0];
            let half = |i: usize| half_to_float(u16::from_le_bytes([px[i], px[i + 1]]));

            if fmt == GLES_SURF_RGBA16F {
                for (ch, v) in c.iter_mut().enumerate() {
                    *v = half(ch * 2);
                }
            } else if fmt == GLES_SURF_R16F {
                c[0] = half(0);
            } else {
                c[0] = f32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            }
            if type_ == GL_FLOAT {
                for (ch, v) in c.iter().enumerate() {
                    o[oi + ch * 4..oi + ch * 4 + 4].copy_from_slice(&v.to_ne_bytes());
                }
                oi += 16;
            } else {
                for (ch, v) in c.iter().enumerate() {
                    /* NaN stays NaN (as through the C comparisons) and the
                     * cast below gives 0 (C: undefined, 0 on the targets) */
                    o[oi + ch] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                oi += 4;
            }
        }
        src_off += n;
        npix -= k;
    }
    ok
}

/// EGLImage target: the bound renderbuffer takes `mem` as storage without
/// owning it. `mem` must hold `w` x `h` RGBA8888 pixels at the pair-padded
/// row pitch (the PBE's rows); -1 otherwise, the renderbuffer unchanged.
pub fn renderbuffer_attach_external(c: &mut GlContext, mem: &OsalGpuMem, w: u32, h: u32) -> i32 {
    if w == 0 || h == 0 || w > 4096 || h > 4096 || mem.impl_.is_null() || !storage_holds(mem, 0, w, h, RGBA8_BPP) {
        return -1;
    }
    let bound = c.bound_renderbuffer;
    let Some(rb) = c.renderbuffer(bound) else {
        return -1;
    };
    context::release_storage(&mut rb.mem, &mut rb.external);
    rb.mem = *mem;
    rb.external = true;
    rb.internalformat = GL_RGBA;
    rb.width = w;
    rb.height = h;
    rb.storage_gen = context::respecified(rb.serial, false);
    0
}

/// The C name of [`renderbuffer_attach_external`], on the current context.
///
/// `_owner` is the EGLImage, for source compatibility with the C library's
/// signature; this port finds the storage record from `mem` (see
/// `gl_internal.h`), so the argument is not used.
///
/// # Safety
/// `mem` is null or a valid descriptor; `_owner` is not dereferenced.
#[no_mangle]
pub unsafe extern "C" fn gl_renderbuffer_attach_external(
    mem: *const OsalGpuMem,
    w: u32,
    h: u32,
    _owner: *mut c_void,
) -> i32 {
    // SAFETY: a non-null `mem` points at a valid descriptor (the C contract).
    match (context::current(), unsafe { mem.as_ref() }) {
        (Some(c), Some(mem)) => renderbuffer_attach_external(c, mem, w, h),
        _ => -1,
    }
}

/* --------------------------------------------------------------------------- */
/* OpenGL ES 3.0 framebuffer entry points. */

fn log_available() -> bool {
    osal::ops().is_some_and(|o| o.log.is_some())
}

/// Multisampled renderbuffers are single-sampled: GL_MAX_SAMPLES is 1, so a
/// conformant caller never asks for more; one that does gets the plain
/// storage (and no resolve).
#[no_mangle]
pub unsafe extern "C" fn glRenderbufferStorageMultisample(
    target: GLenum,
    samples: GLsizei,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
) {
    gl_trace!(glRenderbufferStorageMultisample);
    const FN: &str = "glRenderbufferStorageMultisample";
    let Some(c) = context::current() else { return };
    /* ES 3.0 4.4.2, in dEQP's order (negative_api.buffer.
     * renderbuffer_storage_multisample): the target and the format are
     * GL_INVALID_ENUM, a negative sample count or a size past the limit
     * GL_INVALID_VALUE, and a count past the format's GL_SAMPLES (1 here:
     * no multisampled storage exists) GL_INVALID_OPERATION. Until
     * 2026-09-24 the count was logged and single-sampled storage handed
     * out, so an application that asked for 4 samples was told it had them. */
    if !renderbuffer_storage_args_ok(c, target, internalformat, width, height, FN) {
        return;
    }
    if samples < 0 {
        c.set_error(GL_INVALID_VALUE, FN);
        return;
    }
    if samples > 1 {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    renderbuffer_storage(c, target, internalformat, width, height);
}

/// glInvalidateFramebuffer on the current context (glInvalidateSubFramebuffer
/// lands here too).
fn invalidate_framebuffer(c: &mut GlContext, target: GLenum, num_attachments: GLsizei, attachments: *const GLenum) {
    const FUNC: &str = "glInvalidateFramebuffer";
    if !is_fb_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if num_attachments < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if attachments.is_null() {
        return;
    }
    /* ES 3.0 4.5.1: the default framebuffer takes GL_COLOR, GL_DEPTH and
     * GL_STENCIL (anything else GL_INVALID_ENUM); a framebuffer object
     * takes the attachment points, and GL_COLOR_ATTACHMENTm with m at or
     * past GL_MAX_COLOR_ATTACHMENTS is GL_INVALID_OPERATION (dEQP
     * negative_api.buffer.invalidate_framebuffer). Nothing was checked
     * until 2026-09-24. */
    for i in 0..num_attachments as usize {
        // SAFETY: the application passes `num_attachments` enums, any alignment.
        let a = unsafe { attachments.add(i).read_unaligned() };
        let err = if c.bound_framebuffer == 0 {
            (!matches!(a, GL_COLOR | GL_DEPTH | GL_STENCIL)).then_some(GL_INVALID_ENUM)
        } else if (GL_COLOR_ATTACHMENT0..GL_COLOR_ATTACHMENT0 + 32).contains(&a) {
            ((a - GL_COLOR_ATTACHMENT0) as usize >= CAP_COLOR_ATTACHMENTS).then_some(GL_INVALID_OPERATION)
        } else {
            (!matches!(a, GL_DEPTH_ATTACHMENT | GL_STENCIL_ATTACHMENT | GL_DEPTH_STENCIL_ATTACHMENT))
                .then_some(GL_INVALID_ENUM)
        };
        if let Some(e) = err {
            c.set_error(e, FUNC);
            return;
        }
    }
}

/// Nothing to discard: depth/stencil live on chip per kick and colour is
/// always kept, so an invalidate is a no-op beyond validation.
#[no_mangle]
pub unsafe extern "C" fn glInvalidateFramebuffer(target: GLenum, numAttachments: GLsizei, attachments: *const GLenum) {
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if target == GL_READ_FRAMEBUFFER {
        if let Some(c) = context::current() {
            if as_read(c, || unsafe { glInvalidateFramebuffer(GL_FRAMEBUFFER, numAttachments, attachments) }).is_some()
            {
                return;
            }
        }
    }
    gl_trace!(glInvalidateFramebuffer);
    let Some(c) = context::current() else { return };
    invalidate_framebuffer(c, target, numAttachments, attachments);
}

#[no_mangle]
pub unsafe extern "C" fn glInvalidateSubFramebuffer(
    target: GLenum,
    numAttachments: GLsizei,
    _attachments: *const GLenum,
    _x: GLint,
    _y: GLint,
    width: GLsizei,
    height: GLsizei,
) {
    gl_trace!(glInvalidateSubFramebuffer);
    if width < 0 || height < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glInvalidateSubFramebuffer");
        return;
    }
    glInvalidateFramebuffer(target, numAttachments, _attachments);
}

/// One colour attachment: the only draw buffer is it (or none).
#[no_mangle]
pub unsafe extern "C" fn glDrawBuffers(n: GLsizei, bufs: *const GLenum) {
    gl_trace!(glDrawBuffers);
    const FUNC: &str = "glDrawBuffers";
    let Some(c) = context::current() else { return };
    /* The errors in the order dEQP-GLES3 negative_api.buffer.draw_buffers
     * checks them (ES 3.0 4.2.1): a negative n; a value that is no draw
     * buffer at all (GL_INVALID_ENUM, even when n is also too large); the
     * window given other than one buffer; n past GL_MAX_DRAW_BUFFERS; then
     * the per-buffer rules. n past the limit was checked first until
     * 2026-09-24, which answered GL_INVALID_VALUE to the first two. */
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* n draw buffers and no array to read them from: ES 3.0 4.2.1 names no
     * error for it, and there is nothing to carry out, so it is refused as a
     * bad value and changes nothing (review R-22). Accepting it turned a
     * caller's bug into "every output GL_NONE" with no error. */
    if n > 0 && bufs.is_null() {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* the application's array, read one enum at a time (it need not be
     * aligned for a GLenum) */
    let buf = |i: GLsizei| -> GLenum {
        // SAFETY: the application passes `n` enums; i < n.
        unsafe { bufs.add(i as usize).read_unaligned() }
    };
    let accepted = |b: GLenum| {
        b == GL_NONE
            || b == GL_BACK
            || (GL_COLOR_ATTACHMENT0..GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum).contains(&b)
    };
    /* the enum scan reads at most a sane count: n far past the limit is
     * GL_INVALID_VALUE below whatever the array holds */
    if n <= 32 && (0..n).any(|i| !accepted(buf(i))) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if c.bound_framebuffer == 0 {
        /* ES 3.0 4.2.1: "If the GL is bound to the default framebuffer, then
         * n must be 1 and the constant must be BACK or NONE." The choice is
         * state: GL_NONE writes no colour (it was accepted and ignored). */
        if n != 1 || (buf(0) != GL_NONE && buf(0) != GL_BACK) {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
        // SAFETY: single-threaded by contract; no other reference is live.
        unsafe { DEFAULT_DRAW_NONE.get()[c.index as usize % MAX_CONTEXTS] = buf(0) == GL_NONE };
        return;
    }
    if n > context::max_draw_buffers() as GLsizei {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if c.framebuffer_ref(c.bound_framebuffer).is_none() {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    for i in 0..n {
        /* ES 3.0: buffer i names GL_COLOR_ATTACHMENTi or GL_NONE */
        if buf(i) != GL_NONE && buf(i) != GL_COLOR_ATTACHMENT0 + i as GLenum {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    }
    let bound = c.bound_framebuffer;
    let Some(fb) = c.framebuffer(bound) else { return };

    /* "The draw buffer for fragment colors beyond n is set to NONE", and
     * n == 0 "is equivalent to setting all the draw buffers to NONE" (ES 3.0
     * 4.2.1). `num_draw_buf` 0 means glDrawBuffers was never called (output
     * i to attachment i), so the count kept is the whole table: n was kept
     * until 2026-09-29, and glDrawBuffers(0, ...) then read as "never
     * called" and every attachment was written. */
    fb.draw_buf = [-1; CAP_COLOR_ATTACHMENTS];
    for i in 0..n {
        fb.draw_buf[i as usize] = if buf(i) == GL_NONE { -1 } else { i as i8 };
    }
    fb.num_draw_buf = CAP_COLOR_ATTACHMENTS as u8;
}

#[no_mangle]
pub unsafe extern "C" fn glReadBuffer(src: GLenum) {
    /* the READ framebuffer when it is not the draw one (ES 3.0 4.4.1) */
    if let Some(c) = context::current() {
        if as_read(c, || unsafe { glReadBuffer(src) }).is_some() {
            return;
        }
    }
    gl_trace!(glReadBuffer);
    const FN: &str = "glReadBuffer";
    let Some(c) = context::current() else { return };
    /* ES 3.0 4.3.1: GL_BACK, GL_NONE and GL_COLOR_ATTACHMENT0..31 are the
     * names (anything else GL_INVALID_ENUM); the window takes GL_BACK, a
     * framebuffer object an attachment below GL_MAX_COLOR_ATTACHMENTS
     * (anything else GL_INVALID_OPERATION). An attachment past 0 was refused
     * until 2026-09-25, so an MRT read-back always read attachment 0 (gl_test
     * 106 mrt3 on the AM62P). */
    let attachment = (GL_COLOR_ATTACHMENT0..GL_COLOR_ATTACHMENT0 + 32).contains(&src);
    if src != GL_NONE && src != GL_BACK && !attachment {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    let bound = c.bound_framebuffer;
    let ok = if bound == 0 {
        src == GL_BACK || src == GL_NONE
    } else {
        src == GL_NONE || (attachment && src < GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum)
    };
    if !ok {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    /* recorded per framebuffer, read back by GL_READ_BUFFER */
    let none = src == GL_NONE;
    match c.framebuffer(bound) {
        Some(fb) => {
            fb.read_none = none;
            fb.read_index = if attachment { (src - GL_COLOR_ATTACHMENT0) as u8 } else { 0 };
        }
        None => c.default_read_none = none,
    }
}

/// Run `f` (an entry point that reads the framebuffer, called again) with the
/// read framebuffer's glReadBuffer attachment in the place of attachment 0,
/// where the read path looks; None when that is attachment 0 already.
pub fn as_read_attachment<R>(c: &mut GlContext, f: impl FnOnce() -> R) -> Option<R> {
    let bound = c.bound_framebuffer;
    let n = c.framebuffer_ref(bound).map_or(0, |fb| fb.read_index as usize);
    if n == 0 || n >= CAP_COLOR_ATTACHMENTS {
        return None;
    }
    super::draw::flush_pending_clear(c);
    let swap = |c: &mut GlContext| {
        if let Some(fb) = c.framebuffer(bound) {
            core::mem::swap(&mut fb.color, &mut fb.color_n[n - 1]);
            fb.read_index = if fb.read_index == 0 { n as u8 } else { 0 };
        }
    };
    swap(c);
    let r = f();
    if let Some(c) = context::current() {
        swap(c);
    }
    Some(r)
}

/// No multisample resolve is ever needed (single-sampled FBOs) and no other
/// blit is issued by Qt's RHI; a caller that blits anyway is told so.
#[no_mangle]
pub unsafe extern "C" fn glBlitFramebuffer(
    _srcX0: GLint,
    _srcY0: GLint,
    _srcX1: GLint,
    _srcY1: GLint,
    _dstX0: GLint,
    _dstY0: GLint,
    _dstX1: GLint,
    _dstY1: GLint,
    _mask: GLbitfield,
    _filter: GLenum,
) {
    gl_trace!(glBlitFramebuffer);
    {
        // SAFETY: single-threaded by contract; no other reference is live.
        let warned = unsafe { WARNED.get() };
        if !warned.blit && log_available() {
            warned.blit = true;
            glog!("glBlitFramebuffer: not implemented (GL_INVALID_OPERATION)");
        }
    }
    context::set_error_no_ctx(GL_INVALID_OPERATION, "glBlitFramebuffer");
}

#[no_mangle]
pub unsafe extern "C" fn glGetInternalformativ(
    target: GLenum,
    internalformat: GLenum,
    pname: GLenum,
    bufSize: GLsizei,
    params: *mut GLint,
) {
    gl_trace!(glGetInternalformativ);
    const FN: &str = "glGetInternalformativ";
    /* ES 3.0 6.1.15: the target is GL_RENDERBUFFER, the format renderable,
     * the pname one of the two, and bufSize not negative (dEQP
     * negative_api.state.get_internalformativ). One sample count, 1. */
    /* ES 3.0 table 3.13's integer colour formats and RGB10_A2 are
     * renderable formats the query must know (dEQP-GLES3 state_query.
     * internal_format.*_samples, 20 cases), though no renderbuffer of them
     * can be made here yet. An integer format has no multisample counts
     * (6.1.15: NUM_SAMPLE_COUNTS may be 0 for one). */
    let integer = matches!(
        internalformat,
        GL_R8I
            | GL_R8UI
            | GL_R16I
            | GL_R16UI
            | GL_R32I
            | GL_R32UI
            | GL_RG8I
            | GL_RG8UI
            | GL_RG16I
            | GL_RG16UI
            | GL_RG32I
            | GL_RG32UI
            | GL_RGBA8I
            | GL_RGBA8UI
            | GL_RGBA16I
            | GL_RGBA16UI
            | GL_RGBA32I
            | GL_RGBA32UI
            | GL_RGB10_A2UI
    );
    if target != GL_RENDERBUFFER || !(is_renderable_format(internalformat) || integer || internalformat == GL_RGB10_A2)
    {
        context::set_error_no_ctx(GL_INVALID_ENUM, FN);
        return;
    }
    if pname != GL_NUM_SAMPLE_COUNTS && pname != GL_SAMPLES {
        context::set_error_no_ctx(GL_INVALID_ENUM, FN);
        return;
    }
    if bufSize < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FN);
        return;
    }
    if bufSize < 1 || params.is_null() {
        return;
    }
    let counts = if integer { 0 } else { 1 };
    if pname == GL_SAMPLES && counts == 0 {
        return; /* no sample counts to list */
    }
    // SAFETY: `params` is non-null with room for bufSize >= 1 values (not
    // necessarily aligned).
    unsafe { params.write_unaligned(if pname == GL_NUM_SAMPLE_COUNTS { counts } else { 1 }) };
}

/// Unit tests: re-arm the multisample / blit warnings (process-global;
/// testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised.
    unsafe {
        *WARNED.get() = Warned { blit: false };
        *TARGET_USE.get() = [[0; MAX_TEXTURES]; MAX_CONTEXTS];
        *DEPTH_TARGET.get() = [[DepthTarget { level: 0, resolved: 0 }; MAX_TEXTURES]; MAX_CONTEXTS];
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::state::{glClear, glClearColor, glFlush, glGetError, glPixelStorei};
    use crate::gl::texture::{
        glBindTexture, glCopyTexSubImage2D, glDeleteTextures, glGenTextures, glIsTexture, glTexImage2D,
    };
    use crate::testutil::{gpu_render, CacheOp, MemEvent, TestGl, HEAP_DATA};
    use std::vec::Vec;

    /* ---- helpers ------------------------------------------------------------ */

    fn gen_fb() -> GLuint {
        let mut n: GLuint = 0;
        // SAFETY: room for one name.
        unsafe { glGenFramebuffers(1, &mut n) };
        n
    }

    fn gen_rb() -> GLuint {
        let mut n: GLuint = 0;
        // SAFETY: room for one name.
        unsafe { glGenRenderbuffers(1, &mut n) };
        n
    }

    /// A new framebuffer, bound.
    fn bound_fb() -> GLuint {
        let fb = gen_fb();
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        fb
    }

    /// A new renderbuffer, bound, with storage.
    fn rb_with_storage(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        }
        rb
    }

    /// A name only: generated, never bound, so no texture object yet.
    fn gen_tex() -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: room for one name.
        unsafe { glGenTextures(1, &mut t) };
        t
    }

    fn attachment_param(attachment: GLenum, pname: GLenum) -> GLint {
        let mut v: GLint = -1;
        // SAFETY: room for one value.
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, pname, &mut v) };
        v
    }

    /// A new 2D texture, bound, with (uninitialised) RGBA storage.
    fn tex_with_storage(w: GLsizei, h: GLsizei) -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        t
    }

    /// A 24-bit depth texture's internal format: GL_DEPTH_COMPONENT24 is
    /// ES 3.0's; the ES 2.0 surface (GL_OES_depth_texture) names it unsized.
    const DEPTH24_TEX: GLenum = if crate::REPORT_ES2 { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 };

    /// A new 2D depth texture (`GL_DEPTH_COMPONENT`, `GL_UNSIGNED_SHORT`) or
    /// depth-stencil texture (`GL_DEPTH_STENCIL`, `GL_UNSIGNED_INT_24_8`),
    /// bound, with storage: the formats dEQP-GLES2 fbo.completeness makes
    /// through GL_OES_depth_texture and GL_OES_packed_depth_stencil.
    fn depth_tex(format: GLenum, w: GLsizei, h: GLsizei) -> GLuint {
        let type_ = if format == GL_DEPTH_STENCIL { GL_UNSIGNED_INT_24_8 } else { GL_UNSIGNED_SHORT };
        let mut t: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, w, h, 0, format, type_, core::ptr::null());
        }
        t
    }

    fn attach_tex(attachment: GLenum, tex: GLuint) {
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D, tex, 0) };
    }

    fn attach_rb(attachment: GLenum, rb: GLuint) {
        // SAFETY: plain GL call.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb) };
    }

    fn status() -> GLenum {
        // SAFETY: plain GL call.
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
    }

    /// glGetFramebufferAttachmentParameteriv; -1 when nothing was written.
    fn fb_param(attachment: GLenum, pname: GLenum) -> GLint {
        let mut v: GLint = -1;
        // SAFETY: `v` is a writable GLint.
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, attachment, pname, &mut v) };
        v
    }

    /// glGetRenderbufferParameteriv; -1 when nothing was written.
    fn rb_param(pname: GLenum) -> GLint {
        let mut v: GLint = -1;
        // SAFETY: `v` is a writable GLint.
        unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
        v
    }

    /// The test pattern's RGBA texel at column `x` of stored row `row`.
    fn px(x: usize, row: usize) -> [u8; 4] {
        [x as u8 + 1, row as u8 + 0x10, 0xAB, 0xCD]
    }

    /// The GPU renders a linear RGBA8888 store of `h` rows at `pitch` texels
    /// per row: [`px`], the padding texels beyond `w` get 0xEE. The CPU view
    /// sees it only after glReadPixels invalidates it.
    fn fill_rgba(mem: &OsalGpuMem, pitch: usize, w: usize, h: usize) {
        assert!(mem.size >= pitch * h * 4);
        let mut bytes = std::vec![0u8; pitch * h * 4];
        for row in 0..h {
            for x in 0..pitch {
                let v = if x < w { px(x, row) } else { [0xEE; 4] };
                bytes[(row * pitch + x) * 4..][..4].copy_from_slice(&v);
            }
        }
        gpu_render(mem.gpu, &bytes);
    }

    const SENTINEL: u8 = 0x77;

    /// glReadPixels(RGBA, UNSIGNED_BYTE) into a buffer pre-filled with SENTINEL.
    fn read(x: GLint, y: GLint, w: GLsizei, h: GLsizei) -> Vec<u8> {
        let mut out = std::vec![SENTINEL; w as usize * h as usize * 4];
        // SAFETY: `out` holds h rows of w RGBA pixels.
        unsafe { glReadPixels(x, y, w, h, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
        out
    }

    fn pixel(buf: &[u8], w: usize, x: usize, j: usize) -> [u8; 4] {
        let o = (j * w + x) * 4;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    /// A second context whose default surface is caller-mapped memory of a
    /// given `GLES_SURF_*` format; current while it lives. Dropping it destroys
    /// the context, frees the surface and makes the TestGl context current again.
    struct SurfaceCtx<'a> {
        gl: &'a TestGl,
        ctx: *mut GlContext,
        mem: OsalGpuMem,
    }

    impl<'a> SurfaceCtx<'a> {
        fn new(gl: &'a TestGl, w: u32, h: u32, format: u8, bytes: usize) -> Self {
            let mut mem = OsalGpuMem::ZERO;
            assert_eq!(osal::gpu_alloc(HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem), 0);
            // SAFETY: `mem` is a live descriptor; the pool has a free slot.
            let ctx = unsafe { context::gl_context_create_for_surface(&mut mem, w, h, format, 1, context::GROUP_NONE) };
            assert!(!ctx.is_null());
            SurfaceCtx { gl, ctx, mem }
        }

        /// The GPU renders `bytes` at `off` of the surface.
        fn render(&self, off: usize, bytes: &[u8]) {
            gpu_render(self.mem.gpu + off as u64, bytes);
        }
    }

    impl Drop for SurfaceCtx<'_> {
        fn drop(&mut self) {
            // SAFETY: the context is ours and no reference to it is live.
            unsafe {
                context::gl_context_destroy(self.ctx);
                context::gl_context_set_current(self.gl.ctx());
            }
            osal::gpu_free(&mut self.mem);
        }
    }

    /* ---- pure helpers ------------------------------------------------------- */

    #[test]
    fn attachment_points_accepted_are_the_three_colours_depth_stencil_and_depth_stencil() {
        /* upstream e471a73: a G-buffer pass writes attachments 0..2 */
        for a in [
            GL_COLOR_ATTACHMENT0,
            GL_COLOR_ATTACHMENT1,
            GL_COLOR_ATTACHMENT2,
            GL_DEPTH_ATTACHMENT,
            GL_STENCIL_ATTACHMENT,
            GL_DEPTH_STENCIL_ATTACHMENT,
        ] {
            assert!(is_attachment_point(a), "0x{a:x}");
        }
        for a in [GL_COLOR_ATTACHMENT3, GL_NONE, GL_TEXTURE, GL_FRAMEBUFFER, GL_DEPTH_STENCIL] {
            assert!(!is_attachment_point(a), "0x{a:x}");
        }
    }

    #[test]
    fn framebuffer_targets_are_the_three_es3_binding_points() {
        assert!(is_fb_target(GL_FRAMEBUFFER) && is_fb_target(GL_DRAW_FRAMEBUFFER) && is_fb_target(GL_READ_FRAMEBUFFER));
        assert!(!is_fb_target(GL_RENDERBUFFER) && !is_fb_target(0) && !is_fb_target(GL_TEXTURE_2D));
    }

    #[test]
    fn set_attachment_depth_stencil_sets_both_points_and_name_zero_clears() {
        let mut fb = FRAMEBUFFER_ZERO;
        set_attachment(&mut fb, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 3, 0, 0);
        assert_eq!((fb.depth.type_, fb.depth.name), (GL_RENDERBUFFER, 3));
        assert_eq!((fb.stencil.type_, fb.stencil.name), (GL_RENDERBUFFER, 3));
        assert_eq!(fb.color.type_, GL_NONE);

        set_attachment(&mut fb, GL_COLOR_ATTACHMENT0, GL_TEXTURE, 7, 0, 0);
        assert_eq!((fb.color.type_, fb.color.name, fb.color.level), (GL_TEXTURE, 7, 0));

        /* name 0 detaches whatever type is passed */
        set_attachment(&mut fb, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0, 0, 0);
        assert_eq!((fb.depth.type_, fb.depth.name), (GL_NONE, 0));
        assert_eq!((fb.stencil.type_, fb.stencil.name), (GL_NONE, 0));
        assert_eq!(fb.color.name, 7, "the colour point is untouched");

        /* an unknown point changes nothing */
        set_attachment(&mut fb, GL_COLOR_ATTACHMENT1, GL_TEXTURE, 9, 0, 0);
        assert_eq!(fb.color.name, 7);
    }

    #[test]
    fn format_classes_colour_depth_stencil() {
        for f in [GL_RGBA4, GL_RGB5_A1, GL_RGB565, GL_RGBA8, GL_RGB8, GL_SRGB8_ALPHA8, GL_R8, GL_RG8, GL_RGBA16F] {
            assert!(is_color_format(f) && !is_depth_format(f) && !is_stencil_format(f), "0x{f:x}");
        }
        for f in [GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT32F] {
            assert!(is_depth_format(f) && !is_color_format(f) && !is_stencil_format(f), "0x{f:x}");
        }
        assert!(is_stencil_format(GL_STENCIL_INDEX8) && !is_depth_format(GL_STENCIL_INDEX8));
        /* the packed formats are both */
        for f in [GL_DEPTH24_STENCIL8, GL_DEPTH32F_STENCIL8] {
            assert!(is_depth_format(f) && is_stencil_format(f) && !is_color_format(f), "0x{f:x}");
        }
        /* unsized / unrelated enums are none of them */
        for f in [GL_DEPTH_COMPONENT, GL_DEPTH_STENCIL, GL_LUMINANCE, 0] {
            assert!(!is_color_format(f) && !is_depth_format(f) && !is_stencil_format(f), "0x{f:x}");
        }
    }

    /* ---- framebuffer objects: gen / bind / delete / is ------------------------ */

    #[test]
    fn gen_framebuffers_returns_distinct_nonzero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [0 as GLuint; 3];
        // SAFETY: room for three names.
        unsafe { glGenFramebuffers(3, names.as_mut_ptr()) };
        assert!(names.iter().all(|&n| n != 0));
        assert!(names[0] != names[1] && names[1] != names[2] && names[0] != names[2]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn gen_framebuffers_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let mut n: GLuint = 99;
        // SAFETY: a negative count writes nothing.
        unsafe { glGenFramebuffers(-1, &mut n) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(n, 99);
    }

    #[test]
    fn gen_framebuffers_null_and_zero_count_are_no_ops() {
        let gl = TestGl::new(8, 8);
        let mut n: GLuint = 99;
        // SAFETY: null is tolerated; a zero count writes nothing.
        unsafe {
            glGenFramebuffers(2, core::ptr::null_mut());
            glGenFramebuffers(0, &mut n);
        }
        assert_eq!(n, 99);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().framebuffers.iter().all(|f| !f.used));
    }

    #[test]
    fn gen_framebuffers_past_the_table_returns_zero_names() {
        /* the table holds MAX_FRAMEBUFFERS - 1 objects (name 0 is the window);
         * names that do not fit come back as 0 */
        let gl = TestGl::new(8, 8);
        let mut names = [77 as GLuint; MAX_FRAMEBUFFERS + 3];
        // SAFETY: room for every name asked for.
        unsafe { glGenFramebuffers(names.len() as GLsizei, names.as_mut_ptr()) };
        assert!(names[..MAX_FRAMEBUFFERS - 1].iter().all(|&n| n != 0 && (n as usize) < MAX_FRAMEBUFFERS));
        assert!(names[MAX_FRAMEBUFFERS - 1..].iter().all(|&n| n == 0));
        /* an exhausted pool says so: a client that took 0 for a name would
         * bind the default framebuffer instead (upstream 7219228) */
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn deleted_framebuffer_name_is_reused_and_comes_back_empty() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: one valid name.
        unsafe { glDeleteFramebuffers(1, &fb) };
        assert_eq!(gen_fb(), fb, "the lowest free name is handed out again");
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attaching_a_generated_but_unbound_name_is_invalid_operation() {
        /* dEQP fbo.api.attach_names. A name glGen* handed out is not an
         * object until it has been bound (ES 2.0 2.1), and attaching one is
         * GL_INVALID_OPERATION (4.4.3), not a silent attachment of nothing. */
        let gl = TestGl::new(8, 8);
        let _fb = bound_fb();
        let tex = gen_tex();
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsTexture(tex), GL_FALSE_B);
            assert_eq!(glIsRenderbuffer(rb), GL_FALSE_B);

            attach_tex(GL_COLOR_ATTACHMENT0, tex);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_NEGATIVE_X, tex, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            attach_rb(GL_COLOR_ATTACHMENT0, rb);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        // nothing was attached
        let fb = gl.ctx().bound_framebuffer;
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
    }

    #[test]
    fn deleting_an_object_leaves_a_not_bound_framebuffer_attached() {
        /* dEQP fbo.api.delete_{tex_2d,tex_cube,rbo}_attached_to_not_bound_fbo.
         * ES 2.0 3.7.13 / 4.4.3: the attachment is broken in the framebuffer
         * that is bound, and in no other. The query still answers with the
         * deleted name, and fbo.render.recreate_*.no_rebind_* renders through
         * the surviving attachment after re-creating the object. */
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);

        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteTextures(1, &tex);
            glDeleteRenderbuffers(1, &rb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_TEXTURE as GLint);
        assert_eq!(attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), tex as GLint);
        assert_eq!(
            attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
            GL_RENDERBUFFER as GLint
        );
        assert_eq!(attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), rb as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- the backend learns when depth/stencil contents are gone ---- */

    /// The id the backend keeps renderbuffer `rb`'s depth/stencil under.
    fn rb_ds_id(gl: &TestGl, rb: GLuint) -> u32 {
        context::ds_object_id(gl.ctx().objs().renderbuffers[rb as usize].serial, false)
    }

    /// A depth renderbuffer's contents are gone when it is freed -- deleted
    /// while nothing else holds it -- and not before: held by a framebuffer
    /// that is not bound it is an orphan whose image the framebuffer still
    /// uses (ES 2.0 4.4.3), and the backend hears of it only when the last
    /// framebuffer lets go. `backend::attachment_gone` names it once.
    #[test]
    fn a_depth_renderbuffer_is_gone_for_the_backend_when_it_is_freed() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        let id = rb_ds_id(&gl, depth);
        gl.gones();
        // SAFETY: plain GL calls with one readable name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteRenderbuffers(1, &depth);
        }
        assert_eq!(gl.gones(), Vec::<u32>::new(), "an orphan the unbound framebuffer still renders with");
        // SAFETY: as above.
        unsafe { glDeleteFramebuffers(1, &fb) };
        assert_eq!(gl.gones(), [id], "the last holder let go: the contents are gone");
        let lone = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        let lone_id = rb_ds_id(&gl, lone);
        gl.gones();
        // SAFETY: as above.
        unsafe { glDeleteRenderbuffers(1, &lone) };
        assert_eq!(gl.gones(), [lone_id], "deleted and held by nothing: gone at once");
    }

    /// Specifying a renderbuffer's storage again undefines its contents: the
    /// backend is told before the new storage is used, under the object's
    /// id (its serial, which the orphan rule and a respecification keep).
    #[test]
    fn respecifying_a_renderbuffer_tells_the_backend_its_contents_are_gone() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_DEPTH24_STENCIL8_OES, 4, 4);
        let id = rb_ds_id(&gl, rb);
        gl.gones();
        // SAFETY: plain GL call on the bound renderbuffer.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, 8, 8) };
        assert_eq!(gl.gones(), [id]);
        assert_eq!(rb_ds_id(&gl, rb), id, "the same object, new storage");
    }

    /// A depth texture's contents are gone for the backend when the texture
    /// is freed, under the id with the texture bit (`GlesDraw::depth_id`).
    #[test]
    fn a_deleted_texture_is_gone_for_the_backend_with_the_texture_bit() {
        let gl = TestGl::new(8, 8);
        let tex = tex_with_storage(4, 4);
        let id = context::ds_object_id(gl.ctx().objs().textures[tex as usize].serial, true);
        assert_ne!(id & 0x4000_0000, 0);
        gl.gones();
        // SAFETY: one readable name.
        unsafe { glDeleteTextures(1, &tex) };
        assert_eq!(gl.gones(), [id]);
    }

    /* ---- the orphan rule: dEQP-GLES2 lifetime.attach.deleted_{input,output}.* ---- */

    /// The storage the bound framebuffer renders into.
    fn target_gpu(gl: &TestGl) -> u64 {
        fb_target(gl.ctx()).expect("a complete framebuffer").0.gpu
    }

    /// dEQP lifetime.attach.deleted_input.renderbuffer_framebuffer and
    /// deleted_output.renderbuffer_framebuffer, as dEQP drives them: a
    /// renderbuffer attached to a framebuffer that is then unbound, deleted,
    /// a new renderbuffer generated and given storage. The framebuffer must
    /// keep rendering into (input) and reading from (output) the deleted
    /// renderbuffer's image, not the new object's: the new object gets a
    /// name of its own and storage of its own. All four cases failed on the
    /// i.MX8MP (2026-09-24): the delete freed the slot, glGen* handed the
    /// same name out again and the framebuffer read the new object.
    #[test]
    fn a_deleted_renderbuffer_held_by_an_unbound_framebuffer_keeps_its_image() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let old_gpu = target_gpu(&gl);
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
        }
        assert_eq!(unsafe { glIsRenderbuffer(rb) }, GL_FALSE_B, "the name is gone");
        let new_rb = rb_with_storage(GL_RGBA4, 4, 4);
        assert_ne!(new_rb, rb, "the deleted name is not handed out again while a framebuffer holds it");
        assert_eq!(unsafe { glIsRenderbuffer(new_rb) }, GL_TRUE_B);
        let new_gpu = gl.ctx().objs().renderbuffers[new_rb as usize].mem.gpu;
        assert_ne!(new_gpu, old_gpu, "the new object has storage of its own");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        assert_eq!(attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), rb as GLint);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the deleted object's image is still there");
        assert_eq!(target_gpu(&gl), old_gpu, "the framebuffer still renders into the deleted object's image");
        /* attaching the deleted name is refused, as any unknown name */
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        /* letting go of it frees the slot: the next glGen* may reuse the name */
        attach_rb(GL_COLOR_ATTACHMENT0, new_rb);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!gl.ctx().objs().renderbuffers[rb as usize].used, "the orphan is freed with its last attachment");
        assert_eq!(target_gpu(&gl), new_gpu);
    }

    /// The texture halves of the two dEQP cases above (deleted_input /
    /// deleted_output.texture_framebuffer), and the delete of the
    /// framebuffer as the other way an orphan is let go of.
    #[test]
    fn a_deleted_texture_held_by_an_unbound_framebuffer_keeps_its_image() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let old_gpu = target_gpu(&gl);
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteTextures(1, &tex);
        }
        assert_eq!(unsafe { glIsTexture(tex) }, GL_FALSE_B, "the name is gone");
        let new_tex = tex_with_storage(4, 4);
        assert_ne!(new_tex, tex, "the deleted name is not handed out again while a framebuffer holds it");
        let new_gpu = gl.ctx().objs().textures[new_tex as usize].mem.gpu;
        assert_ne!(new_gpu, old_gpu, "the new object has storage of its own");
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
        assert_eq!(attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), tex as GLint);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the deleted object's image is still there");
        assert_eq!(target_gpu(&gl), old_gpu, "the framebuffer still renders into the deleted object's image");
        attach_tex(GL_DEPTH_ATTACHMENT, tex);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the deleted name cannot be attached again");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!gl.ctx().objs().textures[tex as usize].used, "the orphan is freed with the framebuffer that held it");
    }

    /// dEQP-GLES2 lifetime.attach.deleted_{input,output}.texture_framebuffer
    /// as dEQP drives them on ES 2.0, where binding needs no glGen*: the
    /// deleted texture's NAME is reused for the new object
    /// (glsLifetimeTests.cpp replaceName). The framebuffer, unbound at the
    /// delete, must keep showing (input) and writing (output) the deleted
    /// image, never the new object's. The orphan moves to a free slot and
    /// the attachment follows it; detaching it frees that slot.
    #[test]
    fn a_deleted_texture_whose_name_is_bound_again_stays_the_framebuffers_image() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let old_gpu = target_gpu(&gl);
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteTextures(1, &tex);
            glBindTexture(GL_TEXTURE_2D, tex); /* a new texture under the old name */
            assert_eq!(glIsTexture(tex), GL_TRUE_B);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let new_gpu = gl.ctx().objs().textures[tex as usize].mem.gpu;
        assert_ne!(new_gpu, old_gpu, "a new texture, new storage");
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(target_gpu(&gl), old_gpu, "the framebuffer shows and writes the deleted image, not the new one");
        let held = attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME) as usize;
        assert_ne!(held, tex as usize, "the orphan moved out of the name's slot");
        assert!(gl.ctx().objs().textures[held].orphan && gl.ctx().objs().textures[held].used);
        /* dEQP's detach: the last attachment lets go, the slot is freed */
        attach_tex(GL_COLOR_ATTACHMENT0, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!gl.ctx().objs().textures[held].used);
        assert!(gl.ctx().objs().textures[tex as usize].used, "the new object is untouched");
    }

    /// The renderbuffer halves (lifetime.attach.deleted_{input,output}.
    /// renderbuffer_framebuffer), and dEQP-GLES2 fbo.render.
    /// recreate_colorbuffer.no_rebind_rbo_rgba4: delete, bind the same name,
    /// give it storage, bind the framebuffer without attaching again. The
    /// framebuffer stays complete on the kept image; dEQP clears the
    /// recreated buffer before it draws, so its picture is the reference's.
    /// 62fc61fa detached the name here and the 28 recreate_colorbuffer
    /// no_rebind cases failed on the i.MX8MP (7c12d1bd).
    #[test]
    fn a_deleted_colour_renderbuffer_whose_name_is_bound_again_stays_the_framebuffers_image() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        let old_gpu = target_gpu(&gl);
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteRenderbuffers(1, &rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            assert_eq!(glIsRenderbuffer(rb), GL_TRUE_B);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 4, 4);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let new_gpu = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        assert_ne!(new_gpu, old_gpu, "a new renderbuffer, new storage");
        assert_eq!(target_gpu(&gl), old_gpu, "the framebuffer writes the deleted image, not the new one");
        assert_eq!(
            attachment_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
            GL_RENDERBUFFER as GLint
        );
        // SAFETY: plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// fbo.render.recreate_colorbuffer.no_rebind_tex2d_rgba_depth_component16
    /// and recreate_depthbuffer.no_rebind_*: a texture colour and a depth
    /// renderbuffer both recreated under their names; the framebuffer keeps
    /// both images, complete. (The depth and stencil no_rebind cases passed
    /// on the board even detached: a missing depth or stencil buffer passes
    /// their tests like the freshly cleared one would.)
    #[test]
    fn recreating_colour_and_depth_under_their_names_keeps_the_framebuffer_complete() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let old_gpu = target_gpu(&gl);
        // SAFETY: plain GL calls, one valid name each.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteTextures(1, &tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glDeleteRenderbuffers(1, &depth);
            glBindRenderbuffer(GL_RENDERBUFFER, depth);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 4, 4);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "both kept images");
        assert_eq!(target_gpu(&gl), old_gpu);
        assert_eq!(
            attachment_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
            GL_RENDERBUFFER as GLint
        );
        let held = attachment_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME) as usize;
        assert!(gl.ctx().objs().renderbuffers[held].orphan, "the deleted depth buffer, moved");
        assert!(!gl.ctx().objs().renderbuffers[depth as usize].orphan, "the new object under the name");
        // SAFETY: plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* deleting the framebuffer lets both orphans go */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
        }
        assert!(!gl.ctx().objs().renderbuffers[held].used);
    }

    #[test]
    fn is_framebuffer_false_until_first_bound() {
        /* GL ES 2.0 4.4.1 / glIsFramebuffer: a name returned by
         * glGenFramebuffers but not yet bound is not a framebuffer object. */
        let _gl = TestGl::new(8, 8);
        let fb = gen_fb();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsFramebuffer(fb), GL_FALSE_B);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            assert_eq!(glIsFramebuffer(fb), GL_TRUE_B);
        }
    }

    #[test]
    fn is_framebuffer_for_bound_deleted_zero_and_out_of_range_names() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsFramebuffer(fb), GL_TRUE_B);
            assert_eq!(glIsFramebuffer(0), GL_FALSE_B);
            assert_eq!(glIsFramebuffer(fb + 1), GL_FALSE_B);
            assert_eq!(glIsFramebuffer(MAX_FRAMEBUFFERS as GLuint), GL_FALSE_B);
            assert_eq!(glIsFramebuffer(GLuint::MAX), GL_FALSE_B);
            glDeleteFramebuffers(1, &fb);
            assert_eq!(glIsFramebuffer(fb), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_framebuffer_records_the_binding_for_every_target() {
        let gl = TestGl::new(8, 8);
        let a = gen_fb();
        let b = gen_fb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, a);
            assert_eq!((gl.ctx().bound_framebuffer, read_framebuffer(gl.ctx())), (a, a));
            /* ES 3.0 4.4.1: the draw and read bindings are separate (they
             * were one binding until 2026-09-24) */
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, b);
            assert_eq!((gl.ctx().bound_framebuffer, read_framebuffer(gl.ctx())), (b, a));
            glBindFramebuffer(GL_READ_FRAMEBUFFER, b);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, a);
            assert_eq!((gl.ctx().bound_framebuffer, read_framebuffer(gl.ctx())), (a, b));
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!((gl.ctx().bound_framebuffer, read_framebuffer(gl.ctx())), (0, 0));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_framebuffer_bad_target_is_invalid_enum_and_keeps_the_binding() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_RENDERBUFFER, 0) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(gl.ctx().bound_framebuffer, fb);
    }

    #[test]
    fn bind_framebuffer_creates_a_name_not_from_gen() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsFramebuffer(9), GL_FALSE_B);
            glBindFramebuffer(GL_FRAMEBUFFER, 9);
            assert_eq!(glIsFramebuffer(9), GL_TRUE_B);
        }
        assert_eq!(gl.ctx().bound_framebuffer, 9);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_framebuffer_name_beyond_the_table_is_a_far_name() {
        /* ES 2.0 binds names nobody generated (gl/names.rs) */
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, MAX_FRAMEBUFFERS as GLuint);
            assert_eq!(glIsFramebuffer(MAX_FRAMEBUFFERS as GLuint), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().bound_framebuffer, (MAX_FRAMEBUFFERS - 1) as GLuint);
    }

    #[test]
    fn delete_bound_framebuffer_rebinds_the_default() {
        let gl = TestGl::new(8, 8);
        let keep = gen_fb();
        let fb = bound_fb();
        // SAFETY: two readable names.
        unsafe { glDeleteFramebuffers(2, [fb, 0].as_ptr()) };
        assert_eq!(gl.ctx().bound_framebuffer, 0);
        assert!(gl.ctx().framebuffers[keep as usize].used);
        assert_eq!(gl.error(), GL_NO_ERROR, "name 0 and unused names are silently ignored");
    }

    #[test]
    fn delete_unbound_framebuffer_keeps_the_binding() {
        let gl = TestGl::new(8, 8);
        let other = gen_fb();
        let fb = bound_fb();
        // SAFETY: one readable name.
        unsafe { glDeleteFramebuffers(1, &other) };
        assert_eq!(gl.ctx().bound_framebuffer, fb);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_framebuffers_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        // SAFETY: a negative count reads nothing.
        unsafe { glDeleteFramebuffers(-1, &fb) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert!(gl.ctx().framebuffers[fb as usize].used);
    }

    #[test]
    fn delete_framebuffers_ignores_stale_and_out_of_range_names() {
        let gl = TestGl::new(8, 8);
        let names = [5 as GLuint, MAX_FRAMEBUFFERS as GLuint, GLuint::MAX];
        // SAFETY: three readable names; null is tolerated.
        unsafe {
            glDeleteFramebuffers(3, names.as_ptr());
            glDeleteFramebuffers(1, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- attachments ---------------------------------------------------------- */

    #[test]
    fn framebuffer_texture_2d_records_the_attachment() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let a = gl.ctx().framebuffers[fb as usize].color;
        assert_eq!((a.type_, a.name, a.level), (GL_TEXTURE, tex, 0));
    }

    #[test]
    fn framebuffer_texture_2d_on_the_default_framebuffer_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn framebuffer_texture_2d_invalid_enums() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_RENDERBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_TEXTURE_2D, tex, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "attachment past the cap");
            /* the cube map target itself is not a face */
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP, tex, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "textarget");
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D_ARRAY, tex, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "textarget");
        }
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
    }

    #[test]
    fn framebuffer_texture_2d_unknown_texture_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, 42);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
    }

    #[test]
    fn framebuffer_texture_2d_level_outside_the_mip_range_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, CAP_MIP_LEVELS as GLint);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
    }

    #[test]
    fn framebuffer_texture_2d_name_zero_detaches() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: plain GL call; the level is ignored when detaching.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 5) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let a = gl.ctx().framebuffers[fb as usize].color;
        assert_eq!((a.type_, a.name), (GL_NONE, 0));
    }

    #[test]
    fn framebuffer_renderbuffer_records_the_attachment_per_point() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let color = rb_with_storage(GL_RGBA4, 4, 4);
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        let stencil = rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, color);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        attach_rb(GL_STENCIL_ATTACHMENT, stencil);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.color.type_, f.color.name), (GL_RENDERBUFFER, color));
        assert_eq!((f.depth.type_, f.depth.name), (GL_RENDERBUFFER, depth));
        assert_eq!((f.stencil.type_, f.stencil.name), (GL_RENDERBUFFER, stencil));
    }

    #[test]
    fn framebuffer_renderbuffer_depth_stencil_point_sets_depth_and_stencil() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let ds = rb_with_storage(GL_DEPTH24_STENCIL8, 4, 4);
        attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, ds);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.depth.type_, f.depth.name), (GL_RENDERBUFFER, ds));
        assert_eq!((f.stencil.type_, f.stencil.name), (GL_RENDERBUFFER, ds));
        attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, 0);
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.depth.type_, f.stencil.type_), (GL_NONE, GL_NONE));
    }

    #[test]
    fn framebuffer_renderbuffer_errors() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the default framebuffer has no attachment points");

        let fb = bound_fb();
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferRenderbuffer(GL_TEXTURE_2D, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rb);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "renderbuffertarget");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "attachment past the cap");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb + 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not a renderbuffer");
        }
        assert_eq!(gl.ctx().framebuffers[fb as usize].color.type_, GL_NONE);
    }

    /* ---- completeness ---------------------------------------------------------- */

    #[test]
    fn status_of_the_default_framebuffer_is_complete() {
        let gl = TestGl::new(8, 8);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn status_bad_target_is_invalid_enum_and_returns_zero() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_RENDERBUFFER) }, 0);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn status_accepts_the_es3_draw_and_read_targets() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glCheckFramebufferStatus(GL_DRAW_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            assert_eq!(glCheckFramebufferStatus(GL_READ_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn status_without_attachments_is_missing_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT);
    }

    #[test]
    fn status_complete_with_a_texture_colour_attachment() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn status_complete_with_colour_renderbuffer_and_packed_depth_stencil() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let color = rb_with_storage(GL_RGBA8, 6, 3);
        let ds = rb_with_storage(GL_DEPTH24_STENCIL8, 6, 3);
        attach_rb(GL_COLOR_ATTACHMENT0, color);
        attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, ds);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    }

    #[test]
    fn status_complete_with_separate_depth_and_stencil_renderbuffers() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        let stencil = rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        attach_rb(GL_STENCIL_ATTACHMENT, stencil);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    }

    #[test]
    fn status_texture_without_storage_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let mut tex: GLuint = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
        }
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_renderbuffer_without_storage_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let rb = gen_rb();
        /* bound, so the object exists; attaching a name that was only
         * generated is GL_INVALID_OPERATION and attaches nothing. */
        // SAFETY: plain GL call.
        unsafe { glBindRenderbuffer(GL_RENDERBUFFER, rb) };
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_zero_sized_colour_renderbuffer_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let rb = rb_with_storage(GL_RGBA4, 0, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_depth_renderbuffer_as_colour_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let rb = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_colour_renderbuffer_as_depth_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_texture_as_depth_is_incomplete_attachment() {
        /* textures are RGBA8888 storage: not depth renderable */
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let color = tex_with_storage(4, 4);
        let depth = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, color);
        attach_tex(GL_DEPTH_ATTACHMENT, depth);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_depth_only_format_as_stencil_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_stencil_only_format_as_depth_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn status_depth_size_mismatch_is_incomplete_dimensions() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 8);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    }

    #[test]
    fn status_stencil_size_mismatch_is_incomplete_dimensions() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_STENCIL_INDEX8, 2, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    }

    #[test]
    fn status_depth_and_stencil_of_different_sizes_without_colour_is_incomplete_dimensions() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        let stencil = rb_with_storage(GL_STENCIL_INDEX8, 8, 8);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        attach_rb(GL_STENCIL_ATTACHMENT, stencil);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    }

    /// The status of a depth-only framebuffer: complete where the backend
    /// draws with no colour target (backend::DEPTH_ONLY_TARGETS, the
    /// GC7000), unsupported where every kick renders into a colour buffer
    /// (ES 2.0 4.4.5 lets an implementation say so). Colour only on an
    /// attachment past 0 stays unsupported everywhere.
    #[test]
    fn status_depth_only_framebuffer_is_complete_where_the_backend_draws_without_colour() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        let want =
            if crate::backend::DEPTH_ONLY_TARGETS { GL_FRAMEBUFFER_COMPLETE } else { GL_FRAMEBUFFER_UNSUPPORTED };
        assert_eq!(status(), want);
    }

    #[test]
    fn status_follows_the_texture_storage_being_respecified() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        // SAFETY: null pixels only allocate; the texture is still bound.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        }
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
    }

    /* ---- colour attachments 1.. (ES 2.0.25 4.4.5, ES 3.0.6 4.4.4) ------------------ */

    /// Attachment 1 of another size than attachment 0: on the ES 2.0 surface
    /// GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS ("All attached images have the
    /// same width and height"); ES 3.0 cannot generate that token (4.4.4.2),
    /// so there it is complete. Before, attachment 1 was never checked.
    #[test]
    fn status_colour_attachment_1_of_another_size() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(4, 4));
        attach_tex(GL_COLOR_ATTACHMENT1, tex_with_storage(8, 4));
        let want = if crate::REPORT_ES2 { GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS } else { GL_FRAMEBUFFER_COMPLETE };
        assert_eq!(status(), want);
    }

    /// Attachment 1 without a colour-renderable format (a depth
    /// renderbuffer): GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, on both surfaces.
    #[test]
    fn status_colour_attachment_1_with_a_depth_format_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(4, 4));
        attach_rb(GL_COLOR_ATTACHMENT1, rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4));
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    /// Attachment 1 a texture with no image: GL_FRAMEBUFFER_INCOMPLETE_
    /// ATTACHMENT ("The width and height of image must be non-zero").
    #[test]
    fn status_colour_attachment_1_with_no_image_is_incomplete_attachment() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(4, 4));
        let t = gen_tex();
        // SAFETY: a plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, t) };
        attach_tex(GL_COLOR_ATTACHMENT1, t);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    /// Three colour attachments of one size, and a depth renderbuffer of that
    /// size: complete.
    #[test]
    fn status_three_colour_attachments_of_one_size_are_complete() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        for a in [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2] {
            attach_tex(a, tex_with_storage(4, 4));
        }
        attach_rb(GL_DEPTH_ATTACHMENT, rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4));
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    }

    /// Attachment 1 alone, attachment 0 empty: an image is attached, so not
    /// INCOMPLETE_MISSING_ATTACHMENT (which it read while attachment 1 went
    /// unchecked); every render writes attachment 0 here, so UNSUPPORTED, the
    /// implementation-dependent restriction ES 2.0 4.4.5 allows.
    #[test]
    fn status_colour_attachment_1_alone_is_unsupported() {
        let _gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT1, tex_with_storage(4, 4));
        assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED);
    }

    /* ---- deleting attached objects ----------------------------------------------- */

    #[test]
    fn deleting_an_attached_texture_detaches_it_from_the_bound_framebuffer_only() {
        /* ES 2.0 3.7.13: the attachment is broken where the texture is
         * attached to the framebuffer that is bound, and nowhere else. */
        let gl = TestGl::new(8, 8);
        let tex = tex_with_storage(4, 4);
        let other = tex_with_storage(4, 4);
        let a = bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let b = bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let keep = bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, other);
        // SAFETY: one readable name.
        unsafe { glDeleteTextures(1, &tex) };
        let c = gl.ctx();
        assert_eq!(c.framebuffers[a as usize].color.name, tex, "framebuffer a was not bound: it keeps the name");
        assert_eq!(c.framebuffers[b as usize].color.name, tex, "framebuffer b was not bound either");
        assert_eq!(c.framebuffers[keep as usize].color.name, other, "other textures stay attached");
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* and the bound one does lose it */
        // SAFETY: plain GL calls; `other` is a live name.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, keep);
            glDeleteTextures(1, &other);
        }
        assert_eq!(gl.ctx().framebuffers[keep as usize].color.type_, GL_NONE);
        assert_eq!(gl.ctx().framebuffers[keep as usize].color.name, 0);
    }

    #[test]
    fn deleting_an_attached_renderbuffer_detaches_every_point_it_was_on() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        let ds = rb_with_storage(GL_DEPTH24_STENCIL8, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, ds);
        // SAFETY: one readable name.
        unsafe { glDeleteRenderbuffers(1, &ds) };
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.depth.type_, f.stencil.type_), (GL_NONE, GL_NONE));
        assert_eq!((f.color.type_, f.color.name), (GL_TEXTURE, tex));
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    }

    #[test]
    fn a_texture_and_a_renderbuffer_of_the_same_name_are_told_apart_on_delete() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        assert_eq!(tex, rb, "both tables hand out name 1 first");
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        // SAFETY: one readable name.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!(f.depth.type_, GL_NONE);
        assert_eq!((f.color.type_, f.color.name), (GL_TEXTURE, tex));
    }

    #[test]
    fn c_abi_deleted_hooks_detach_on_the_current_context() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let tex = tex_with_storage(4, 4);
        let rb = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        // SAFETY: plain calls on the current context.
        unsafe {
            gl_fb_texture_deleted(tex);
            gl_fb_renderbuffer_deleted(rb);
        }
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.color.type_, f.depth.type_), (GL_NONE, GL_NONE));
    }

    /* ---- attachment queries -------------------------------------------------------- */

    #[test]
    fn attachment_query_texture_attachment() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_TEXTURE as GLint);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), tex as GLint);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL), 0);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attachment_query_renderbuffer_attachment() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let rb = rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);
        assert_eq!(fb_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_RENDERBUFFER as GLint);
        assert_eq!(fb_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), rb as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* the texture-only pnames are INVALID_ENUM for a renderbuffer */
        assert_eq!(fb_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(fb_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn attachment_query_empty_point_reports_none_and_rejects_other_pnames() {
        /* An attachment point with nothing attached, where the two versions
         * genuinely differ (dEQP reaches this through
         * negative_api.state.get_framebuffer_attachment_parameteriv, which
         * asks glGetIntegerv(GL_MAJOR_VERSION) first and then expects one
         * answer or the other):
         *
         *   ES 2.0 4.4.5   only OBJECT_TYPE may be asked; the rest is
         *                  INVALID_ENUM.
         *   ES 3.0 6.1.13  OBJECT_NAME answers 0, and any other attachment
         *                  pname is INVALID_OPERATION -- the enum is valid,
         *                  the state is what cannot answer it.
         */
        let gl = TestGl::new(8, 8);
        bound_fb();
        assert_eq!(fb_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_NONE as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* one call, then its error: the first error wins and no further one
         * is recorded until glGetError is called, so reading them out of step
         * would test the previous call's error against this call's value */
        let name = fb_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME);
        if crate::REPORT_ES2 {
            assert_eq!((name, gl.error()), (-1, GL_INVALID_ENUM));
        } else {
            assert_eq!((name, gl.error()), (0, GL_NO_ERROR), "ES 3.0: the name of nothing is 0, not an error");
        }

        let level = fb_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL);
        let want = if crate::REPORT_ES2 { GL_INVALID_ENUM } else { GL_INVALID_OPERATION };
        assert_eq!((level, gl.error()), (-1, want));

        /* not an attachment query at all: INVALID_ENUM under either version */
        assert_eq!(fb_param(GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
    }

    #[test]
    fn attachment_query_errors() {
        let gl = TestGl::new(8, 8);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), -1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "the default framebuffer");

        bound_fb();
        let mut v: GLint = -1;
        // SAFETY: `v` is writable; null params is tolerated.
        unsafe {
            glGetFramebufferAttachmentParameteriv(
                GL_RENDERBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
                &mut v,
            );
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            glGetFramebufferAttachmentParameteriv(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE,
                core::ptr::null_mut(),
            );
            assert_eq!(gl.error(), GL_NO_ERROR, "null params");
        }
        assert_eq!(v, -1);
        /* every point below GL_MAX_COLOR_ATTACHMENTS exists (empty:
         * GL_NONE): ES 3.0, and ES 2.0 with GL_NV_fbo_color_attachments */
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT1, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_NONE as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR, "attachment 1 is a point");
        let past = GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum;
        assert_eq!(fb_param(past, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "attachment");
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER_WIDTH), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "pname");
    }

    /// ES 3.0 6.1.13's image pnames (dEQP-GLES3 state_query.fbo.*, 12
    /// cases that read an error until 2026-09-24): sizes, component type
    /// and encoding of the attached image (0 for an image with no storage
    /// yet), the layer of an array attachment, and the default
    /// framebuffer's GL_BACK / GL_DEPTH / GL_STENCIL.
    #[test]
    fn attachment_query_answers_the_es3_image_pnames() {
        let gl = TestGl::new(8, 8);
        if crate::REPORT_ES2 {
            return;
        }
        let sizes = |att| {
            [
                GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
                GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
                GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
                GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
                GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE,
                GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE,
            ]
            .map(|p| fb_param(att, p))
        };
        assert_eq!(fb_param(GL_BACK, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_FRAMEBUFFER_DEFAULT as GLint);
        assert_eq!(sizes(GL_BACK), [8, 8, 8, 8, 0, 0]);
        assert_eq!(fb_param(GL_DEPTH, GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE), 24);
        assert_eq!(fb_param(GL_BACK, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "the default framebuffer has no object name");
        bound_fb();
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(sizes(GL_COLOR_ATTACHMENT0), [4, 4, 4, 4, 0, 0]);
        let srgb = rb_with_storage(GL_SRGB8_ALPHA8, 4, 4);
        attach_rb(GL_COLOR_ATTACHMENT0, srgb);
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING), GL_SRGB as GLint);
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        assert_eq!(sizes(GL_DEPTH_ATTACHMENT), [0, 0, 0, 0, 16, 0]);
        assert_eq!(fb_param(GL_DEPTH_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING), GL_LINEAR as GLint);
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(sizes(GL_COLOR_ATTACHMENT0), [8, 8, 8, 8, 0, 0]);
        assert_eq!(
            fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE),
            GL_UNSIGNED_NORMALIZED as GLint
        );
        assert_eq!(fb_param(GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER), 0);
        /* an image with no storage yet has 0 bits */
        let empty = gen_rb();
        // SAFETY: plain GL call.
        unsafe { glBindRenderbuffer(GL_RENDERBUFFER, empty) };
        attach_rb(GL_COLOR_ATTACHMENT0, empty);
        assert_eq!(sizes(GL_COLOR_ATTACHMENT0), [0; 6]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attachment_query_depth_stencil_point() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let ds = rb_with_storage(GL_DEPTH24_STENCIL8, 4, 4);
        attach_rb(GL_DEPTH_STENCIL_ATTACHMENT, ds);
        assert_eq!(fb_param(GL_DEPTH_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), ds as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- renderbuffers: gen / bind / delete / is -------------------------------------- */

    #[test]
    fn gen_renderbuffers_returns_distinct_nonzero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [0 as GLuint; 3];
        // SAFETY: room for three names.
        unsafe { glGenRenderbuffers(3, names.as_mut_ptr()) };
        assert!(names.iter().all(|&n| n != 0));
        assert!(names[0] != names[1] && names[1] != names[2] && names[0] != names[2]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.live_allocs(), 1, "names own no memory");
    }

    #[test]
    fn gen_renderbuffers_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let mut n: GLuint = 99;
        // SAFETY: a negative count writes nothing; null is tolerated.
        unsafe {
            glGenRenderbuffers(-1, &mut n);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGenRenderbuffers(1, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(n, 99);
    }

    #[test]
    fn gen_renderbuffers_past_the_table_returns_zero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [77 as GLuint; MAX_RENDERBUFFERS + 2];
        // SAFETY: room for every name asked for.
        unsafe { glGenRenderbuffers(names.len() as GLsizei, names.as_mut_ptr()) };
        assert!(names[..MAX_RENDERBUFFERS - 1].iter().all(|&n| n != 0 && (n as usize) < MAX_RENDERBUFFERS));
        assert!(names[MAX_RENDERBUFFERS - 1..].iter().all(|&n| n == 0));
        /* an exhausted pool says so (upstream 7219228) */
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn is_renderbuffer_false_until_first_bound() {
        let _gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsRenderbuffer(rb), GL_FALSE_B);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            assert_eq!(glIsRenderbuffer(rb), GL_TRUE_B);
        }
    }

    #[test]
    fn is_renderbuffer_for_bound_deleted_zero_and_out_of_range_names() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            assert_eq!(glIsRenderbuffer(rb), GL_TRUE_B);
            assert_eq!(glIsRenderbuffer(0), GL_FALSE_B);
            assert_eq!(glIsRenderbuffer(rb + 1), GL_FALSE_B);
            assert_eq!(glIsRenderbuffer(MAX_RENDERBUFFERS as GLuint), GL_FALSE_B);
            glDeleteRenderbuffers(1, &rb);
            assert_eq!(glIsRenderbuffer(rb), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_renderbuffer_records_the_binding_and_creates_unknown_names() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            assert_eq!(gl.ctx().bound_renderbuffer, rb);
            glBindRenderbuffer(GL_RENDERBUFFER, 7);
            assert_eq!(gl.ctx().bound_renderbuffer, 7);
            assert_eq!(glIsRenderbuffer(7), GL_TRUE_B);
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
            assert_eq!(gl.ctx().bound_renderbuffer, 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_renderbuffer_errors_keep_the_binding() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glBindRenderbuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            /* past the far-name table: GL_OUT_OF_MEMORY (gl/names.rs) */
            for i in 0..super::names::FAR_NAMES as GLuint {
                glBindRenderbuffer(GL_RENDERBUFFER, 0x1000 + i);
            }
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glBindRenderbuffer(GL_RENDERBUFFER, MAX_RENDERBUFFERS as GLuint);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        }
        assert_eq!(gl.ctx().bound_renderbuffer, rb);
    }

    #[test]
    fn delete_renderbuffer_frees_its_storage_and_unbinds_it() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        let gpu = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        assert_eq!(gl.live_allocs(), 2);
        gl.mem_events();
        // SAFETY: one readable name.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
        assert_eq!(gl.live_allocs(), 1);
        assert_eq!(gl.ctx().bound_renderbuffer, 0);
        assert!(!gl.ctx().objs().renderbuffers[rb as usize].used);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_renderbuffers_errors_and_ignored_names() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        let names = [0 as GLuint, rb + 1, MAX_RENDERBUFFERS as GLuint, GLuint::MAX];
        // SAFETY: a negative count reads nothing; four readable names; null is tolerated.
        unsafe {
            glDeleteRenderbuffers(-1, &rb);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteRenderbuffers(4, names.as_ptr());
            glDeleteRenderbuffers(1, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().renderbuffers[rb as usize].used);
        assert_eq!(gl.ctx().bound_renderbuffer, rb);
        assert_eq!(gl.live_allocs(), 2);
    }

    /* ---- glRenderbufferStorage ---------------------------------------------------------- */

    #[test]
    fn colour_storage_is_rgba8888_at_the_even_row_pitch() {
        /* whatever the (smaller) requested format: tex_pitch(w) * h * 4 bytes */
        let gl = TestGl::new(8, 8);
        for (format, w, h, bytes) in [
            (GL_RGBA4, 4, 4, 4 * 4 * 4),
            (GL_RGB565, 5, 3, 6 * 3 * 4),
            (GL_RGB5_A1, 1, 1, 2 * 4),
            (GL_RGBA8, 7, 2, 8 * 2 * 4),
            (GL_R8, 3, 5, 4 * 5 * 4),
        ] {
            gl.mem_events();
            let rb = rb_with_storage(format, w, h);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let (gpu, size) = {
                let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
                assert_eq!((r.internalformat, r.width, r.height), (format, w as u32, h as u32));
                assert!(!r.external);
                (r.mem.gpu, r.mem.size)
            };
            assert_eq!(size, bytes);
            assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: bytes, gpu }]);
        }
    }

    #[test]
    fn depth_and_stencil_storage_owns_no_memory() {
        let gl = TestGl::new(8, 8);
        for format in [
            GL_DEPTH_COMPONENT16,
            GL_DEPTH_COMPONENT24,
            GL_DEPTH_COMPONENT32F,
            GL_STENCIL_INDEX8,
            GL_DEPTH24_STENCIL8,
            GL_DEPTH32F_STENCIL8,
        ] {
            let rb = rb_with_storage(format, 16, 16);
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{format:x}");
            let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
            assert_eq!((r.internalformat, r.width, r.height), (format, 16, 16));
            assert!(r.mem.impl_.is_null());
        }
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn storage_again_frees_the_old_allocation_first() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        let old = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        gl.mem_events();
        // SAFETY: plain GL call; the renderbuffer is still bound.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 2) };
        let new = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Free { gpu: old }, MemEvent::Alloc { heap: HEAP_DATA, size: 8 * 2 * 4, gpu: new }]
        );
        assert_eq!(gl.live_allocs(), 2);
    }

    #[test]
    fn storage_changing_colour_to_depth_releases_the_colour_memory() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 4, 4) };
        assert_eq!(gl.live_allocs(), 1);
        assert!(gl.ctx().objs().renderbuffers[rb as usize].mem.impl_.is_null());
    }

    #[test]
    fn storage_of_zero_size_is_accepted_and_allocates_nothing() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 0, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.mem_events().is_empty());
        let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
        assert_eq!((r.internalformat, r.width, r.height), (GL_RGBA4, 0, 0));
    }

    #[test]
    fn storage_at_the_maximum_size_is_accepted() {
        let gl = TestGl::new(8, 8);
        rb_with_storage(GL_DEPTH_COMPONENT16, 4096, 4096);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 4096);
        assert_eq!(rb_param(GL_RENDERBUFFER_HEIGHT), 4096);
    }

    #[test]
    fn storage_size_errors_are_invalid_value_and_keep_the_old_storage() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        gl.mem_events();
        for (w, h) in [(-1, 4), (4, -1), (4097, 4), (4, 4097)] {
            // SAFETY: plain GL call.
            unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, w, h) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "{w}x{h}");
        }
        assert!(gl.mem_events().is_empty());
        let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
        assert_eq!((r.width, r.height), (4, 4));
        assert!(!r.mem.impl_.is_null());
    }

    #[test]
    fn storage_enum_errors() {
        let gl = TestGl::new(8, 8);
        gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, 1);
            glRenderbufferStorage(GL_FRAMEBUFFER, GL_RGBA4, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            for format in [GL_DEPTH_COMPONENT, GL_DEPTH_STENCIL, GL_LUMINANCE, 0] {
                glRenderbufferStorage(GL_RENDERBUFFER, format, 4, 4);
                assert_eq!(gl.error(), GL_INVALID_ENUM, "0x{format:x}");
            }
        }
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn storage_without_a_bound_renderbuffer_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 4, 4) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn storage_allocation_failure_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        gl.fail_alloc_in(1);
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 4, 4);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1);
        assert!(gl.ctx().objs().renderbuffers[rb as usize].mem.impl_.is_null());
        /* and it does not render */
        bound_fb();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn storage_reallocation_failure_leaves_no_storage_and_no_leak() {
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        gl.fail_alloc_in(1);
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 8) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), 1, "the old storage was released, none took its place");
        assert!(gl.ctx().objs().renderbuffers[rb as usize].mem.impl_.is_null());
        /* the next attempt succeeds */
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 8) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.live_allocs(), 2);
    }

    /* ---- glGetRenderbufferParameteriv ------------------------------------------------------ */

    #[test]
    fn renderbuffer_params_of_a_colour_renderbuffer() {
        let gl = TestGl::new(8, 8);
        rb_with_storage(GL_RGB565, 5, 3);
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 5);
        assert_eq!(rb_param(GL_RENDERBUFFER_HEIGHT), 3);
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT), GL_RGB565 as GLint);
        /* the storage is RGBA8888 whatever was asked for */
        for p in [
            GL_RENDERBUFFER_RED_SIZE,
            GL_RENDERBUFFER_GREEN_SIZE,
            GL_RENDERBUFFER_BLUE_SIZE,
            GL_RENDERBUFFER_ALPHA_SIZE,
        ] {
            assert_eq!(rb_param(p), 8, "0x{p:x}");
        }
        assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* ES 3.0's sample count (dEQP-GLES3 state_query.rbo.renderbuffer_samples) */
        if !crate::REPORT_ES2 {
            assert_eq!(rb_param(GL_RENDERBUFFER_SAMPLES), 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn renderbuffer_params_of_depth16_and_stencil8() {
        let gl = TestGl::new(8, 8);
        rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), 16);
        assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_RED_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_ALPHA_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT), GL_DEPTH_COMPONENT16 as GLint);
        rb_with_storage(GL_STENCIL_INDEX8, 4, 4);
        assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), 8);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn renderbuffer_params_depth_and_stencil_bits_of_the_es3_formats() {
        let gl = TestGl::new(8, 8);
        for (format, depth, stencil) in [
            (GL_DEPTH_COMPONENT24, 24, 0),
            (GL_DEPTH24_STENCIL8, 24, 8),
            (GL_DEPTH_COMPONENT32F, 32, 0),
            (GL_DEPTH32F_STENCIL8, 32, 8),
        ] {
            rb_with_storage(format, 4, 4);
            assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), depth, "0x{format:x}");
            assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), stencil, "0x{format:x}");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn renderbuffer_params_before_storage_are_the_initial_values() {
        /* GL ES 2.0 table 6.23: 0 x 0, GL_RGBA4, no bits */
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL call.
        unsafe { glBindRenderbuffer(GL_RENDERBUFFER, rb) };
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_HEIGHT), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT), GL_RGBA4 as GLint);
        assert_eq!(rb_param(GL_RENDERBUFFER_RED_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_DEPTH_SIZE), 0);
        assert_eq!(rb_param(GL_RENDERBUFFER_STENCIL_SIZE), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn renderbuffer_params_errors() {
        let gl = TestGl::new(8, 8);
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), -1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "no renderbuffer bound");

        rb_with_storage(GL_RGBA4, 4, 4);
        let mut v: GLint = -1;
        // SAFETY: `v` is writable; null params is tolerated.
        unsafe {
            glGetRenderbufferParameteriv(GL_FRAMEBUFFER, GL_RENDERBUFFER_WIDTH, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_WIDTH, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR, "null params");
        }
        assert_eq!(v, -1);
        assert_eq!(rb_param(GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "pname");
    }

    /* ---- the render target ---------------------------------------------------------------------- */

    #[test]
    fn target_of_the_default_framebuffer_is_the_window_surface_flipped() {
        let gl = TestGl::new(16, 8);
        let c = gl.ctx();
        let (mem, w, h, _) = fb_target(c).expect("the default surface");
        assert_eq!((mem.gpu, w, h), (c.default_surface.gpu, 16, 8));
        assert!(fb_flip_y(c), "the window surface is stored top-down");
    }

    #[test]
    fn target_without_a_default_surface_is_none() {
        let gl = TestGl::new(16, 8);
        let c = gl.ctx();
        c.default_surface_valid = false;
        let none = fb_target(c).is_none();
        c.default_surface_valid = true; /* TestGl::drop frees it */
        assert!(none);
    }

    #[test]
    fn target_of_a_complete_fbo_is_the_colour_texture_unflipped() {
        let gl = TestGl::new(16, 8);
        bound_fb();
        let tex = tex_with_storage(4, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let c = gl.ctx();
        let (mem, w, h, _) = fb_target(c).expect("a complete framebuffer");
        assert_eq!((mem.gpu, w, h), (c.objs().textures[tex as usize].mem.gpu, 4, 2));
        assert!(!fb_flip_y(c), "a texture is stored bottom-up");
    }

    #[test]
    fn target_of_a_complete_fbo_with_a_colour_renderbuffer() {
        let gl = TestGl::new(16, 8);
        bound_fb();
        let rb = rb_with_storage(GL_RGBA8, 5, 3);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        let c = gl.ctx();
        let (mem, w, h, _) = fb_target(c).expect("a complete framebuffer");
        assert_eq!((mem.gpu, w, h), (c.objs().renderbuffers[rb as usize].mem.gpu, 5, 3));
    }

    #[test]
    fn target_of_an_incomplete_fbo_is_none() {
        let gl = TestGl::new(16, 8);
        bound_fb();
        assert!(fb_target(gl.ctx()).is_none());
        let depth = rb_with_storage(GL_DEPTH_COMPONENT16, 4, 4);
        attach_rb(GL_DEPTH_ATTACHMENT, depth);
        assert!(fb_target(gl.ctx()).is_none(), "depth only");
    }

    #[test]
    fn c_abi_target_points_at_the_descriptor_inside_the_context() {
        let gl = TestGl::new(16, 8);
        let (mut w, mut h) = (99u32, 99u32);
        // SAFETY: `w` and `h` are writable.
        let m = unsafe { gl_fb_target(&mut w, &mut h, core::ptr::null_mut()) };
        assert_eq!(m as *const OsalGpuMem, &gl.ctx().default_surface as *const OsalGpuMem);
        assert_eq!((w, h), (16, 8));
        // SAFETY: plain call.
        assert!(unsafe { gl_fb_flip_y() });

        bound_fb();
        // SAFETY: as above.
        let m = unsafe { gl_fb_target(&mut w, &mut h, core::ptr::null_mut()) };
        assert!(m.is_null(), "incomplete");
        assert_eq!((w, h), (0, 0));

        let tex = tex_with_storage(4, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: as above.
        let m = unsafe { gl_fb_target(&mut w, &mut h, core::ptr::null_mut()) };
        assert_eq!(m as *const OsalGpuMem, &gl.ctx().objs_ref().textures[tex as usize].mem as *const OsalGpuMem);
        assert_eq!((w, h), (4, 2));
        // SAFETY: plain call.
        assert!(!unsafe { gl_fb_flip_y() });
    }

    #[test]
    fn a_clear_into_an_fbo_targets_the_texture_without_flip() {
        let gl = TestGl::new(16, 8);
        bound_fb();
        let tex = tex_with_storage(4, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 1.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        let d = &draws[0].draw;
        assert_eq!(d.target_gpu, gl.ctx().objs().textures[tex as usize].mem.gpu);
        assert_eq!((d.target_w, d.target_h), (4, 2));
        assert!(!d.flip_y);
        assert_eq!(d.target_format, GLES_SURF_RGBA8888);

        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].draw.target_gpu, gl.ctx().default_surface.gpu);
        assert!(draws[0].draw.flip_y);
    }

    /* ---- glReadPixels ------------------------------------------------------------------------------ */

    #[test]
    fn read_pixels_swaps_a_target_the_pixel_engine_wrote_as_bgra() {
        /* upstream 9ca8a45: a pixel engine with no RGBA8888 render format
         * writes a GL RGBA8888 target as A8R8G8B8, which is B,G,R,A in
         * memory. Sampling it is corrected by the texture descriptor; the
         * path that hands the bytes to the application is not, and every
         * colour an application read back had red and blue transposed. */
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let plain = read(0, 0, 4, 3);
        gl.backend_like_gc7000();
        let swapped = read(0, 0, 4, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_ne!(plain, swapped);
        for (p, q) in plain.as_chunks::<4>().0.iter().zip(swapped.as_chunks::<4>().0.iter()) {
            /* red and blue transposed, and the alpha the application
             * rendered kept -- upstream forces it to 0xff here, which is
             * right for the X of XRGB8888 and wrong for an RGBA8888 target */
            assert_eq!(*q, [p[2], p[1], p[0], p[3]]);
            assert_ne!(p[3], 0xff, "the fixture's alpha is not opaque");
        }
    }

    /// A readback after GPU work that did not complete -- the GC7000 driver
    /// resets the GPU on a fence timeout and reports -ECANCELED -- hands out
    /// nothing: the rows stay as the caller left them, no GL error is made up
    /// (ES 2.0/3.0 have none for it), and the loss is counted. It used to
    /// read the target the lost kick never drew (color_clear on the i.MX8MP).
    #[test]
    fn read_pixels_after_lost_gpu_work_writes_no_row_and_counts_it() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let good = read(0, 0, 4, 3);
        assert!(good.iter().any(|b| *b != SENTINEL), "control: a completed readback writes its rows");

        for (what, flush_rc, finish_rc) in [("a wait the reset cancelled", 0, -125), ("a submit that failed", -5, 0)] {
            let before = crate::backend::gpu_work_lost();
            gl.set_flush_result(flush_rc);
            gl.set_finish_result(finish_rc);
            let lost = read(0, 0, 4, 3);
            gl.set_flush_result(0);
            gl.set_finish_result(0);
            assert!(lost.iter().all(|b| *b == SENTINEL), "{what}: no row is written");
            assert_eq!(gl.error(), GL_NO_ERROR, "{what}: no GL error is invented");
            /* sticky and process-wide: other threads may add to it, never take away */
            assert!(crate::backend::gpu_work_lost() > before, "{what}: the loss is counted");
        }
        assert_eq!(read(0, 0, 4, 3), good, "and the next completed readback is whole again");
    }

    /// dEQP's random-shader and shader-render cases draw, call glFlush, then
    /// glReadPixels. The flush submits the job; when it is lost (the Pi 4's
    /// MMU faults, 2026-09-25: struct.uniform case #30 passed with its own
    /// job faulted), glFlush drops the error, the read's own flush and finish
    /// find nothing to do, and the read handed out what the lost job left.
    /// The read after such a loss hands out nothing, as one that met the loss
    /// itself; the one after it reads again.
    #[test]
    fn a_loss_an_earlier_glflush_or_glfinish_dropped_withholds_the_next_read() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let good = read(0, 0, 4, 3);
        assert!(good.iter().any(|b| *b != SENTINEL), "control: a completed readback writes its rows");

        type Call = unsafe extern "C" fn();
        for (what, call, flush_rc, finish_rc) in [
            ("glFlush, the submit failed", glFlush as Call, -14, 0),
            ("glFinish, the wait failed", crate::gl::state::glFinish as Call, 0, -14),
        ] {
            gl.set_flush_result(flush_rc);
            gl.set_finish_result(finish_rc);
            // SAFETY: a plain GL call.
            unsafe { call() };
            gl.set_flush_result(0);
            gl.set_finish_result(0);
            assert_eq!(gl.error(), GL_NO_ERROR, "{what}: no GL error is invented");
            let lost = read(0, 0, 4, 3);
            assert!(lost.iter().all(|b| *b == SENTINEL), "{what}: the next read writes no row");
            assert_eq!(read(0, 0, 4, 3), good, "{what}: the read after it is whole again");
        }
    }

    /// A loss from before a context existed is not that context's: its first
    /// read reads.
    #[test]
    fn a_new_context_does_not_inherit_an_earlier_loss() {
        {
            let gl = TestGl::new(4, 3);
            gl.set_flush_result(-14);
            // SAFETY: a plain GL call.
            unsafe { glFlush() };
            gl.set_flush_result(0);
        }
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        assert!(read(0, 0, 4, 3).iter().any(|b| *b != SENTINEL), "the first read of a new context reads");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn read_pixels_window_surface_rows_come_back_bottom_up() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let out = read(0, 0, 4, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for j in 0..3 {
            for x in 0..4 {
                /* GL row j (from the bottom) is stored row 2 - j (from the top) */
                assert_eq!(pixel(&out, 4, x, j), px(x, 2 - j), "x {x} row {j}");
            }
        }
    }

    #[test]
    fn read_pixels_fbo_texture_rows_are_not_flipped() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(gl.ctx().objs().textures[tex as usize].level_off[0], 0);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 4, 4, 4);
        let out = read(0, 0, 4, 4);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for j in 0..4 {
            for x in 0..4 {
                assert_eq!(pixel(&out, 4, x, j), px(x, j), "x {x} row {j}");
            }
        }
    }

    #[test]
    fn read_pixels_fbo_renderbuffer() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let rb = rb_with_storage(GL_RGBA8, 2, 2);
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        fill_rgba(&gl.ctx().objs_ref().renderbuffers[rb as usize].mem, 2, 2, 2);
        let out = read(0, 0, 2, 2);
        assert_eq!(out, [px(0, 0), px(1, 0), px(0, 1), px(1, 1)].concat());
    }

    #[test]
    fn read_pixels_odd_width_target_uses_the_even_row_pitch() {
        /* a 5-texel-wide target stores its rows 6 texels apart */
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(5, 3);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 6, 5, 3);
        let out = read(0, 0, 5, 3);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for j in 0..3 {
            for x in 0..5 {
                assert_eq!(pixel(&out, 5, x, j), px(x, j), "x {x} row {j}");
            }
        }
        /* the caller's rows are tightly packed: no padding texel shows up */
        assert!(!out.contains(&0xEE));
    }

    #[test]
    /* a BUG test until upstream fixed it (7e76640) */
    fn read_pixels_odd_width_default_surface_lies_inside_the_surface() {
        let gl = TestGl::new(5, 3);
        let c = gl.ctx();
        /* what glReadPixels(0, 0, 5, 3) touches: the last texel of the last row at the target's pitch */
        let end = ((c.target_h as usize - 1) * tex_pitch(c.target_w) as usize + c.target_w as usize) * 4;
        assert!(end <= c.default_surface.size, "reads up to byte {end} of {}", c.default_surface.size);
    }

    #[test]
    fn read_pixels_sub_rectangle_of_the_window() {
        let gl = TestGl::new(4, 4);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 4);
        let out = read(1, 1, 2, 2);
        /* GL rows 1 and 2 are stored rows 2 and 1 */
        assert_eq!(out, [px(1, 2), px(2, 2), px(1, 1), px(2, 1)].concat());
    }

    #[test]
    fn read_pixels_clips_left_and_bottom_leaving_outside_pixels_untouched() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let out = read(-1, -1, 3, 3);
        let s = [SENTINEL; 4];
        /* row 0 is below the window; column 0 is left of it */
        assert_eq!(&out[..12], [s, s, s].concat());
        assert_eq!(&out[12..24], [s, px(0, 2), px(1, 2)].concat());
        assert_eq!(&out[24..], [s, px(0, 1), px(1, 1)].concat());
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn read_pixels_clips_right_and_top_leaving_outside_pixels_untouched() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let out = read(2, 1, 4, 3);
        let s = [SENTINEL; 4];
        assert_eq!(&out[..16], [px(2, 1), px(3, 1), s, s].concat());
        assert_eq!(&out[16..32], [px(2, 0), px(3, 0), s, s].concat());
        assert_eq!(&out[32..], [s; 4].concat(), "above the window");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn read_pixels_entirely_outside_touches_nothing() {
        let gl = TestGl::new(4, 3);
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        for (x, y) in [(4, 0), (-2, 0), (0, 3), (0, -2), (GLint::MAX, 0), (0, GLint::MAX)] {
            let out = read(x, y, 2, 2);
            assert!(out.iter().all(|&b| b == SENTINEL), "({x}, {y})");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn read_pixels_clips_against_the_fbo_attachment_not_the_window() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(2, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 2, 2, 2);
        let out = read(1, 1, 3, 3);
        let s = [SENTINEL; 4];
        assert_eq!(&out[..12], [px(1, 1), s, s].concat());
        assert!(out[12..].iter().all(|&b| b == SENTINEL));
    }

    #[test]
    fn read_pixels_rgb565_window_expands_to_rgba8_by_bit_replication() {
        let gl = TestGl::new(8, 8);
        {
            let s = SurfaceCtx::new(&gl, 2, 2, GLES_SURF_RGB565, 2 * 2 * 2);
            /* stored top-down, little-endian R5G6B5:
             *   top:    pure red 0xF800, pure green 0x07E0
             *   bottom: 0x1234 (r5 2, g6 17, b5 20), white 0xFFFF */
            let words: [u16; 4] = [0xF800, 0x07E0, 0x1234, 0xFFFF];
            for (i, w) in words.iter().enumerate() {
                s.render(i * 2, &w.to_le_bytes());
            }
            let out = read(0, 0, 2, 2);
            // SAFETY: plain GL call on the (current) extra context.
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
            /* bottom row first; 2 -> 16, 17 -> 69, 20 -> 165 */
            assert_eq!(&out[..8], [[16, 69, 165, 255], [255, 255, 255, 255]].concat());
            assert_eq!(&out[8..], [[255, 0, 0, 255], [0, 255, 0, 255]].concat());
        }
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn read_pixels_rgb565_window_clipped_read_keeps_outside_pixels() {
        let gl = TestGl::new(8, 8);
        let s = SurfaceCtx::new(&gl, 2, 1, GLES_SURF_RGB565, 2 * 2);
        s.render(0, &[0x1F, 0x00, 0x00, 0xF8]); /* blue, red */
        let out = read(1, 0, 2, 1);
        assert_eq!(out, [[255, 0, 0, 255], [SENTINEL; 4]].concat());
    }

    #[test]
    fn read_pixels_xrgb8888_window_swaps_to_rgba_and_forces_alpha() {
        let gl = TestGl::new(8, 8);
        let s = SurfaceCtx::new(&gl, 2, 2, GLES_SURF_XRGB8888, 2 * 2 * 4);
        /* memory order B,G,R,X; top row then bottom row */
        s.render(0, &[1, 2, 3, 0, 4, 5, 6, 0x55, 7, 8, 9, 0, 10, 11, 12, 0xAA]);
        let out = read(0, 0, 2, 2);
        // SAFETY: plain GL call on the (current) extra context.
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
        assert_eq!(&out[..8], [[9, 8, 7, 255], [12, 11, 10, 255]].concat(), "the bottom row comes first");
        assert_eq!(&out[8..], [[3, 2, 1, 255], [6, 5, 4, 255]].concat());
    }

    #[test]
    fn read_pixels_fbo_on_an_xrgb_window_context_reads_plain_rgba() {
        /* the window's format only applies to framebuffer 0 */
        let gl = TestGl::new(8, 8);
        let _s = SurfaceCtx::new(&gl, 2, 2, GLES_SURF_XRGB8888, 2 * 2 * 4);
        bound_fb();
        let tex = tex_with_storage(2, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        // SAFETY: a by-value read of the current-context pointer; the context is live.
        let mem = unsafe { (*context::gl_ctx).objs().textures[tex as usize].mem };
        fill_rgba(&mem, 2, 2, 2);
        let out = read(0, 0, 2, 2);
        assert_eq!(out, [px(0, 0), px(1, 0), px(0, 1), px(1, 1)].concat());
        // SAFETY: the texture is the extra context's; delete it before that context goes.
        unsafe { glDeleteTextures(1, &tex) };
    }

    #[test]
    fn read_pixels_flushes_and_finishes_before_reading() {
        let gl = TestGl::new(4, 4);
        let out = read(0, 0, 1, 1);
        assert_eq!(out.len(), 4);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
        assert!(gl.draws().is_empty(), "no clear was pending");
    }

    #[test]
    fn read_pixels_emits_a_pending_clear_first() {
        let gl = TestGl::new(4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        assert!(gl.draws().is_empty(), "the clear waits for a draw");
        read(0, 0, 1, 1);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        let d = &draws[0].draw;
        assert!(d.do_clear);
        assert_eq!(d.clear_rgba, [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(d.index_count, 0, "a clear-only kick");
        assert_eq!(d.target_gpu, gl.ctx().default_surface.gpu);
        assert_eq!((gl.flushes(), gl.finishes()), (1, 1));
        /* the clear is consumed */
        read(0, 0, 1, 1);
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn read_pixels_negative_size_is_invalid_value() {
        let gl = TestGl::new(4, 4);
        let mut out = [SENTINEL; 16];
        // SAFETY: nothing is written on error.
        unsafe {
            glReadPixels(0, 0, -1, 1, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glReadPixels(0, 0, 1, -1, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(out.iter().all(|&b| b == SENTINEL));
        assert_eq!(gl.flushes(), 0);
    }

    #[test]
    fn read_pixels_unknown_format_or_type_is_invalid_enum() {
        let gl = TestGl::new(4, 4);
        let mut out = [SENTINEL; 16];
        // SAFETY: nothing is written on error.
        unsafe {
            glReadPixels(0, 0, 1, 1, GL_DEPTH_COMPONENT16, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_TEXTURE_2D, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        assert!(out.iter().all(|&b| b == SENTINEL));
        assert_eq!(gl.flushes(), 0);
    }

    #[test]
    fn read_pixels_unsupported_format_type_pair_is_invalid_operation() {
        let gl = TestGl::new(4, 4);
        let mut out = [SENTINEL; 16];
        // SAFETY: nothing is written on error.
        unsafe {
            glReadPixels(0, 0, 1, 1, GL_RGB, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, out.as_mut_ptr() as *mut c_void);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert!(out.iter().all(|&b| b == SENTINEL));
    }

    #[test]
    fn read_pixels_null_or_empty_is_a_no_op_without_a_flush() {
        let gl = TestGl::new(4, 4);
        let mut out = [SENTINEL; 4];
        // SAFETY: null is tolerated; an empty rectangle writes nothing.
        unsafe {
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null_mut());
            glReadPixels(0, 0, 0, 1, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
            glReadPixels(0, 0, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(out, [SENTINEL; 4]);
        assert_eq!((gl.flushes(), gl.finishes()), (0, 0));
    }

    #[test]
    fn read_pixels_incomplete_framebuffer_is_invalid_framebuffer_operation() {
        let gl = TestGl::new(4, 4);
        bound_fb();
        let out = read(0, 0, 1, 1);
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        assert_eq!(out, [SENTINEL; 4]);
        assert_eq!(gl.flushes(), 0);
    }

    /* ---- external (EGLImage) renderbuffer storage ------------------------------------------------------ */

    fn external_mem(bytes: usize) -> OsalGpuMem {
        let mut mem = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(HEAP_DATA, bytes, OSAL_MEM_DATA, &mut mem), 0);
        mem
    }

    #[test]
    fn attach_external_makes_the_bound_renderbuffer_an_rgba_target() {
        let gl = TestGl::new(8, 8);
        let mut ext = external_mem(4 * 2 * 4);
        let rb = gen_rb();
        // SAFETY: plain GL call.
        unsafe { glBindRenderbuffer(GL_RENDERBUFFER, rb) };
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 4, 2), 0);
        {
            let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
            assert!(r.external);
            assert_eq!((r.mem.gpu, r.internalformat, r.width, r.height), (ext.gpu, GL_RGBA, 4, 2));
        }
        bound_fb();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let (mem, w, h, _) = fb_target(gl.ctx()).unwrap();
        assert_eq!((mem.gpu, w, h), (ext.gpu, 4, 2));
        // SAFETY: one readable name.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        osal::gpu_free(&mut ext);
    }

    #[test]
    fn attach_external_rejects_empty_sizes_and_no_binding() {
        let gl = TestGl::new(8, 8);
        let mut ext = external_mem(64);
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 4, 4), -1, "no renderbuffer bound");
        let rb = rb_with_storage(GL_RGBA4, 2, 2);
        let own = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 0, 4), -1);
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 4, 0), -1);
        let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
        assert_eq!((r.mem.gpu, r.external, r.width), (own, false, 2), "a rejected attach changes nothing");
        osal::gpu_free(&mut ext);
    }

    #[test]
    fn attach_external_releases_owned_storage_but_never_the_external_one() {
        let gl = TestGl::new(8, 8);
        let mut ext = external_mem(4 * 4 * 4);
        let mut ext2 = external_mem(4 * 4 * 4);
        let rb = rb_with_storage(GL_RGBA4, 4, 4);
        let own = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        gl.mem_events();

        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 4, 4), 0);
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu: own }], "the renderbuffer's own storage goes");

        /* replaced by another external buffer: the first is not freed */
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext2, 4, 4), 0);
        assert!(gl.mem_events().is_empty());

        /* glRenderbufferStorage gives it its own storage again, leaving the image's alone */
        // SAFETY: plain GL call.
        unsafe { glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 2, 2) };
        let new = gl.ctx().objs().renderbuffers[rb as usize].mem.gpu;
        assert!(!gl.ctx().objs().renderbuffers[rb as usize].external);
        assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: 2 * 2 * 4, gpu: new }]);

        /* deleting while external frees nothing */
        assert_eq!(renderbuffer_attach_external(gl.ctx(), &ext, 4, 4), 0);
        gl.mem_events();
        // SAFETY: one readable name.
        unsafe { glDeleteRenderbuffers(1, &rb) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.gpu_bytes(ext.gpu, 4).len(), 4, "the external buffer is still allocated");

        osal::gpu_free(&mut ext);
        osal::gpu_free(&mut ext2);
        assert_eq!(gl.live_allocs(), 1);
    }

    #[test]
    fn destroying_the_context_leaves_external_renderbuffer_storage_alone() {
        let gl = TestGl::new(8, 8);
        let mut ext = external_mem(4 * 4 * 4);
        {
            let _s = SurfaceCtx::new(&gl, 2, 2, GLES_SURF_RGBA8888, 2 * 2 * 4);
            let rb = gen_rb();
            // SAFETY: plain GL call; the extra context is current.
            unsafe {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                assert_eq!(gl_renderbuffer_attach_external(&ext, 4, 4, core::ptr::null_mut()), 0);
            }
            gl.mem_events();
        }
        let freed_ext = gl.take_faults().is_empty() && gl.mem_events().contains(&MemEvent::Free { gpu: ext.gpu });
        osal::gpu_free(&mut ext);
        /* the mock reports the owner's free as a double free here */
        assert_eq!(gl.live_allocs(), 1);
        assert!(!freed_ext, "the context released storage it does not own");
    }

    #[test]
    fn c_abi_attach_external_needs_a_descriptor() {
        let gl = TestGl::new(8, 8);
        rb_with_storage(GL_RGBA4, 2, 2);
        // SAFETY: null is tolerated.
        assert_eq!(unsafe { gl_renderbuffer_attach_external(core::ptr::null(), 4, 4, core::ptr::null_mut()) }, -1);
        assert_eq!(gl.live_allocs(), 2);
    }

    /* ---- OpenGL ES 3.0 entry points ------------------------------------------------------------------------ */

    /// More samples than GL_SAMPLES (1) is GL_INVALID_OPERATION and
    /// allocates nothing (ES 3.0 4.4.2). Until 2026-09-24 this test pinned
    /// the opposite: single-sampled storage handed out with a log line, so
    /// an application that asked for four samples was told it had them
    /// (dEQP-GLES3 negative_api.buffer.renderbuffer_storage_multisample).
    #[test]
    fn storage_multisample_with_more_than_one_sample_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, GL_RGBA8, 5, 3);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        let r = &gl.ctx().objs_ref().renderbuffers[rb as usize];
        assert_eq!((r.internalformat, r.width, r.height), (0, 0, 0), "nothing allocated");
        assert_eq!(gl.mem_events(), []);
    }

    #[test]
    fn storage_multisample_with_zero_or_one_sample_is_plain_storage() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: single-threaded (TestGl serialises); plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_DEPTH24_STENCIL8, 4, 4);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_DEPTH24_STENCIL8, 4, 4);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(rb_param(GL_RENDERBUFFER_INTERNAL_FORMAT), GL_DEPTH24_STENCIL8 as GLint);
        assert!(gl.logs().iter().all(|l| !l.contains("single-sampled")));
    }

    #[test]
    fn storage_multisample_errors() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, -1, GL_RGBA8, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "samples");
            assert_eq!(gl.ctx().objs().renderbuffers[rb as usize].width, 0);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_LUMINANCE, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "format");
            glRenderbufferStorageMultisample(GL_FRAMEBUFFER, 0, GL_RGBA8, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "target");
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 0, GL_RGBA8, 4097, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "size");
        }
        assert!(gl.mem_events().is_empty());
    }

    /// (The default framebuffer's attachments are GL_COLOR and GL_DEPTH, as
    /// ES 3.0 4.5.1 names them; this pinned GL_COLOR_ATTACHMENT0 accepted
    /// there until 2026-09-24, see `invalidate_framebuffer_names_its_attachments`.)
    #[test]
    fn invalidate_framebuffer_validates_and_does_nothing() {
        let gl = TestGl::new(8, 8);
        let atts = [GL_COLOR, GL_DEPTH];
        // SAFETY: two readable enums.
        unsafe {
            for target in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
                glInvalidateFramebuffer(target, 2, atts.as_ptr());
                assert_eq!(gl.error(), GL_NO_ERROR);
            }
            glInvalidateFramebuffer(GL_RENDERBUFFER, 2, atts.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glInvalidateFramebuffer(GL_FRAMEBUFFER, -1, atts.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert!(gl.draws().is_empty() && gl.mem_events().is_empty());
        assert_eq!(gl.flushes(), 0);
    }

    /// (GL_COLOR, the default framebuffer's name for its colour buffer; see
    /// `invalidate_framebuffer_names_its_attachments`.)
    #[test]
    fn invalidate_sub_framebuffer_validates_its_rectangle_too() {
        let gl = TestGl::new(8, 8);
        let atts = [GL_COLOR];
        // SAFETY: one readable enum.
        unsafe {
            glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, atts.as_ptr(), 0, 0, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, atts.as_ptr(), 0, 0, -1, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 1, atts.as_ptr(), 0, 0, 4, -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glInvalidateSubFramebuffer(GL_TEXTURE_2D, 1, atts.as_ptr(), 0, 0, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glInvalidateSubFramebuffer(GL_FRAMEBUFFER, -1, atts.as_ptr(), 0, 0, 4, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn draw_buffers_on_the_default_framebuffer_accepts_back_or_none() {
        let gl = TestGl::new(8, 8);
        // SAFETY: one readable enum per call.
        unsafe {
            glDrawBuffers(1, [GL_BACK].as_ptr());
            glDrawBuffers(1, [GL_NONE].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            /* ES 3.0 4.2.1: the window with n other than 1, 0 included */
            glDrawBuffers(0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn draw_buffers_on_an_fbo_accepts_color_attachment0_or_none() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        // SAFETY: one readable enum per call.
        unsafe {
            glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr());
            glDrawBuffers(1, [GL_NONE].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawBuffers(1, [GL_BACK].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDrawBuffers(1, [GL_COLOR_ATTACHMENT1].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn draw_buffers_count_past_the_attachments_is_invalid_value() {
        /* GL_MAX_DRAW_BUFFERS, per GPU (context::MAX_DRAW_BUFFERS) */
        let gl = TestGl::new(8, 8);
        let bufs = [GL_NONE; 8];
        // SAFETY: eight readable enums.
        unsafe {
            glDrawBuffers(-1, bufs.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            /* the window has one buffer whatever the cap is: n other than
             * 1 is its own error, before the count's */
            glDrawBuffers(context::MAX_DRAW_BUFFERS as GLsizei + 1, bufs.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            bound_fb();
            glDrawBuffers(context::MAX_DRAW_BUFFERS as GLsizei + 1, bufs.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /// GL_DRAW_BUFFERi of the bound draw framebuffer.
    fn draw_buffer_q(i: GLenum) -> GLenum {
        let mut v: GLint = -1;
        // SAFETY: one GLint out.
        unsafe { crate::gl::state::glGetIntegerv(GL_DRAW_BUFFER0 + i, &mut v) };
        v as GLenum
    }

    /// n > 0 with no array, and n < 0 (review R-22): GL_INVALID_VALUE, and
    /// the draw buffers stay as they were. The window accepted (1, NULL) and
    /// an object took (n, NULL) as "every output GL_NONE".
    #[test]
    fn draw_buffers_with_no_array_or_a_negative_count_changes_nothing() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; every non-null array holds n enums.
        unsafe {
            glDrawBuffers(1, [GL_NONE].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawBuffers(1, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "the window, n = 1, no array");
            assert_eq!(draw_buffer_q(0), GL_NONE, "unchanged");
            glDrawBuffers(-1, [GL_BACK].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(draw_buffer_q(0), GL_NONE, "unchanged");
            glDrawBuffers(0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the window with n = 0: no array needed");

            /* an object: buffer 0 writes attachment 0, which is what the
             * old reading of (n, NULL) -- every output GL_NONE -- changed */
            bound_fb();
            glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDrawBuffers(1, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an object, n = 1, no array");
            glDrawBuffers(-3, [GL_NONE; 3].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(draw_buffer_q(0), GL_COLOR_ATTACHMENT0, "unchanged");
        }
    }

    /// ES 3.0 4.2.1: "Calling DrawBuffers with 0 as the value of n is
    /// equivalent to setting all the draw buffers to NONE." It read as "never
    /// called" (output i to attachment i), so every attachment was written.
    #[test]
    fn draw_buffers_with_n_zero_sets_every_draw_buffer_to_none() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(8, 8));
        attach_tex(GL_COLOR_ATTACHMENT1, tex_with_storage(8, 8));
        /* GL_DRAW_BUFFER1 exists where GL_MAX_DRAW_BUFFERS is 2 or more (not
         * the GC7000 with its MRT switch off) */
        let two = context::max_draw_buffers() >= 2;
        assert_eq!(draw_buffer_q(0), GL_COLOR_ATTACHMENT0);
        assert!(fb_target_n(gl.ctx(), 1).is_some(), "never set: output 1 writes attachment 1");
        if two {
            assert_eq!(draw_buffer_q(1), GL_COLOR_ATTACHMENT1);
        }
        // SAFETY: n = 0 reads nothing.
        unsafe { glDrawBuffers(0, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(draw_buffer_q(0), GL_NONE);
        assert!(fb_target_n(gl.ctx(), 1).is_none(), "attachment 1 is not written");
        if two {
            assert_eq!(draw_buffer_q(1), GL_NONE);
        }
        /* fewer than the maximum: the rest are GL_NONE */
        // SAFETY: one readable enum.
        unsafe { glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr()) };
        assert_eq!(draw_buffer_q(0), GL_COLOR_ATTACHMENT0);
        assert!(fb_target_n(gl.ctx(), 1).is_none());
        if two {
            assert_eq!(draw_buffer_q(1), GL_NONE);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The default framebuffer's draw buffer is state (ES 3.0 4.2.1): GL_NONE
    /// is answered by GL_DRAW_BUFFER0 and kept per context.
    #[test]
    fn the_default_framebuffer_keeps_its_draw_buffer() {
        let gl = TestGl::new(8, 8);
        assert_eq!(draw_buffer_q(0), GL_BACK);
        // SAFETY: one readable enum per call.
        unsafe { glDrawBuffers(1, [GL_NONE].as_ptr()) };
        assert_eq!(draw_buffer_q(0), GL_NONE);
        assert!(colour_writes_off(gl.ctx()));
        /* an object bound over it has its own; the window's comes back */
        let fb = bound_fb();
        assert_eq!(draw_buffer_q(0), GL_COLOR_ATTACHMENT0);
        assert!(!colour_writes_off(gl.ctx()));
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(draw_buffer_q(0), GL_NONE);
            glDrawBuffers(1, [GL_BACK].as_ptr());
            assert_eq!(draw_buffer_q(0), GL_BACK);
            glDeleteFramebuffers(1, &fb);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// dEQP-GLES3 negative_api.buffer.draw_buffers checks a value that is no
    /// draw buffer at all before the count: {COLOR_ATTACHMENT0,
    /// DEPTH_ATTACHMENT} on an object is GL_INVALID_ENUM however many draw
    /// buffers there are, and the window given two buffers is
    /// GL_INVALID_OPERATION. Both were GL_INVALID_VALUE (n past a cap of 1)
    /// until 2026-09-24.
    #[test]
    fn draw_buffers_checks_the_enums_before_the_count() {
        let gl = TestGl::new(8, 8);
        // SAFETY: two readable enums per call.
        unsafe {
            glDrawBuffers(2, [GL_NONE, GL_BACK].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the window, n = 2");
            bound_fb();
            glDrawBuffers(2, [GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glDrawBuffers(1, [GL_BACK].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "BACK on an object");
        }
    }

    /// glInvalidateFramebuffer names its attachments (ES 3.0 4.5.1): the
    /// default framebuffer takes GL_COLOR/DEPTH/STENCIL, an object the
    /// attachment points, and GL_COLOR_ATTACHMENTm at or past
    /// GL_MAX_COLOR_ATTACHMENTS is GL_INVALID_OPERATION (dEQP-GLES3
    /// negative_api.buffer.invalidate_framebuffer). Nothing was checked
    /// until 2026-09-24.
    #[test]
    fn invalidate_framebuffer_names_its_attachments() {
        let gl = TestGl::new(8, 8);
        let too_far = GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum;
        // SAFETY: plain GL calls; each list is one enum.
        unsafe {
            glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, [GL_COLOR].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, [GL_COLOR_ATTACHMENT0].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not an attachment of the default framebuffer");
            glInvalidateFramebuffer(GL_FRAMEBUFFER, -1, [GL_COLOR].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        bound_fb();
        // SAFETY: as above.
        unsafe {
            glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, [GL_COLOR_ATTACHMENT0].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, [too_far].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glInvalidateFramebuffer(GL_FRAMEBUFFER, 1, [GL_COLOR].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not an attachment point");
        }
    }

    /// glFramebufferTextureLayer takes a 3D or a 2D array texture (ES 3.0
    /// 4.4.2; dEQP-GLES3 negative_api.buffer.framebuffer_texture_layer
    /// hands a 2D one and expects GL_INVALID_OPERATION); a layer past the
    /// limit is GL_INVALID_VALUE.
    #[test]
    fn framebuffer_texture_layer_takes_3d_and_array_textures_only() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex2d = tex_with_storage(4, 4);
        let mut arr: GLuint = 0;
        // SAFETY: plain GL calls; `arr` is one name.
        unsafe {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex2d, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a 2D texture has no layers");
            glGenTextures(1, &mut arr);
            glBindTexture(GL_TEXTURE_2D_ARRAY, arr);
            crate::gl::texture::glTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                GL_RGBA as GLint,
                4,
                4,
                2,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            assert_eq!(gl.error(), GL_NO_ERROR);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 1);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 256);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /// glRenderbufferStorageMultisample (ES 3.0 4.4.2), in dEQP's order
    /// (negative_api.buffer.renderbuffer_storage_multisample): the format
    /// before the sample count, and a count past the format's GL_SAMPLES (1)
    /// is GL_INVALID_OPERATION, not single-sampled storage handed out as if
    /// it were multisampled (until 2026-09-24).
    #[test]
    fn renderbuffer_storage_multisample_refuses_more_samples_than_one() {
        let gl = TestGl::new(8, 8);
        let rb = gen_rb();
        // SAFETY: plain GL calls.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 2, 0x1234, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "the format first");
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 2, GL_RGBA4, 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "two samples");
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA4, -1, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA4, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(rb_param(GL_RENDERBUFFER_WIDTH), 4);
    }

    /// glGetInternalformativ's argument rules (ES 3.0 6.1.15; dEQP-GLES3
    /// negative_api.state.get_internalformativ): GL_RENDERBUFFER only, a
    /// renderable format, the two pnames, a non-negative bufSize; one sample
    /// count, 1. Only the pname was checked until 2026-09-24.
    #[test]
    fn get_internalformativ_checks_its_arguments() {
        let gl = TestGl::new(8, 8);
        let mut v: GLint = -1;
        // SAFETY: plain GL calls; `v` is one GLint.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_NUM_SAMPLE_COUNTS, -1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, 0xffff_ffff, 1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetInternalformativ(GL_TEXTURE_2D, GL_RGBA8, GL_NUM_SAMPLE_COUNTS, 1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetInternalformativ(GL_RENDERBUFFER, GL_COMPRESSED_RGB8_ETC2, GL_NUM_SAMPLE_COUNTS, 1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, 1, &mut v);
            assert_eq!((v, gl.error()), (1, GL_NO_ERROR));
        }
    }

    #[test]
    fn read_buffer_follows_the_bound_framebuffer() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            glReadBuffer(GL_BACK);
            glReadBuffer(GL_NONE);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        bound_fb();
        // SAFETY: plain GL calls.
        unsafe {
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            glReadBuffer(GL_NONE);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glReadBuffer(GL_BACK);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn blit_framebuffer_is_invalid_operation_and_says_so_once() {
        let gl = TestGl::new(8, 8);
        // SAFETY: single-threaded (TestGl serialises); plain GL calls.
        unsafe {
            WARNED.get().blit = false;
            gl.logs();
            glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        let logs = gl.logs();
        assert_eq!(logs.iter().filter(|l| l.contains("glBlitFramebuffer: not implemented")).count(), 1, "{logs:?}");
        assert!(gl.draws().is_empty());
    }

    #[test]
    fn get_internalformat_reports_one_sample_count_of_one() {
        let gl = TestGl::new(8, 8);
        let mut v: [GLint; 2] = [-1, -1];
        // SAFETY: `v` has room for bufSize values.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_NUM_SAMPLE_COUNTS, 2, v.as_mut_ptr());
            assert_eq!(v, [1, -1]);
            v = [-1, -1];
            glGetInternalformativ(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, GL_SAMPLES, 1, v.as_mut_ptr());
            assert_eq!(v, [1, -1]);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_internalformat_bad_pname_and_empty_buffer() {
        let gl = TestGl::new(8, 8);
        let mut v: GLint = -1;
        // SAFETY: `v` is writable; bufSize 0 and null write nothing.
        unsafe {
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, 0, &mut v);
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_SAMPLES, 1, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(v, -1);
            glGetInternalformativ(GL_RENDERBUFFER, GL_RGBA8, GL_RENDERBUFFER_WIDTH, 1, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
        }
        assert_eq!(v, -1);
    }
    /* ---- level / face / layer targets and float targets (upstream 7e76640) ------------------------- */

    fn bind_new_texture(target: GLenum) -> GLuint {
        let mut t: GLuint = 0;
        // SAFETY: valid out pointer, plain GL calls.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(target, t);
        }
        t
    }

    #[test]
    fn half_to_float_decodes_every_class() {
        assert_eq!(half_to_float(0x0000), 0.0);
        assert_eq!(half_to_float(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(half_to_float(0x3C00), 1.0);
        assert_eq!(half_to_float(0xC000), -2.0);
        assert_eq!(half_to_float(0x7BFF), 65504.0);
        assert_eq!(half_to_float(0x7C00), f32::INFINITY);
        assert!(half_to_float(0x7E00).is_nan());
        /* subnormals are normalised, not dropped */
        assert_eq!(half_to_float(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_float(0x0200), 2f32.powi(-15));
    }

    #[test]
    fn cube_face_attachment_targets_its_rows() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new_texture(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL calls.
        unsafe { crate::gl::texture::glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 4, 4) };
        bound_fb();
        // SAFETY: plain GL call.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_NEGATIVE_Y, cube, 0)
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let c = gl.ctx();
        let (mem, w, h, off) = fb_target(c).unwrap();
        assert_eq!((mem.gpu, w, h), (c.objs().textures[cube as usize].mem.gpu, 4, 4));
        assert_eq!(off, 3 * 4 * 4 * 4, "face 3 of 4x4 RGBA8 faces");
        assert_eq!(fb_target_format(c), GLES_SURF_RGBA8888);
        let mut o = 0u32;
        let (mut tw, mut th) = (0u32, 0u32);
        // SAFETY: all three are writable.
        unsafe { gl_fb_target(&mut tw, &mut th, &mut o) };
        assert_eq!((tw, th, o), (4, 4, 48 * 4));
    }

    #[test]
    fn mip_level_attachment_targets_the_level() {
        let gl = TestGl::new(8, 8);
        let tex = bind_new_texture(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe { crate::gl::texture::glTexStorage2D(GL_TEXTURE_2D, 3, GL_RGBA8, 8, 4) };
        bound_fb();
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 2) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let (_, w, h, off) = fb_target(gl.ctx()).unwrap();
        assert_eq!((w, h, off), (2, 1, 8 * 4 * 4 + 4 * 2 * 4));
        // SAFETY: plain GL call.
        let mut v: GLint = -1;
        unsafe {
            glGetFramebufferAttachmentParameteriv(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL,
                &mut v,
            )
        };
        assert_eq!(v, 2);
        /* a level the storage does not hold is incomplete */
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 5) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    #[test]
    fn a_one_wide_level_of_more_than_one_row_is_not_a_render_target() {
        /* The TPU reads a 1-wide level of a PowerVR chain one texel a row;
         * the PBE writes a texel pair a row. Level 1 of a 2x4 (1x2) cannot
         * be both, so it is incomplete rather than drawn wrong. The 1x1
         * level is one row and the same either way. */
        let gl = TestGl::new(8, 8);
        let tex = bind_new_texture(GL_TEXTURE_2D);
        // SAFETY: plain GL calls.
        unsafe { crate::gl::texture::glTexStorage2D(GL_TEXTURE_2D, 3, GL_RGBA8, 2, 4) };
        bound_fb();
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 1) };
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "1x2: two one-texel rows");
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 2) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "1x1: one row");
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0) };
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "the base level is untouched");
        let _ = gl;
    }

    #[test]
    fn every_face_of_a_cube_maps_one_by_one_level_is_a_render_target() {
        /* Qt Quick 3D's IBL prefilter renders into every mip level of the
         * light-probe cube, face by face, through an FBO. Its last level has
         * 1x1 faces: one row each, so the multi-row 1-wide refusal must not
         * fire, and face f sits one texel after face f-1 (one-texel rows). */
        let gl = TestGl::new(8, 8);
        let tex = bind_new_texture(GL_TEXTURE_CUBE_MAP);
        // SAFETY: plain GL call.
        unsafe { crate::gl::texture::glTexStorage2D(GL_TEXTURE_CUBE_MAP, 3, GL_RGBA8, 4, 4) };
        bound_fb();
        let base = gl.ctx().objs_ref().textures[tex as usize].level_off[2];
        for f in 0..6u32 {
            // SAFETY: plain GL call.
            unsafe {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, tex, 2)
            };
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "face {f}, level 2 (1x1)");
            let (_, w, h, off) = fb_target(gl.ctx()).unwrap();
            assert_eq!((w, h, off), (1, 1, base + f * 4), "face {f}");
        }
    }

    #[test]
    fn a_float_renderbuffer_is_stored_in_its_own_surface() {
        /* GL_EXT_color_buffer_half_float names four sized formats; all four
         * have to be accepted and none of them is eight bits a channel. The
         * renderbuffer used to be RGBA8888 whatever was asked for, so a
         * GL_RGBA16F one held half the bytes the PBE writes. */
        let gl = TestGl::new(8, 8);
        bound_fb();
        for (format, surf, bpp) in [
            (GL_RGBA16F, GLES_SURF_RGBA16F, 8u32),
            (GL_RGB16F, GLES_SURF_RGBA16F, 8),
            (GL_RG16F, GLES_SURF_RGBA16F, 8),
            (GL_R16F, GLES_SURF_R16F, 2),
            (GL_R32F, GLES_SURF_R32F, 4),
            (GL_RGBA8, GLES_SURF_RGBA8888, 4),
            (GL_RGB565, GLES_SURF_RGBA8888, 4),
        ] {
            let rb = rb_with_storage(format, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "glRenderbufferStorage(0x{format:x})");
            let mem = gl.ctx().objs().renderbuffers[rb as usize].mem;
            assert_eq!(mem.size, (tex_pitch(4) * 4 * bpp) as usize, "0x{format:x} at {bpp} bytes a pixel");
            attach_rb(GL_COLOR_ATTACHMENT0, rb);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "0x{format:x} is colour-renderable");
            assert_eq!(fb_target_format(gl.ctx()), surf, "0x{format:x} renders as its own surface");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn alpha_bits_follow_the_attachment_format() {
        /* The storage is RGBA8 for every one of these, so the number cannot
         * come from the storage: it is the GL format the application named. */
        let gl = TestGl::new(8, 8);
        bound_fb();
        assert_eq!(fb_target_alpha_bits(gl.ctx()), 8, "the default surface");

        let rgba = tex_with_storage(4, 4);
        attach_tex(GL_COLOR_ATTACHMENT0, rgba);
        assert_eq!(fb_target_alpha_bits(gl.ctx()), 8, "a GL_RGBA texture");

        let rgb = bind_new_texture(GL_TEXTURE_2D);
        // SAFETY: null pixels only allocate.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, 4, 4, 0, GL_RGB, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        attach_tex(GL_COLOR_ATTACHMENT0, rgb);
        assert_eq!(fb_target_alpha_bits(gl.ctx()), 0, "a GL_RGB texture reads alpha 1");

        for (format, bits) in [(GL_RGBA8, 8), (GL_RGBA4, 4), (GL_RGB5_A1, 1), (GL_RGB565, 0), (GL_RGB8, 0)] {
            let rb = rb_with_storage(format, 4, 4);
            attach_rb(GL_COLOR_ATTACHMENT0, rb);
            assert_eq!(fb_target_alpha_bits(gl.ctx()), bits, "renderbuffer 0x{format:x}");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn quantise_unorm_rounds_to_what_the_channel_holds() {
        assert_eq!(quantise_unorm(0.0, 0), 1.0, "no channel: it reads 1");
        assert_eq!(quantise_unorm(0.37, 0), 1.0);
        assert_eq!(quantise_unorm(0.37, 1), 0.0, "one bit: below half is 0");
        assert_eq!(quantise_unorm(0.6, 1), 1.0);
        assert_eq!(quantise_unorm(1.0 / 3.0, 4), 5.0 / 15.0, "four bits");
        assert_eq!(quantise_unorm(0.37, 8), 0.37, "eight bits: unchanged");
        assert_eq!(quantise_unorm(-1.0, 4), 0.0, "clamped");
        assert_eq!(quantise_unorm(2.0, 4), 1.0);
    }

    #[test]
    fn texture_layer_attachment_of_a_float_array() {
        let gl = TestGl::new(8, 8);
        let arr = bind_new_texture(GL_TEXTURE_2D_ARRAY);
        // SAFETY: plain GL calls.
        unsafe { crate::gl::texture::glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA16F, 3, 2, 4) };
        let fb = bound_fb();
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 2);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        let a = gl.ctx().framebuffers[fb as usize].color;
        assert_eq!((a.type_, a.name, a.level, a.layer), (GL_TEXTURE, arr, 0, 2));
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let c = gl.ctx();
        let (_, w, h, off) = fb_target(c).unwrap();
        /* layer 2 of 3x2 layers at the pitch of 4, 8-byte texels */
        assert_eq!((w, h, off), (3, 2, 2 * 2 * 4 * 8));
        assert_eq!(fb_target_format(c), GLES_SURF_RGBA16F);
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "a layer past the array is accepted ...");
        }
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "... and incomplete");
        // SAFETY: plain GL calls.
        unsafe {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 256);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, -1, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, 99, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, arr, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glFramebufferTextureLayer(GL_RENDERBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, arr, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the default framebuffer");
        }
    }

    #[test]
    fn float_target_formats_follow_the_colour_texture() {
        let gl = TestGl::new(8, 8);
        for (internal, surf) in [(GL_R16F, GLES_SURF_R16F), (GL_R32F, GLES_SURF_R32F), (GL_RGBA8, GLES_SURF_RGBA8888)] {
            let tex = bind_new_texture(GL_TEXTURE_2D);
            // SAFETY: plain GL calls.
            unsafe { crate::gl::texture::glTexStorage2D(GL_TEXTURE_2D, 1, internal, 2, 2) };
            bound_fb();
            attach_tex(GL_COLOR_ATTACHMENT0, tex);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
            assert_eq!(fb_target_format(gl.ctx()), surf, "0x{internal:x}");
        }
        // SAFETY: plain GL call.
        assert_eq!(unsafe { gl_fb_target_format() }, GLES_SURF_RGBA8888);
    }

    #[test]
    fn compressed_and_external_rgb565_textures_are_not_render_targets() {
        let gl = TestGl::new(8, 8);
        let tex = bind_new_texture(GL_TEXTURE_2D);
        let blocks = [0u8; 8];
        // SAFETY: one 4x4 ETC2 block.
        unsafe {
            crate::gl::texture::glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_COMPRESSED_RGB8_ETC2,
                4,
                4,
                0,
                8,
                blocks.as_ptr() as *const c_void,
            )
        };
        assert_eq!(gl.error(), GL_NO_ERROR);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);

        let ext_tex = bind_new_texture(GL_TEXTURE_2D);
        let mut ext = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(context::heap_data(), 8, OSAL_MEM_DATA, &mut ext), 0);
        assert_eq!(crate::gl::texture::texture_attach_external(gl.ctx(), &ext, 2, 2, GLES_SURF_RGB565), 0);
        attach_tex(GL_COLOR_ATTACHMENT0, ext_tex);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
        // SAFETY: plain GL call.
        unsafe { glDeleteTextures(1, &ext_tex) };
        osal::gpu_free(&mut ext);
    }

    #[test]
    fn an_incomplete_target_is_logged() {
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        gl.logs();
        assert!(fb_target(gl.ctx()).is_none());
        assert_eq!(gl.logs(), [format!("gl_fb_target: framebuffer {fb} incomplete (status 0x8cd7)")]);
        /* a name that was never created is bound without an object: unknown */
        gl.ctx().bound_framebuffer = 31;
        gl.ctx().framebuffers[31].used = false;
        assert!(fb_target(gl.ctx()).is_none());
        assert_eq!(gl.logs(), ["gl_fb_target: framebuffer 31 unknown (status 0x0)"]);
        gl.ctx().bound_framebuffer = 0;
    }

    #[test]
    /* The sized float internal formats are ES 3.0's; the report-es2
     * surface refuses them at glTexImage2D, so this asks about a
     * capability that build does not offer. */
    #[cfg(not(feature = "report-es2"))]
    fn read_pixels_of_a_float_target_as_float_and_clamped_bytes() {
        let gl = TestGl::new(8, 8);
        let tex = bind_new_texture(GL_TEXTURE_2D);
        let texels = [0.5f32, 2.0, -1.0, 1.0, 0.25, 0.0, 1.0, 0.5];
        // SAFETY: two RGBA float texels.
        unsafe {
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA16F as GLint,
                2,
                1,
                0,
                GL_RGBA,
                GL_FLOAT,
                texels.as_ptr() as *const c_void,
            )
        };
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        let mut f = [0f32; 8];
        // SAFETY: room for two RGBA float pixels.
        unsafe { glReadPixels(0, 0, 2, 1, GL_RGBA, GL_FLOAT, f.as_mut_ptr() as *mut c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(f, texels);
        let mut b = [0u8; 8];
        // SAFETY: room for two RGBA pixels.
        unsafe { glReadPixels(0, 0, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, b.as_mut_ptr() as *mut c_void) };
        assert_eq!(b, [128, 255, 0, 255, 64, 0, 255, 128]);

        /* a float read of an RGBA8 target is not an allowed pair */
        // SAFETY: plain GL call; nothing is written on the error.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_FLOAT, f.as_mut_ptr() as *mut c_void);
        }
        /* known enums in a combination this target does not take */
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    /* The sized float internal formats are ES 3.0's; the report-es2
     * surface refuses them at glTexImage2D, so this asks about a
     * capability that build does not offer. */
    #[cfg(not(feature = "report-es2"))]
    fn read_pixels_of_r16f_and_r32f_targets_fill_rgba() {
        let _gl = TestGl::new(8, 8);
        for internal in [GL_R16F, GL_R32F] {
            let tex = bind_new_texture(GL_TEXTURE_2D);
            let v = [0.75f32];
            // SAFETY: one RED float texel.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    internal as GLint,
                    1,
                    1,
                    0,
                    GL_RED,
                    GL_FLOAT,
                    v.as_ptr() as *const c_void,
                )
            };
            bound_fb();
            attach_tex(GL_COLOR_ATTACHMENT0, tex);
            let mut f = [9f32; 4];
            // SAFETY: room for one RGBA float pixel.
            unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA, GL_FLOAT, f.as_mut_ptr() as *mut c_void) };
            assert_eq!(f, [0.75, 0.0, 0.0, 1.0], "0x{internal:x}");
        }
    }

    #[test]
    fn read_pixels_of_a_cube_face_reads_that_face() {
        let gl = TestGl::new(8, 8);
        let cube = bind_new_texture(GL_TEXTURE_CUBE_MAP);
        for f in 0..6u32 {
            let px = [f as u8 * 10; 4];
            // SAFETY: one RGBA texel.
            unsafe {
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    1,
                    1,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                )
            };
        }
        bound_fb();
        // SAFETY: plain GL calls; room for one pixel.
        let mut out = [0u8; 4];
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_Z, cube, 0);
            glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(out, [40; 4]);
    }

    /* ---- read-back and the cache ------------------------------------------------------------------------ */

    #[test]
    fn read_pixels_invalidates_the_whole_target_after_the_kicks_completed() {
        let gl = TestGl::new(4, 3);
        let gpu = gl.ctx().default_surface.gpu;
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        fill_rgba(&gl.ctx().default_surface, 4, 4, 3);
        let out = read(1, 1, 2, 1);
        assert_eq!(gl.cache_ops(), [CacheOp::Invd { gpu, off: 0, len: 4 * 3 * 4 }]);
        assert!(!gl.kick_pending(1), "the clear kick completed before the read");
        assert_eq!(out, [px(1, 1), px(2, 1)].concat());
        /* the mock would have reported an invalidate of a target a kick still renders */
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn read_pixels_of_an_fbo_invalidates_the_attachment_at_its_pitch() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(5, 3);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let m = gl.ctx().objs().textures[tex as usize].mem;
        fill_rgba(&m, 6, 5, 3);
        gl.cache_ops(); /* the clear of the texture's fresh storage */
        read(0, 0, 1, 1);
        assert_eq!(gl.cache_ops(), [CacheOp::Invd { gpu: m.gpu, off: 0, len: 6 * 3 * 4 }]);
    }

    #[test]
    fn read_pixels_without_a_cpu_view_reads_through_gpu_read() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        bound_fb();
        let tex = tex_with_storage(5, 3);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let m = gl.ctx().objs().textures[tex as usize].mem;
        assert!(m.cpu.is_null());
        fill_rgba(&m, 6, 5, 3);
        let out = read(-1, 1, 7, 2); /* clipped on both sides */
        assert_eq!(gl.error(), GL_NO_ERROR);
        let s = [SENTINEL; 4];
        assert_eq!(&out[..28], [s, px(0, 1), px(1, 1), px(2, 1), px(3, 1), px(4, 1), s].concat());
        assert_eq!(&out[28..], [s, px(0, 2), px(1, 2), px(2, 2), px(3, 2), px(4, 2), s].concat());
        assert!(gl.cache_ops().is_empty());
    }

    #[test]
    fn read_pixels_starts_each_row_at_the_pack_alignment() {
        /* `dEQP-GLES2.functional.read_pixels.{rgba_ubyte_align_8,choose_align_8}`
         * from the 8b5428a sweep. GL_PACK_ALIGNMENT rounds the distance
         * between rows up (ES 2.0 4.3.1); the pixels themselves are never
         * padded. With RGBA/UNSIGNED_BYTE a row of w pixels is 4w bytes,
         * already a multiple of 1, 2 and 4 -- so alignment 8 with an ODD
         * width is the only combination that pads anything, which is why
         * those two cases fail alone and the align_1/2/4 ones pass. */
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 4, 3, 2);

        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_PACK_ALIGNMENT, 8) };
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* 3 RGBA pixels is 12 bytes, so each row occupies 16. */
        let mut out = std::vec![SENTINEL; 2 * 16];
        // SAFETY: two rows at the 16-byte stride.
        unsafe { glReadPixels(0, 0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);

        for (j, row) in out.chunks(16).enumerate() {
            assert_eq!(&row[..12], [px(0, j), px(1, j), px(2, j)].concat(), "row {j}");
            assert_eq!(&row[12..], [SENTINEL; 4], "row {j}'s padding belongs to the caller");
        }

        /* and the default, 4, is still tight: no gap, nothing skipped */
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_PACK_ALIGNMENT, 4) };
        let tight = read(0, 0, 3, 2);
        assert_eq!(tight.len(), 24);
        assert_eq!(&tight[..12], [px(0, 0), px(1, 0), px(2, 0)].concat());
        assert_eq!(&tight[12..], [px(0, 1), px(1, 1), px(2, 1)].concat());
    }

    /// ES 3.0 4.4.1: glReadPixels reads the READ framebuffer while the DRAW
    /// one is another; the read framebuffer's attachments and status answer
    /// through GL_READ_FRAMEBUFFER, and glReadBuffer(GL_NONE) is recorded
    /// per framebuffer. dEQP-GLES3 state_query.integers.framebuffer_binding /
    /// read_buffer read one binding and a fixed read buffer until 2026-09-24.
    #[test]
    fn the_read_framebuffer_is_its_own_binding() {
        let gl = TestGl::new(8, 8);
        let src = bound_fb();
        let tex = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 4, 3, 2);
        let dst = bound_fb(); /* the draw framebuffer now, empty */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src);
            assert_eq!(glCheckFramebufferStatus(GL_READ_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            assert_ne!(glCheckFramebufferStatus(GL_DRAW_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        assert_eq!(read(0, 0, 3, 1), [px(0, 0), px(1, 0), px(2, 0)].concat(), "read from the read framebuffer");
        assert_eq!(gl.ctx().bound_framebuffer, dst, "the draw binding is where it was");
        let mut v: GLint = -1;
        // SAFETY: one writable GLint.
        unsafe {
            glGetFramebufferAttachmentParameteriv(
                GL_READ_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
                &mut v,
            );
            glReadBuffer(GL_NONE);
        }
        assert_eq!((gl.error(), v), (GL_NO_ERROR, tex as GLint));
        assert!(gl.ctx().framebuffers[src as usize].read_none, "recorded on the read framebuffer");
        read(0, 0, 1, 1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "no read buffer to read");
        assert!(!gl.ctx().framebuffers[dst as usize].read_none);
    }

    /// glReadBuffer(GL_COLOR_ATTACHMENTn) makes glReadPixels read attachment
    /// n (gl_test 106 mrt3 read attachment 0 for every n: the call was
    /// refused), and the ES 3.0 4.3.1 errors hold (dEQP-GLES3
    /// negative_api.buffer.read_buffer).
    #[test]
    fn read_buffer_selects_the_attachment_read() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let n = CAP_COLOR_ATTACHMENTS as GLenum - 1;
        let first = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, first);
        let last = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0 + n, last);
        fill_rgba(&gl.ctx().objs_ref().textures[last as usize].mem, 4, 3, 2);
        // SAFETY: plain GL calls.
        unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0 + n) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(read(0, 0, 3, 1), [px(0, 0), px(1, 0), px(2, 0)].concat(), "attachment n's pixels");
        assert_eq!(gl.ctx().framebuffers[gl.ctx().bound_framebuffer as usize].color.name, first, "put back");
        // SAFETY: plain GL calls.
        unsafe {
            glReadBuffer(GL_COLOR_ATTACHMENT0 + CAP_COLOR_ATTACHMENTS as GLenum);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "past GL_MAX_COLOR_ATTACHMENTS");
            glReadBuffer(GL_BACK);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "GL_BACK on an object");
            glReadBuffer(GL_DEPTH_ATTACHMENT);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "an attachment on the window");
        }
    }

    /// ES 3.0 4.3.2: GL_PACK_ROW_LENGTH spaces the rows and the skips move
    /// the first pixel written; both were read back as 0 and ignored until
    /// 2026-09-24.
    #[test]
    fn read_pixels_honours_the_pack_row_length_and_skips() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 4, 3, 2);
        // SAFETY: plain GL calls.
        unsafe {
            glPixelStorei(GL_PACK_ROW_LENGTH, 5);
            glPixelStorei(GL_PACK_SKIP_ROWS, 1);
            glPixelStorei(GL_PACK_SKIP_PIXELS, 2);
        }
        /* rows 5 pixels (20 bytes) apart, the first written at row 1, pixel 2 */
        let mut out = std::vec![SENTINEL; 3 * 20];
        // SAFETY: the skips and two rows of 3 fit in three rows of 5 pixels.
        unsafe { glReadPixels(0, 0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(&out[..28], [SENTINEL; 28], "the skipped row and pixels are the caller's");
        assert_eq!(&out[28..40], [px(0, 0), px(1, 0), px(2, 0)].concat());
        assert_eq!(&out[48..60], [px(0, 1), px(1, 1), px(2, 1)].concat());
    }

    #[test]
    fn a_gl_to_gl_copy_ignores_the_pack_alignment() {
        /* The pack state describes transfers into the application's memory.
         * glCopyTexSubImage2D reads the framebuffer through the same helper
         * but into a buffer of ours, so it passes 1: an alignment left set by
         * the application must not shift the texels it writes. */
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL call.
        unsafe { glPixelStorei(GL_PACK_ALIGNMENT, 8) };
        bound_fb();
        let src = tex_with_storage(3, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, src);
        fill_rgba(&gl.ctx().objs_ref().textures[src as usize].mem, 4, 3, 2);

        let dst = tex_with_storage(3, 2);
        // SAFETY: plain GL calls; the destination level exists at 3x2.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, dst);
            glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 3, 2);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let m = gl.ctx().objs().textures[dst as usize].mem;
        let got = gl.gpu_bytes(m.gpu, 4 * 2 * 4);
        for j in 0..2 {
            let row = &got[j * 16..j * 16 + 12];
            assert_eq!(row, [px(0, j), px(1, j), px(2, j)].concat(), "row {j} lands at the texture pitch");
        }
    }

    #[test]
    fn read_pixels_of_an_rgb565_window_without_a_cpu_view() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let s = SurfaceCtx::new(&gl, 2, 1, GLES_SURF_RGB565, 2 * 2);
        s.render(0, &[0x1F, 0x00, 0x00, 0xF8]); /* blue, red */
        let out = read(0, 0, 2, 1);
        assert_eq!(out, [[0, 0, 255, 255], [255, 0, 0, 255]].concat());
    }

    #[test]
    fn a_failing_read_back_is_reported() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        bound_fb();
        let tex = tex_with_storage(2, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        fill_rgba(&gl.ctx().objs_ref().textures[tex as usize].mem, 2, 2, 2);
        gl.fail_gpu_read_in(1);
        read(0, 0, 2, 2);
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn a_draw_into_an_fbo_texture_uploaded_just_before_sees_the_upload() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(2, 2);
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        let texels = [9u8; 16];
        // SAFETY: plain GL calls; `texels` holds the 2x2 image.
        unsafe {
            crate::gl::texture::glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                2,
                2,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
            glClear(GL_COLOR_BUFFER_BIT);
            glFlush();
        }
        /* the kick loads the target: the mock checked the upload was flushed */
        assert_eq!(gl.draws().len(), 1);
        assert_eq!(gl.gpu_bytes(gl.ctx().objs().textures[tex as usize].mem.gpu, 16), [9; 16]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- regression tests for the round-2 fixes ------------------------------------------ */

    #[test]
    fn gen_and_delete_names_through_a_misaligned_array() {
        /* GLuint* from the application carries no alignment guarantee (the
         * aarch64 target is +strict-align, and a typed slice over it is UB) */
        let gl = TestGl::new(8, 8);
        let mut raw = [0u8; 3 * 4 + 1];
        let p = unsafe { raw.as_mut_ptr().add(1) } as *mut GLuint;
        assert_ne!(p as usize % 4, 0, "a deliberately misaligned array");
        // SAFETY: `p` has room for three names, one byte into the buffer.
        unsafe {
            glGenFramebuffers(3, p);
            glGenRenderbuffers(3, p);
        }
        let names: Vec<GLuint> = (0..3)
            // SAFETY: as above.
            .map(|i| unsafe { p.add(i).read_unaligned() })
            .collect();
        assert!(names.iter().all(|&n| n != 0));
        // SAFETY: as above.
        unsafe {
            glDeleteRenderbuffers(3, p);
            glDeleteFramebuffers(3, p);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().renderbuffers.iter().all(|r| !r.used));
    }

    #[test]
    fn a_compressed_or_rgb565_colour_attachment_is_incomplete() {
        /* the PBE writes linear RGBA8888 (or the float formats): a texture
         * of any other texel format is not a render target, and a draw into
         * it would write its 2-byte or block-compressed store as RGBA8 */
        let gl = TestGl::new(8, 8);
        bound_fb();
        let mut t: GLuint = 0;
        // SAFETY: room for one name; the block data is 2 x 2 blocks of 8 bytes.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            crate::gl::texture::glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_COMPRESSED_RGB8_ETC2,
                8,
                8,
                0,
                4 * 8,
                [0u8; 32].as_ptr() as *const c_void,
            );
        }
        attach_tex(GL_COLOR_ATTACHMENT0, t);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "ETC2");
        /* and glReadPixels of it is INVALID_FRAMEBUFFER_OPERATION, not an
         * over-read of the block store */
        let out = read(0, 0, 8, 8);
        assert!(out.iter().all(|&b| b == SENTINEL));
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);

        let mut backing = std::vec![0u8; 4 * 4 * 2];
        let mem = external_image_mem(&mut backing);
        let e565 = tex_with_storage(1, 1);
        assert_eq!(crate::gl::texture::texture_attach_external(gl.ctx(), &mem, 4, 4, GLES_SURF_RGB565), 0);
        attach_tex(GL_COLOR_ATTACHMENT0, e565);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "an RGB565 image");
        let _ = gl.error();
    }

    #[test]
    fn an_attachment_whose_storage_is_too_small_is_incomplete() {
        /* an image of w x h packed rows does not hold the pair-padded rows
         * the PBE writes: 5 x 4 RGBA8 needs 6 texels a row */
        let gl = TestGl::new(8, 8);
        bound_fb();
        let tex = tex_with_storage(1, 1);
        let mut packed = std::vec![0u8; 5 * 4 * 4];
        let mem = external_image_mem(&mut packed);
        /* the attach itself refuses the buffer */
        assert_eq!(crate::gl::texture::texture_attach_external(gl.ctx(), &mem, 5, 4, GLES_SURF_RGBA8888), -1);
        /* forced in (as a C caller could before the check), it is incomplete */
        {
            let t = &mut gl.ctx().objs().textures[tex as usize];
            t.mem = mem;
            t.external = true;
            t.width = 5;
            t.height = 4;
        }
        attach_tex(GL_COLOR_ATTACHMENT0, tex);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
        let out = read(0, 0, 5, 4);
        assert!(out.iter().all(|&b| b == SENTINEL), "nothing was read past the buffer");
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        gl.ctx().objs().textures[tex as usize].external = false;
        gl.ctx().objs().textures[tex as usize].mem = OsalGpuMem::ZERO;
    }

    #[test]
    fn a_3d_texture_attachment_is_incomplete() {
        /* a 3D texture's slices are tiled in a grid: there is no level the
         * PBE could write at the target's row pitch */
        let gl = TestGl::new(8, 8);
        let mut t: GLuint = 0;
        // SAFETY: room for one name; no pixels are read.
        unsafe {
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_3D, t);
            crate::gl::texture::glTexStorage3D(GL_TEXTURE_3D, 1, GL_RGBA8, 4, 4, 4);
        }
        bound_fb();
        // SAFETY: plain GL call.
        unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, t, 0, 1) };
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
        let _ = gl.error();
    }

    #[test]
    fn renderbuffer_storage_is_padded_and_cleared() {
        /* odd width: the PBE writes rows of whole pixel pairs, and the pool
         * hands out memory as its last user left it */
        let gl = TestGl::new(8, 8);
        let rb = rb_with_storage(GL_RGBA4, 5, 3);
        let mem = gl.ctx().objs().renderbuffers[rb as usize].mem;
        assert_eq!(mem.size, 6 * 3 * 4, "tex_pitch(5) = 6 texels a row");
        assert!(gl.gpu_bytes(mem.gpu, mem.size).iter().all(|&b| b == 0), "cleared");
        bound_fb();
        attach_rb(GL_COLOR_ATTACHMENT0, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
        /* the read-back walks the same pitch and stays inside the allocation */
        assert_eq!(read(0, 0, 5, 3), std::vec![0u8; 5 * 3 * 4]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// A descriptor over memory the test owns, mapped for the GPU: an
    /// EGLImage's buffer as far as the core is concerned.
    fn external_image_mem(backing: &mut Vec<u8>) -> OsalGpuMem {
        let cpu = backing.as_mut_ptr() as *mut c_void;
        crate::testutil::mock_map_phys(0x4321_0000, backing.len(), cpu);
        OsalGpuMem { cpu, gpu: 0x4321_0000, phys: 0x4321_0000, size: backing.len(), impl_: cpu }
    }

    /* ---- the sync to upstream e6c449c ------------------------------------ */

    #[test]
    fn colour_attachments_one_and_two_are_their_own_slots() {
        /* upstream e471a73: a G-buffer pass writes three attachments. */
        let gl = TestGl::new(8, 8);
        let fb = bound_fb();
        let (t0, t1, t2) = (tex_with_storage(8, 8), tex_with_storage(8, 8), tex_with_storage(8, 8));

        attach_tex(GL_COLOR_ATTACHMENT0, t0);
        attach_tex(GL_COLOR_ATTACHMENT1, t1);
        attach_tex(GL_COLOR_ATTACHMENT2, t2);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let f = &gl.ctx().framebuffers[fb as usize];
        assert_eq!((f.color.name, f.color_n[0].name, f.color_n[1].name), (t0, t1, t2));
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
        let c = gl.ctx();
        assert_eq!(fb_target_n(c, 1).map(|t| (t.1, t.2, t.3)), Some((8, 8, GLES_SURF_RGBA8888)));
        assert_eq!(fb_target_n(c, 2).map(|t| (t.1, t.2, t.3)), Some((8, 8, GLES_SURF_RGBA8888)));
        assert!(fb_target_n(c, 0).is_none() && fb_target_n(c, 3).is_none());
    }

    /// The GC7000 writes one colour output (context::MAX_DRAW_BUFFERS = 1), so
    /// a second draw buffer is refused rather than accepted and dropped: the
    /// backend reads target 0 only, and outputs 1 and 2 used to go nowhere.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_takes_one_draw_buffer_and_refuses_a_second() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(8, 8));
        attach_tex(GL_COLOR_ATTACHMENT1, tex_with_storage(8, 8));
        let one = [GL_COLOR_ATTACHMENT0];
        // SAFETY: one readable enum.
        unsafe { glDrawBuffers(1, one.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let two = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
        // SAFETY: two readable enums.
        unsafe { glDrawBuffers(2, two.as_ptr()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE, "past GL_MAX_DRAW_BUFFERS = 1");
    }

    /// The V3D writes every colour attachment a draw names (MRT steps 1-3,
    /// docs/online-shaderc/v3d-backend.md): three draw buffers are taken, and
    /// a fourth is past GL_MAX_DRAW_BUFFERS (CAP_COLOR_ATTACHMENTS, 3).
    #[cfg(feature = "gpu-v3d")]
    #[test]
    fn the_v3d_takes_every_draw_buffer() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let atts = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2];
        for a in atts {
            attach_tex(a, tex_with_storage(8, 8));
        }
        // SAFETY: three readable enums.
        unsafe { glDrawBuffers(3, atts.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR, "three draw buffers");
        /* four valid enums (GL_COLOR_ATTACHMENT3 itself is no attachment
         * point here, GL_INVALID_ENUM, which glDrawBuffers checks first) */
        let four = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_NONE];
        // SAFETY: four readable enums.
        unsafe { glDrawBuffers(4, four.as_ptr()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE, "past GL_MAX_DRAW_BUFFERS = 3");
    }

    /// With draw buffers switched on (`gc_debug_mrt`, MRT steps 1-2), the
    /// GC7000 answers GL_MAX_DRAW_BUFFERS with every colour attachment and
    /// takes that many draw buffers; switched off it is one again, the query
    /// and glDrawBuffers agreeing both ways.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_gc7000_takes_every_draw_buffer_with_mrt_switched_on() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        for (i, a) in [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2].iter().enumerate() {
            attach_tex(*a, tex_with_storage(8, 8));
            assert_eq!(gl.error(), GL_NO_ERROR, "attachment {i}");
        }
        let all = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2];
        let mut max = 0;
        crate::vivante::backend::gc_debug_set_mrt(1);
        // SAFETY: a readable int; three readable enums.
        unsafe {
            super::super::state::glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut max);
            glDrawBuffers(3, all.as_ptr());
        }
        let on = (max, gl.error());
        crate::vivante::backend::gc_debug_set_mrt(0);
        // SAFETY: as above.
        unsafe {
            super::super::state::glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut max);
            glDrawBuffers(3, all.as_ptr());
        }
        let off = (max, gl.error());
        assert_eq!(on, (CAP_COLOR_ATTACHMENTS as GLint, GL_NO_ERROR), "switched on: every attachment");
        assert_eq!(off, (1, GL_INVALID_VALUE), "switched off: one, as before");
    }

    /// Where more than one output is written (PowerVR); the GC7000 writes one
    /// until multiple render targets land, and refuses a second below.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn draw_buffers_says_which_attachment_an_output_writes() {
        /* upstream 6a1c6df: output n goes to the attachment glDrawBuffers
         * named for it; GL_NONE drops it. */
        let gl = TestGl::new(8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(8, 8));
        attach_tex(GL_COLOR_ATTACHMENT1, tex_with_storage(8, 8));
        let bufs = [GL_COLOR_ATTACHMENT0, GL_NONE];
        // SAFETY: two readable enums.
        unsafe { glDrawBuffers(2, bufs.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(fb_target_n(gl.ctx(), 1).is_none(), "output 1 writes nothing");
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
        // SAFETY: two readable enums.
        unsafe { glDrawBuffers(2, bufs.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(fb_target_n(gl.ctx(), 1).is_some());
        /* buffer i must name attachment i */
        let bufs = [GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT0];
        // SAFETY: two readable enums.
        unsafe { glDrawBuffers(2, bufs.as_ptr()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn a_depth_texture_may_be_the_depth_attachment() {
        /* upstream 6a1c6df. */
        let gl = TestGl::new(8, 8);
        bound_fb();
        let mut dt: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut dt);
            glBindTexture(GL_TEXTURE_2D, dt);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                DEPTH24_TEX as GLint,
                8,
                8,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(8, 8));
        // SAFETY: plain GL call.
        unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, dt, 0) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
        /* a colour texture is still not a depth attachment */
        // SAFETY: plain GL call.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex_with_storage(8, 8), 0)
        };
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    }

    /// The attachment kinds dEQP-GLES2 fbo.completeness.attachment_combinations
    /// builds once GL_OES_depth_texture and GL_OES_packed_depth_stencil are
    /// claimed (glsFboCompletenessTests.cpp makeDepthAndStencil: the stencil
    /// image first, from the first STENCIL_RENDERABLE | TEXTURE_VALID format,
    /// (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8); then the depth one, a
    /// (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT) texture or a DEPTH_COMPONENT16
    /// renderbuffer; separate images even when both are textures). Each one
    /// with a colour attachment is complete; a depth-stencil texture is a
    /// depth attachment, a stencil attachment and both, and not a colour one;
    /// a depth-only texture is no stencil attachment.
    #[test]
    fn depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say() {
        let gl = TestGl::new(8, 8);
        let colour = tex_with_storage(8, 8);
        let ds = depth_tex(GL_DEPTH_STENCIL, 8, 8);
        let ds2 = depth_tex(GL_DEPTH_STENCIL, 8, 8);
        let d = depth_tex(GL_DEPTH_COMPONENT, 8, 8);
        assert_eq!(gl.error(), GL_NO_ERROR, "both formats are texture images");
        let d16 = rb_with_storage(GL_DEPTH_COMPONENT16, 8, 8);
        let s8 = rb_with_storage(GL_STENCIL_INDEX8, 8, 8);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, colour);
        /* (depth kind, stencil kind): none / rbo / tex on each point */
        let cases: [(GLenum, GLuint, GLenum, GLuint, &str); 7] = [
            (GL_NONE, 0, GL_TEXTURE, ds, "a depth-stencil texture on the stencil point alone"),
            (GL_RENDERBUFFER, d16, GL_TEXTURE, ds, "a depth renderbuffer and a depth-stencil texture's stencil"),
            (GL_TEXTURE, d, GL_TEXTURE, ds, "a depth texture and another texture's stencil"),
            (GL_TEXTURE, ds2, GL_TEXTURE, ds, "two depth-stencil textures, one on each point"),
            (GL_TEXTURE, ds, GL_TEXTURE, ds, "one depth-stencil texture on both points"),
            (GL_TEXTURE, d, GL_RENDERBUFFER, s8, "a depth texture and a stencil renderbuffer"),
            (GL_TEXTURE, ds, GL_NONE, 0, "a depth-stencil texture on the depth point alone"),
        ];
        for (dk, dn, sk, sn, what) in cases {
            for (point, kind, name) in [(GL_DEPTH_ATTACHMENT, dk, dn), (GL_STENCIL_ATTACHMENT, sk, sn)] {
                match kind {
                    GL_TEXTURE => attach_tex(point, name),
                    GL_RENDERBUFFER => attach_rb(point, name),
                    _ => attach_tex(point, 0),
                }
            }
            assert_eq!(gl.error(), GL_NO_ERROR, "{what}");
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what}");
        }
        /* a depth-only texture has no stencil */
        attach_tex(GL_DEPTH_ATTACHMENT, 0);
        attach_tex(GL_STENCIL_ATTACHMENT, d);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a depth texture on the stencil point");
        /* neither kind is a colour image */
        attach_tex(GL_STENCIL_ATTACHMENT, 0);
        attach_tex(GL_COLOR_ATTACHMENT0, ds);
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "a depth-stencil texture as colour");
        /* without colour: complete where the backend draws with no colour
         * target, else nothing for the PBE to write, which ES 2.0 lets an
         * implementation call unsupported (4.4.5) */
        attach_tex(GL_COLOR_ATTACHMENT0, 0);
        attach_tex(GL_DEPTH_ATTACHMENT, ds);
        attach_tex(GL_STENCIL_ATTACHMENT, ds);
        let want =
            if crate::backend::DEPTH_ONLY_TARGETS { GL_FRAMEBUFFER_COMPLETE } else { GL_FRAMEBUFFER_UNSUPPORTED };
        assert_eq!(status(), want);
        /* another size is another size, texture or not */
        attach_tex(GL_COLOR_ATTACHMENT0, colour);
        attach_tex(GL_STENCIL_ATTACHMENT, depth_tex(GL_DEPTH_STENCIL, 4, 4));
        assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// The attachment queries name a depth-stencil texture on the stencil
    /// point as the texture it is, level 0.
    #[test]
    fn a_depth_stencil_texture_on_the_stencil_point_is_queried_as_a_texture() {
        let _gl = TestGl::new(8, 8);
        let ds = depth_tex(GL_DEPTH_STENCIL, 8, 8);
        bound_fb();
        attach_tex(GL_STENCIL_ATTACHMENT, ds);
        assert_eq!(attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), GL_TEXTURE as GLint);
        assert_eq!(attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), ds as GLint);
        assert_eq!(attachment_param(GL_STENCIL_ATTACHMENT, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL), 0);
    }

    /// GL_DEPTH24_STENCIL8_OES (the same token as ES 3.0's) is renderbuffer
    /// storage on both surfaces and completes the depth and stencil points
    /// as one image.
    #[test]
    fn a_depth24_stencil8_renderbuffer_is_depth_and_stencil() {
        let gl = TestGl::new(8, 8);
        let colour = tex_with_storage(8, 8);
        let rb = rb_with_storage(GL_DEPTH24_STENCIL8, 8, 8);
        assert_eq!(gl.error(), GL_NO_ERROR);
        bound_fb();
        attach_tex(GL_COLOR_ATTACHMENT0, colour);
        attach_rb(GL_STENCIL_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "stencil alone");
        attach_rb(GL_DEPTH_ATTACHMENT, rb);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "both points");
    }

    #[test]
    fn a_depth_texture_is_not_a_colour_attachment() {
        let gl = TestGl::new(8, 8);
        bound_fb();
        let mut dt: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut dt);
            glBindTexture(GL_TEXTURE_2D, dt);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                DEPTH24_TEX as GLint,
                8,
                8,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
        }
        attach_tex(GL_COLOR_ATTACHMENT0, dt);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
        attach_tex(GL_COLOR_ATTACHMENT0, tex_with_storage(8, 8));
        attach_tex(GL_COLOR_ATTACHMENT1, dt);
        assert!(fb_target_n(gl.ctx(), 1).is_none(), "attachment 1 is not a colour surface either");
    }

    #[test]
    fn a_framebuffer_to_texture_copy_syncs_once() {
        /* upstream 8c7a67e / 6b584b9: through glReadPixels every chunk
         * submitted the frame, waited for the fence and invalidated the
         * whole surface again. */
        let gl = TestGl::new(64, 4);
        let dst = tex_with_storage(64, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glBindTexture(GL_TEXTURE_2D, dst);
        }
        let _ = gl.cache_ops(); /* drain what the clear left */
        // SAFETY: plain GL call.
        unsafe { crate::gl::texture::glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 64, 4) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let ops = gl.cache_ops();
        let invds = ops.iter().filter(|o| matches!(o, CacheOp::Invd { .. })).count();
        assert_eq!(invds, 1, "one invalidate for the whole copy, not one per chunk: {ops:?}");
    }

    /* ---- the enums dEQP-GLES2 negative_api.buffer.* asks for --------------- */

    /// `framebuffer_texture2d`: `textarget` must name the kind of texture it
    /// was given. A cube face of a 2D texture, or GL_TEXTURE_2D of a cube map,
    /// is GL_INVALID_OPERATION.
    #[test]
    fn framebuffer_texture_2d_textarget_must_match_the_texture() {
        let gl = TestGl::new(8, 8);
        let (mut fbo, mut tex2d, mut cube) = (0 as GLuint, 0 as GLuint, 0 as GLuint);
        // SAFETY: room for one name each; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            texture::glGenTextures(1, &mut tex2d);
            texture::glBindTexture(GL_TEXTURE_2D, tex2d);
            texture::glGenTextures(1, &mut cube);
            texture::glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
            assert_eq!(gl.error(), GL_NO_ERROR);

            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X, tex2d, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "a cube face of a 2D texture");
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, cube, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "GL_TEXTURE_2D of a cube map");

            /* each kind with its own target is accepted */
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex2d, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X, cube, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            /* detaching names no texture, so no kind to disagree with */
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 0, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    /// `renderbuffer_storage`: renderbuffer storage is named by a SIZED
    /// format. The unsized base formats belong to textures -- and a texture
    /// attachment reports GL_RGBA, so the colour-attachment predicate keeps
    /// it and this one does not.
    #[test]
    fn the_sized_bgra_name_is_taken_wherever_the_unsized_one_is() {
        /* GL_EXT_texture_format_BGRA8888 is in GL_EXTENSIONS, and dEQP's FBO
         * completeness requires GL_BGRA and GL_BGRA8_EXT from anyone who
         * advertises it: renderbuffer storage, a texture internal format and
         * a complete framebuffer from either. Only the unsized name was
         * taken, so the sized one answered GL_INVALID_ENUM on a renderbuffer
         * and GL_INVALID_VALUE on a texture -- claiming the extension and
         * then refusing half of it. */
        let gl = TestGl::new(8, 8);
        for f in [GL_BGRA_EXT, GL_BGRA8_EXT] {
            let rb = rb_with_storage(f, 4, 4);
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "renderbuffer 0x{f:x}");
            assert_ne!(gl.ctx().objs().renderbuffers[rb as usize].mem.gpu, 0, "storage for 0x{f:x}");
        }
        /* and as a texture internal format */
        let mut t: GLuint = 0;
        // SAFETY: room for one name; null pixels only allocate.
        unsafe {
            crate::gl::texture::glGenTextures(1, &mut t);
            crate::gl::texture::glBindTexture(GL_TEXTURE_2D, t);
            crate::gl::texture::glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_BGRA8_EXT as GLint,
                4,
                4,
                0,
                GL_BGRA_EXT,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
        }
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "GL_BGRA8_EXT as an internal format");
    }

    #[test]
    fn renderbuffer_storage_takes_sized_formats_only() {
        let gl = TestGl::new(8, 8);
        let mut rb: GLuint = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA, 8, 8);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "GL_RGBA is unsized");
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 8, 8);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert!(is_color_format(GL_RGBA), "a texture colour attachment still reports GL_RGBA");
    }

    /// dEQP `es2fFboStateQueryTests.framebuffer_attachment_texture_cube_map_face`:
    /// the query reads back the face that was attached, and 0 for a texture
    /// that is not a cube map. The face was recorded on the attachment and
    /// thrown away on the read.
    #[test]
    fn the_attachment_query_reports_the_cube_face_it_was_given() {
        let gl = TestGl::new(8, 8);
        let (mut fbo, mut cube, mut tex2d) = (0 as GLuint, 0 as GLuint, 0 as GLuint);
        let px = std::vec![0u8; 4 * 4 * 4];
        // SAFETY: room for one name each; `px` holds each 4x4 face.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            texture::glGenTextures(1, &mut cube);
            texture::glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
            for f in 0..6u32 {
                texture::glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                    0,
                    GL_RGBA as GLint,
                    4,
                    4,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                );
            }
            texture::glGenTextures(1, &mut tex2d);
            texture::glBindTexture(GL_TEXTURE_2D, tex2d);
            texture::glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                4,
                4,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        for f in 0..6u32 {
            let face = GL_TEXTURE_CUBE_MAP_POSITIVE_X + f;
            let mut v: GLint = -1;
            // SAFETY: plain GL calls; `v` holds one value.
            unsafe {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, face, cube, 0);
                glGetFramebufferAttachmentParameteriv(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE,
                    &mut v,
                );
            }
            assert_eq!(v as GLenum, face, "face {f}");
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        /* a 2D texture has no face: 0 */
        let mut v: GLint = -1;
        // SAFETY: plain GL calls; `v` holds one value.
        unsafe {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex2d, 0);
            glGetFramebufferAttachmentParameteriv(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE,
                &mut v,
            );
        }
        assert_eq!(v, 0, "a 2D texture attachment has no cube face");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
