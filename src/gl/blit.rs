// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer (ES 3.0.6 4.3.3): a copy of a rectangle of the READ
//! framebuffer's read buffer into every draw buffer of the DRAW framebuffer,
//! scaled, flipped and converted on the way.
//!
//! It is a CPU copy in the GL core, shared by the three back ends, built on
//! what glReadPixels and glCopyTex* already rely on: every colour image is
//! linear memory the CPU can reach (`framebuffer::attachment_storage`), the
//! GPU work is made idle once (`framebuffer::gpu_idle_for_cpu`), the source
//! rows are brought into the CPU's view, and the destination's rows are
//! invalidated before they are written and written back after. A frame that
//! later renders into the destination without clearing it first loads its
//! colour back from memory (every back end does, as for a texture the CPU
//! wrote), so the copy is seen by the draws that follow.
//!
//! The pixels follow dEQP's reference (`sglr::ReferenceContext::
//! blitFramebuffer`), which is the spec's arithmetic: the source rectangle is
//! cut to the read buffer and the destination rectangle to each draw buffer
//! and the scissor box; destination pixel centre (x + 0.5, y + 0.5) maps
//! through the affine transform of the two *uncut* rectangles into the cut
//! source, and a pixel whose sample point falls outside it is left alone.
//! NEAREST takes the texel under the point; LINEAR (fixed and floating point
//! only, and only where the rectangles differ in size) filters the four
//! around it with clamp-to-edge inside the cut source.
//!
//! Conversion goes through a value per channel: a fixed or floating point
//! read buffer decodes to floats (unorm `v / (2^b - 1)`), a draw buffer
//! encodes them with its own rounding (unorm: clamped and rounded to the
//! *format's* bits, then stored at the storage's eight; half floats as the
//! upload path rounds them); an integer read buffer copies values into the
//! same signedness, saturated to the draw buffer's width. A channel the
//! format lacks reads 0 (alpha 1) and is written 0 (alpha 1). The decode and
//! encode are the one place per storage class: a class added later (sRGB
//! storage, whose decode before filtering and encode after belong in
//! [`decode_float`] / [`encode_float`]) needs no change to the copy itself.
//!
//! Depth and stencil are validated as the spec says, but not copied: the back
//! ends keep depth/stencil in buffers of their own (`ds_retention`), which
//! have no CPU write path yet; a blit that would copy them is refused with
//! GL_INVALID_OPERATION and says so once in the log.

use core::ffi::c_void;

use super::consts::*;
use super::context::{GlAttachment, GlContext, CAP_COLOR_ATTACHMENTS};
use super::framebuffer::{
    self, attached_samples, attachment_storage, check_status, loc_pitch, mem_at, mem_at_mut, quantise_unorm,
    renderbuffer_bits, renderbuffer_surf_format, texture_bits, AttachmentBits, ColorKind, MemLoc,
};
use super::types::*;
use crate::backend::{
    self, gles_surf_bytes_per_pixel, gles_surf_int_layout, GLES_SURF_R16F, GLES_SURF_R32F, GLES_SURF_RGB565,
    GLES_SURF_RGBA16F, GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_TEXFMT_R16F, GLES_TEXFMT_R32F, GLES_TEXFMT_RGBA16F,
    GLES_TEXFMT_RGBA8,
};
use crate::osal::{self, OsalGpuMem};
use crate::util::Global;

const FN: &str = "glBlitFramebuffer";

/// One colour image of a framebuffer as it is stored.
#[derive(Clone, Copy, Debug)]
struct Image {
    loc: MemLoc,
    /// Byte offset of the attached level/face/layer.
    off: usize,
    w: u32,
    h: u32,
    /// Row pitch in pixels.
    pitch: usize,
    /// `GLES_SURF_*` the bytes are laid out in.
    surf: u8,
    bpp: usize,
    /// RGBA8888 bytes stored B,G,R,A (the pixel engine's order on some GPUs,
    /// an XRGB8888 image's always).
    bgra: bool,
    /// The fourth byte is the X of XRGB8888: it reads 1.
    opaque: bool,
    /// Stored top-down (a window surface): GL row y is storage row h-1-y.
    flip: bool,
    kind: ColorKind,
    /// The GL format's bits a channel (R, G, B, A); 0 for a channel it lacks.
    bits: [GLint; 4],
    /// The GL internal format with its encoding (the multisample rule
    /// compares them).
    internalformat: GLenum,
    /// The texture slot and level, for a texture's write bookkeeping.
    tex: Option<(usize, GLint)>,
}

/// GL_SAMPLE_BUFFERS of framebuffer `fb` (the default one's: the surface's).
fn fb_samples(c: &GlContext, fb: GLuint) -> u8 {
    if fb == 0 {
        return if c.default_samples > 1 { c.default_samples } else { 0 };
    }
    c.framebuffer_ref(fb)
        .filter(|f| check_status(c, f) == GL_FRAMEBUFFER_COMPLETE)
        .and_then(|f| attached_samples(c, f))
        .unwrap_or(0)
}

/// Is framebuffer `fb` complete (the default one: when it has a surface)?
fn fb_complete(c: &GlContext, fb: GLuint) -> bool {
    if fb == 0 {
        return c.default_surface_valid;
    }
    c.framebuffer_ref(fb).is_some_and(|f| check_status(c, f) == GL_FRAMEBUFFER_COMPLETE)
}

/// The default surface's colour image.
fn default_image(c: &GlContext) -> Option<Image> {
    if !c.default_surface_valid {
        return None;
    }
    let surf = c.default_format;
    let (bits, internalformat) = match surf {
        GLES_SURF_RGB565 => ([5, 6, 5, 0], GL_RGB565),
        GLES_SURF_XRGB8888 => ([8, 8, 8, 0], GL_RGB8),
        _ => ([8, 8, 8, 8], GL_RGBA8),
    };
    Some(Image {
        loc: MemLoc::DefaultSurface,
        off: 0,
        w: c.target_w,
        h: c.target_h,
        pitch: c.default_pitch_px() as usize,
        surf,
        bpp: gles_surf_bytes_per_pixel(surf) as usize,
        bgra: surf == GLES_SURF_XRGB8888 || (surf == GLES_SURF_RGBA8888 && backend::rt_rgba8888_is_bgra()),
        opaque: surf == GLES_SURF_XRGB8888,
        flip: !c.default_y_up,
        kind: ColorKind::Float,
        bits,
        internalformat,
        tex: None,
    })
}

/// The colour image attached at `a` of a framebuffer object, as stored.
fn attachment_image(c: &GlContext, a: &GlAttachment) -> Option<Image> {
    if a.type_ == GL_NONE {
        return None;
    }
    let s = attachment_storage(c, a)?;
    let loc = s.mem?;
    let (surf, bits, bgra, opaque, tex): (u8, AttachmentBits, bool, bool, _) = match a.type_ {
        GL_TEXTURE => {
            let t = c.texture_ref(a.name)?;
            if matches!(t.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL) {
                return None;
            }
            let surf = match t.texfmt {
                GLES_TEXFMT_RGBA16F => GLES_SURF_RGBA16F,
                GLES_TEXFMT_R16F => GLES_SURF_R16F,
                GLES_TEXFMT_R32F => GLES_SURF_R32F,
                GLES_TEXFMT_RGBA8 if t.bgra => GLES_SURF_XRGB8888,
                GLES_TEXFMT_RGBA8 => GLES_SURF_RGBA8888,
                _ => return None,
            };
            /* a texture the pixel engine rendered into holds B,G,R,A
             * (`pe_order`); one only the CPU wrote, R,G,B,A */
            (surf, texture_bits(t), t.bgra || t.pe_order, t.bgra, Some((a.name as usize, a.level)))
        }
        _ => {
            let surf = renderbuffer_surf_format(s.internalformat);
            let bits = renderbuffer_bits(s.internalformat);
            if bits.rgba == [0; 4] {
                return None; /* a depth or stencil renderbuffer */
            }
            (surf, bits, surf == GLES_SURF_RGBA8888 && backend::rt_rgba8888_is_bgra(), false, None)
        }
    };
    let kind = match bits.component_type {
        GL_INT => ColorKind::Int,
        GL_UNSIGNED_INT => ColorKind::Uint,
        _ => ColorKind::Float,
    };
    Some(Image {
        loc,
        off: s.offset as usize,
        w: s.w,
        h: s.h,
        pitch: loc_pitch(c, loc, s.w) as usize,
        surf,
        bpp: gles_surf_bytes_per_pixel(surf) as usize,
        bgra,
        opaque,
        flip: false,
        kind,
        bits: bits.rgba,
        internalformat: s.internalformat,
        tex,
    })
}

/// The read buffer of framebuffer `fb` (None: GL_NONE, or nothing attached
/// where it points).
fn read_image(c: &GlContext, fb: GLuint) -> Option<Image> {
    if fb == 0 {
        return if c.default_read_none { None } else { default_image(c) };
    }
    let f = c.framebuffer_ref(fb)?;
    if f.read_none {
        return None;
    }
    let a = match f.read_index as usize {
        0 => &f.color,
        n if n < CAP_COLOR_ATTACHMENTS => &f.color_n[n - 1],
        _ => return None,
    };
    attachment_image(c, a)
}

/// The images the bound DRAW framebuffer's draw buffers write (GL_NONE and
/// draw buffers with nothing attached write nothing).
fn draw_images(c: &GlContext) -> ([Option<Image>; CAP_COLOR_ATTACHMENTS], usize) {
    let mut out = [None; CAP_COLOR_ATTACHMENTS];
    let mut n = 0;
    if c.bound_framebuffer == 0 {
        if !framebuffer::default_draw_none(c) {
            out[0] = default_image(c);
            n = out[0].is_some() as usize;
        }
        return (out, n);
    }
    let Some(f) = c.framebuffer_ref(c.bound_framebuffer) else { return (out, 0) };
    for i in 0..CAP_COLOR_ATTACHMENTS {
        let b = framebuffer::draw_buffer(c, i);
        if b == GL_NONE {
            continue;
        }
        let a = match (b - GL_COLOR_ATTACHMENT0) as usize {
            0 => &f.color,
            k if k < CAP_COLOR_ATTACHMENTS => &f.color_n[k - 1],
            _ => continue,
        };
        if let Some(img) = attachment_image(c, a) {
            out[n] = Some(img);
            n += 1;
        }
    }
    (out, n)
}

/// A depth or stencil attachment of a framebuffer, as the blit's rules see
/// it.
struct DsImage {
    /// (depth bits, depth component type, stencil bits).
    fmt: (GLint, GLenum, GLint),
    /// Which image it is: the attached object (type, name), or (GL_NONE, 0)
    /// for the window surface's own.
    image: (GLenum, GLuint),
}

/// The depth (or with `stencil` the stencil) attachment of framebuffer
/// `fb`; None when `fb` has none there.
fn ds_format(c: &GlContext, fb: GLuint, stencil: bool) -> Option<DsImage> {
    if fb == 0 {
        /* the window surface always has a D24S8 (`fb_target_depth_stencil`) */
        return Some(DsImage { fmt: (24, GL_UNSIGNED_NORMALIZED, 8), image: (GL_NONE, 0) });
    }
    let f = c.framebuffer_ref(fb)?;
    let a = if stencil { &f.stencil } else { &f.depth };
    if a.type_ == GL_NONE {
        return None;
    }
    let s = attachment_storage(c, a)?;
    let b = renderbuffer_bits(s.internalformat);
    Some(DsImage { fmt: (b.depth, b.component_type, b.stencil), image: (a.type_, a.name) })
}

/// A sized internal format with its sRGB encoding folded away (the
/// multisample rule lets a blit between the two encodings through).
fn linear_format(f: GLenum) -> GLenum {
    match f {
        GL_SRGB8_ALPHA8 => GL_RGBA8,
        GL_RGBA => GL_RGBA8,
        GL_RGB => GL_RGB8,
        f => f,
    }
}

/// A rectangle as x, y, width, height (width/height may be 0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

impl Rect {
    /// The rectangle of two corners, either order (p0 inclusive, p1 exclusive).
    fn of(x0: GLint, y0: GLint, x1: GLint, y1: GLint) -> Rect {
        let (x0, y0, x1, y1) = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
        Rect { x: x0.min(x1), y: y0.min(y1), w: (x1 - x0).abs(), h: (y1 - y0).abs() }
    }

    fn cut(self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }

    fn empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    fn of_image(i: &Image) -> Rect {
        Rect { x: 0, y: 0, w: i.w as i64, h: i.h as i64 }
    }
}

/// The two corners' rectangles of the call, as given.
#[derive(Clone, Copy, Debug)]
pub(super) struct Corners {
    /// srcX0, srcY0, srcX1, srcY1.
    pub(super) src: [GLint; 4],
    /// dstX0, dstY0, dstX1, dstY1.
    pub(super) dst: [GLint; 4],
}

/// The one-axis map from destination pixel `o` of the cut destination
/// rectangle (starting at `cut_dst0`) to a source coordinate in the cut
/// source rectangle (starting at `cut_src0`): the affine part of dEQP's
/// transform, in its f32 operation order, so NEAREST picks the texels its
/// reference picks at the half-way points.
#[derive(Clone, Copy, Debug)]
struct Axis {
    scale: f32,
    bias: f32,
}

impl Axis {
    fn new(s0: GLint, s1: GLint, d0: GLint, d1: GLint, cut_src0: i64, cut_dst0: i64) -> Axis {
        let scale = (s1 as i64 - s0 as i64) as f32 / (d1 as i64 - d0 as i64) as f32;
        let bias = scale * (cut_dst0 - d0 as i64) as f32 + (s0 as i64 - cut_src0) as f32;
        Axis { scale, bias }
    }

    fn at(self, o: i64) -> f32 {
        self.scale * (o as f32 + 0.5) + self.bias
    }
}

/// Bring `img`'s rows into the CPU's view (after the GPU wrote them).
fn invalidate(c: &GlContext, img: &Image) {
    let m = mem_at(c, img.loc);
    if m.cpu.is_null() {
        return;
    }
    let bytes = ((img.h as usize - 1) * img.pitch + img.w as usize) * img.bpp;
    // SAFETY: attachment_storage / the default surface checked that the
    // mapping holds the image's rows from `off`.
    unsafe { osal::cache_invd((m.cpu as *mut u8).add(img.off) as *mut c_void, bytes) };
}

/// Write `img`'s rows back from the CPU's cache (the GPU reads memory).
fn write_back(c: &GlContext, img: &Image) {
    let m = mem_at(c, img.loc);
    if m.cpu.is_null() {
        return;
    }
    let bytes = ((img.h as usize - 1) * img.pitch + img.w as usize) * img.bpp;
    // SAFETY: as in `invalidate`.
    unsafe { osal::cache_flush((m.cpu as *mut u8).add(img.off) as *mut c_void, bytes) };
}

/// The byte offset of GL pixel (x, y) of `img` in its memory.
fn pixel_off(img: &Image, x: u32, y: u32) -> usize {
    let row = if img.flip { img.h - 1 - y } else { y };
    img.off + (row as usize * img.pitch + x as usize) * img.bpp
}

/// Read the stored bytes of GL pixel (x, y) of `img` into `raw`. False when
/// the platform failed the read (the bytes are then zero).
fn load(c: &GlContext, img: &Image, x: u32, y: u32, raw: &mut [u8; 16]) -> bool {
    let m = mem_at(c, img.loc);
    let off = pixel_off(img, x, y);
    if !m.cpu.is_null() {
        // SAFETY: (x, y) lies inside the image, whose rows the mapping holds.
        let s = unsafe { core::slice::from_raw_parts((m.cpu as *const u8).add(off), img.bpp) };
        raw[..img.bpp].copy_from_slice(s);
        return true;
    }
    let mut copy: OsalGpuMem = *m;
    let ok = osal::ops().and_then(|o| o.gpu_read).is_some_and(|gpu_read| {
        // SAFETY: a live descriptor (a copy of it: the OSAL reads its handle)
        // and room for bpp bytes.
        unsafe { gpu_read(&mut copy, off, raw.as_mut_ptr() as *mut c_void, img.bpp) == 0 }
    });
    if !ok {
        raw.fill(0);
    }
    ok
}

/// Store `raw` as GL pixel (x, y) of `img`. False when the platform failed
/// the write.
fn store(c: &mut GlContext, img: &Image, x: u32, y: u32, raw: &[u8; 16]) -> bool {
    let off = pixel_off(img, x, y);
    let m: *mut OsalGpuMem = mem_at_mut(c, img.loc);
    // SAFETY: `m` points at a descriptor inside the current context.
    let cpu = unsafe { (*m).cpu };
    if !cpu.is_null() {
        // SAFETY: (x, y) lies inside the image, whose rows the mapping holds.
        unsafe { core::ptr::copy_nonoverlapping(raw.as_ptr(), (cpu as *mut u8).add(off), img.bpp) };
        return true;
    }
    osal::ops().and_then(|o| o.gpu_write).is_some_and(|gpu_write| {
        // SAFETY: a live descriptor and `bpp` bytes of `raw`.
        unsafe { gpu_write(m, off, raw.as_ptr() as *const c_void, img.bpp) == 0 }
    })
}

/// The channels a fixed or floating point image's stored pixel holds, as
/// floats; the channels its format lacks read 0, 0, 0, 1.
///
/// The place a storage class with its own decode (sRGB: to linear, before
/// any filtering) adds it.
fn decode_float(img: &Image, raw: &[u8; 16]) -> [f32; 4] {
    let half = |i: usize| framebuffer::half_to_float(u16::from_le_bytes([raw[i], raw[i + 1]]));
    let mut v = match img.surf {
        GLES_SURF_RGB565 => {
            let p = u16::from_le_bytes([raw[0], raw[1]]) as u32;
            [((p >> 11) & 0x1f) as f32 / 31.0, ((p >> 5) & 0x3f) as f32 / 63.0, (p & 0x1f) as f32 / 31.0, 1.0]
        }
        GLES_SURF_RGBA16F => [half(0), half(2), half(4), half(6)],
        GLES_SURF_R16F => [half(0), 0.0, 0.0, 1.0],
        GLES_SURF_R32F => [f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]), 0.0, 0.0, 1.0],
        _ => {
            let (r, b) = if img.bgra { (raw[2], raw[0]) } else { (raw[0], raw[2]) };
            let a = if img.opaque { 255 } else { raw[3] };
            [r as f32 / 255.0, raw[1] as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
        }
    };
    for (ch, x) in v.iter_mut().enumerate() {
        if img.bits[ch] == 0 {
            *x = if ch == 3 { 1.0 } else { 0.0 };
        }
    }
    v
}

/// An unorm channel `v` at `bits` bits, as the eight-bit storage keeps it.
fn unorm8(v: f32, bits: GLint) -> u8 {
    /* (quantise_unorm's 0 bits read 1: the caller has already put the
     * value a missing channel is written as) */
    let q = if bits <= 0 { v } else { quantise_unorm(v, bits.min(8) as u8) };
    /* NaN stays NaN through clamp and the cast gives 0 */
    (q.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The stored bytes of float channels `v` in a fixed or floating point
/// image: unorm rounded to the format's bits, half floats as uploads round
/// them; the channels the format lacks are written 0, 0, 0, 1.
///
/// The place a storage class with its own encode (sRGB: from linear, after
/// filtering) adds it.
fn encode_float(img: &Image, mut v: [f32; 4], raw: &mut [u8; 16]) {
    for (ch, x) in v.iter_mut().enumerate() {
        if img.bits[ch] == 0 {
            *x = if ch == 3 { 1.0 } else { 0.0 };
        }
    }
    let half = |raw: &mut [u8; 16], i: usize, x: f32| {
        raw[i..i + 2].copy_from_slice(&super::texture::float_to_half(x).to_le_bytes());
    };
    match img.surf {
        GLES_SURF_RGB565 => {
            let q = |x: f32, max: f32| (x.clamp(0.0, 1.0) * max + 0.5) as u16;
            let p = q(v[0], 31.0) << 11 | q(v[1], 63.0) << 5 | q(v[2], 31.0);
            raw[..2].copy_from_slice(&p.to_le_bytes());
        }
        GLES_SURF_RGBA16F => {
            for (ch, &x) in v.iter().enumerate() {
                half(raw, ch * 2, x);
            }
        }
        GLES_SURF_R16F => half(raw, 0, v[0]),
        GLES_SURF_R32F => raw[..4].copy_from_slice(&v[0].to_le_bytes()),
        _ => {
            let b: [u8; 4] = core::array::from_fn(|ch| unorm8(v[ch], img.bits[ch]));
            let a = if img.opaque { 0xff } else { b[3] };
            raw[..4].copy_from_slice(&if img.bgra { [b[2], b[1], b[0], a] } else { [b[0], b[1], b[2], a] });
        }
    }
}

/// The values of an integer image's stored pixel (a signed format's sign
/// extended); the channels it lacks read 0, 0, 0, 1.
fn decode_int(img: &Image, raw: &[u8; 16]) -> [u32; 4] {
    let Some(l) = gles_surf_int_layout(img.surf) else { return [0, 0, 0, 1] };
    let mut v = [0u32, 0, 0, 1];
    if l.bits == 10 {
        let w = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        return [w & 0x3ff, (w >> 10) & 0x3ff, (w >> 20) & 0x3ff, w >> 30];
    }
    let step = l.bits as usize / 8;
    for (ch, out) in v.iter_mut().enumerate().take(l.channels as usize) {
        let b = &raw[ch * step..ch * step + step];
        *out = match (step, l.signed) {
            (1, true) => b[0] as i8 as i32 as u32,
            (1, false) => b[0] as u32,
            (2, true) => i16::from_le_bytes([b[0], b[1]]) as i32 as u32,
            (2, false) => u16::from_le_bytes([b[0], b[1]]) as u32,
            _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        };
    }
    v
}

/// `v` (signed when `signed`) saturated to `bits` bits of that signedness.
fn saturate(v: u32, bits: u32, signed: bool) -> u32 {
    if signed {
        let (lo, hi) = (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1);
        (v as i32 as i64).clamp(lo, hi) as i32 as u32
    } else {
        let hi = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
        v.min(hi)
    }
}

/// The stored bytes of integer values `v` in an integer image, each
/// saturated to the channel's width (dEQP's reference saturates too).
fn encode_int(img: &Image, v: [u32; 4], raw: &mut [u8; 16]) {
    let Some(l) = gles_surf_int_layout(img.surf) else { return };
    if l.bits == 10 {
        let w = saturate(v[0], 10, false)
            | saturate(v[1], 10, false) << 10
            | saturate(v[2], 10, false) << 20
            | saturate(v[3], 2, false) << 30;
        raw[..4].copy_from_slice(&w.to_le_bytes());
        return;
    }
    let step = l.bits as usize / 8;
    for (ch, &x) in v.iter().enumerate().take(l.channels as usize) {
        let s = saturate(x, l.bits as u32, l.signed).to_le_bytes();
        raw[ch * step..ch * step + step].copy_from_slice(&s[..step]);
    }
}

/// The float value of the source at (sx, sy) of the cut source `cut`,
/// bilinearly filtered with clamp-to-edge inside it (tcu's LINEAR sample of
/// unnormalised coordinates).
fn sample_linear(c: &GlContext, src: &Image, cut: Rect, sx: f32, sy: f32, ok: &mut bool) -> [f32; 4] {
    let u = sx - 0.5;
    let v = sy - 0.5;
    let (fu, fv) = (floor(u), floor(v));
    let (a, b) = (u - fu as f32, v - fv as f32);
    let clamp = |i: i64, n: i64| i.clamp(0, n - 1);
    let (x0, x1) = (clamp(fu, cut.w), clamp(fu + 1, cut.w));
    let (y0, y1) = (clamp(fv, cut.h), clamp(fv + 1, cut.h));
    let mut raw = [0u8; 16];
    let mut px = |x: i64, y: i64| {
        *ok &= load(c, src, (cut.x + x) as u32, (cut.y + y) as u32, &mut raw);
        decode_float(src, &raw)
    };
    let (p00, p10, p01, p11) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    core::array::from_fn(|ch| {
        p00[ch] * (1.0 - a) * (1.0 - b) + p10[ch] * a * (1.0 - b) + p01[ch] * (1.0 - a) * b + p11[ch] * a * b
    })
}

/// floor() of an f32 (no libm in this no_std crate), as an integer.
fn floor(v: f32) -> i64 {
    let t = v as i64; /* truncates towards zero; saturates out of range */
    if (t as f32) > v {
        t - 1
    } else {
        t
    }
}

/// Copy the colour of `src` (cut to `src_cut`) into `dst` (its cut
/// rectangle `dst_cut`). False when the platform failed a read or a write.
fn copy_colour(
    c: &mut GlContext,
    src: &Image,
    src_cut: Rect,
    dst: &Image,
    dst_cut: Rect,
    k: &Corners,
    linear: bool,
) -> bool {
    let ax = Axis::new(k.src[0], k.src[2], k.dst[0], k.dst[2], src_cut.x, dst_cut.x);
    let ay = Axis::new(k.src[1], k.src[3], k.dst[1], k.dst[3], src_cut.y, dst_cut.y);
    let int = src.kind != ColorKind::Float;
    let mut ok = true;
    let mut raw = [0u8; 16];
    let mut out = [0u8; 16];
    for yo in 0..dst_cut.h {
        let sy = ay.at(yo);
        if sy < 0.0 || sy >= src_cut.h as f32 {
            continue; /* outside the cut source: left as it is */
        }
        for xo in 0..dst_cut.w {
            let sx = ax.at(xo);
            if sx < 0.0 || sx >= src_cut.w as f32 {
                continue;
            }
            if int {
                ok &= load(c, src, (src_cut.x + floor(sx)) as u32, (src_cut.y + floor(sy)) as u32, &mut raw);
                encode_int(dst, decode_int(src, &raw), &mut out);
            } else {
                let v = if linear {
                    sample_linear(c, src, src_cut, sx, sy, &mut ok)
                } else {
                    ok &= load(c, src, (src_cut.x + floor(sx)) as u32, (src_cut.y + floor(sy)) as u32, &mut raw);
                    decode_float(src, &raw)
                };
                encode_float(dst, v, &mut out);
            }
            ok &= store(c, dst, (dst_cut.x + xo) as u32, (dst_cut.y + yo) as u32, &out);
        }
    }
    ok
}

/// "Said it once" for the depth/stencil refusal.
static WARNED_DS: Global<bool> = Global::new(false);

/// Unit tests: re-arm the log line.
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: single-threaded by contract.
    unsafe { *WARNED_DS.get() = false };
}

/// glBlitFramebuffer on the current context, the READ framebuffer `read`
/// and the bound DRAW framebuffer.
#[allow(clippy::too_many_arguments)]
pub(super) fn blit_framebuffer(c: &mut GlContext, k: Corners, mut mask: GLbitfield, filter: GLenum) {
    let read = framebuffer::read_framebuffer(c);
    let draw = c.bound_framebuffer;

    /* The errors in Mesa's order (main/blit.c), which is the order of ES
     * 3.0.6 4.3.3's list: completeness first, then the arguments, then the
     * buffers. */
    if !fb_complete(c, read) || !fb_complete(c, draw) {
        c.set_error(GL_INVALID_FRAMEBUFFER_OPERATION, FN);
        return;
    }
    if filter != GL_NEAREST && filter != GL_LINEAR {
        c.set_error(GL_INVALID_ENUM, FN);
        return;
    }
    if mask & !(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) != 0 {
        c.set_error(GL_INVALID_VALUE, FN);
        return;
    }
    if mask & (GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) != 0 && filter != GL_NEAREST {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    /* "If SAMPLE_BUFFERS for the draw framebuffer is greater than zero, an
     * INVALID_OPERATION error is generated"; a multisample read framebuffer
     * is resolved only between identical rectangles of identical formats */
    let read_ms = fb_samples(c, read) > 0;
    if fb_samples(c, draw) > 0 || (read_ms && k.src != k.dst) {
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    /* "If a buffer is specified in mask and does not exist in both the read
     * and draw framebuffers, the corresponding bit is silently ignored." */
    let src = read_image(c, read);
    let (dsts, ndst) = draw_images(c);
    if mask & GL_COLOR_BUFFER_BIT != 0 {
        match src {
            Some(s) if ndst > 0 => {
                for d in dsts.iter().flatten() {
                    /* the same image (not another level, layer or face) */
                    let same = d.loc == s.loc && d.off == s.off;
                    let ms_mismatch = read_ms && linear_format(d.internalformat) != linear_format(s.internalformat);
                    if same || d.kind != s.kind || ms_mismatch {
                        c.set_error(GL_INVALID_OPERATION, FN);
                        return;
                    }
                }
                if filter == GL_LINEAR && s.kind != ColorKind::Float {
                    c.set_error(GL_INVALID_OPERATION, FN);
                    return;
                }
            }
            _ => mask &= !GL_COLOR_BUFFER_BIT,
        }
    }
    for (bit, stencil) in [(GL_STENCIL_BUFFER_BIT, true), (GL_DEPTH_BUFFER_BIT, false)] {
        if mask & bit == 0 {
            continue;
        }
        let (Some(r), Some(d)) = (ds_format(c, read, stencil), ds_format(c, draw, stencil)) else {
            mask &= !bit;
            continue;
        };
        /* the plane blitted must match, and the other one where both have it */
        let depth_match = r.fmt.0 == d.fmt.0 && r.fmt.1 == d.fmt.1;
        let stencil_match = r.fmt.2 == d.fmt.2;
        let ok = if stencil {
            stencil_match && (r.fmt.0 == 0 || d.fmt.0 == 0 || depth_match)
        } else {
            depth_match && (r.fmt.2 == 0 || d.fmt.2 == 0 || stencil_match)
        };
        /* the same image read and written ("identical buffers") */
        if r.image == d.image || !ok {
            c.set_error(GL_INVALID_OPERATION, FN);
            return;
        }
    }
    if mask & (GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) != 0 {
        // SAFETY: single-threaded by contract; no other reference is live.
        let warned = unsafe { WARNED_DS.get() };
        if !*warned {
            *warned = true;
            glog!("glBlitFramebuffer: depth/stencil blits are not implemented (GL_INVALID_OPERATION)");
        }
        c.set_error(GL_INVALID_OPERATION, FN);
        return;
    }
    let Some(src) = src.filter(|_| mask & GL_COLOR_BUFFER_BIT != 0) else { return };

    let src_cut = Rect::of(k.src[0], k.src[1], k.src[2], k.src[3]).cut(Rect::of_image(&src));
    if src_cut.empty() {
        return;
    }
    let scaled = {
        let (s, d) =
            (Rect::of(k.src[0], k.src[1], k.src[2], k.src[3]), Rect::of(k.dst[0], k.dst[1], k.dst[2], k.dst[3]));
        s.w != d.w || s.h != d.h
    };
    let scissor = c.scissor_test.then(|| {
        let [x, y, w, h] = c.scissor;
        Rect { x: x as i64, y: y as i64, w: w.max(0) as i64, h: h.max(0) as i64 }
    });
    let dst_rect = Rect::of(k.dst[0], k.dst[1], k.dst[2], k.dst[3]);
    if dst_rect.empty() {
        return;
    }
    /* The GPU's work reaches memory, once for the read and every write. Lost
     * work leaves the draw buffers as they were, with no GL error (as
     * glReadPixels and glCopyTex*: none fits). */
    if !framebuffer::gpu_idle_for_cpu(c) {
        return;
    }
    invalidate(c, &src);
    let mut ok = true;
    for d in dsts.iter().flatten() {
        let mut cut = dst_rect.cut(Rect::of_image(d));
        if let Some(s) = scissor {
            cut = cut.cut(s);
        }
        if cut.empty() {
            continue;
        }
        /* the CPU's lines of the image may be stale: dropped before the
         * pixels are written, so the write-back carries only fresh bytes */
        invalidate(c, d);
        ok &= copy_colour(c, &src, src_cut, d, cut, &k, filter == GL_LINEAR && scaled);
        write_back(c, d);
        if let Some((ti, level)) = d.tex {
            super::texture::note_blit_written(c, ti, level);
        }
    }
    if !ok {
        /* no GL error fits a failed copy exactly; OUT_OF_MEMORY is the "could
         * not be carried out" one, as for glReadPixels */
        c.set_error(GL_OUT_OF_MEMORY, FN);
    }
}

/* ES 3.0 only: under `report-es2` the command is GL_INVALID_OPERATION
 * (`framebuffer::tests::blit_framebuffer_is_es3_only`). */
#[cfg(all(test, not(feature = "report-es2")))]
mod tests {
    //! Each failure class of dEQP-GLES3 fbo.blit against a software model of
    //! ES 3.0.6 4.3.3 written apart from the code above: the conversions are
    //! the spec's formulas in f64 (unorm `round(clamp(f) * (2^b - 1))`, the
    //! eight-bit storage keeping `round(q * 255)` of the b-bit value), the
    //! sample points chosen where f32 and f64 agree exactly.
    use super::*;
    use crate::gl::context::tex_pitch;
    use crate::gl::framebuffer::{
        glBindFramebuffer, glBindRenderbuffer, glBlitFramebuffer, glDrawBuffers, glFramebufferRenderbuffer,
        glFramebufferTexture2D, glGenFramebuffers, glGenRenderbuffers, glReadBuffer, glReadPixels,
        glRenderbufferStorage, glRenderbufferStorageMultisample,
    };
    use crate::gl::state::{glClear, glClearColor, glEnable, glScissor};
    use crate::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
    use crate::testutil::TestGl;

    /// A framebuffer with a `format` renderbuffer of `w` x `h` at colour
    /// attachment 0, bound to GL_FRAMEBUFFER: (framebuffer, renderbuffer).
    fn rbo_fbo(format: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
        let (mut fb, mut rb) = (0, 0);
        // SAFETY: plain GL calls with room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        (fb, rb)
    }

    fn rb_gpu(gl: &TestGl, rb: GLuint) -> u64 {
        gl.ctx().objs_ref().renderbuffers[rb as usize].mem.gpu
    }

    /// The GPU renders a `w` x `h` image of `bpp`-byte pixels into `gpu`
    /// (rows at the pair-padded pitch, bottom-up as an FBO's), pixel (x, y)
    /// being `f(x, y)`.
    fn render(gl: &TestGl, gpu: u64, w: u32, h: u32, bpp: usize, f: impl Fn(u32, u32) -> Vec<u8>) {
        let pitch = tex_pitch(w) as usize;
        let mut bytes = vec![0u8; ((h as usize - 1) * pitch + w as usize) * bpp];
        for y in 0..h {
            for x in 0..w {
                let p = f(x, y);
                assert_eq!(p.len(), bpp);
                bytes[(y as usize * pitch + x as usize) * bpp..][..bpp].copy_from_slice(&p);
            }
        }
        gl.gpu_render(gpu, &bytes);
    }

    /// Pixel (x, y) of a `w`-wide image of `bpp`-byte pixels at `gpu`, as
    /// the GPU sees memory (what a CPU write left unflushed is not there).
    fn stored(gl: &TestGl, gpu: u64, w: u32, bpp: usize, x: u32, y: u32) -> Vec<u8> {
        let off = (y as usize * tex_pitch(w) as usize + x as usize) * bpp;
        gl.gpu_bytes(gpu + off as u64, bpp)
    }

    fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield, filter: GLenum) {
        // SAFETY: plain GL call.
        unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, filter) };
    }

    fn bind(read: GLuint, draw: GLuint) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
        }
    }

    /// The spec's unorm encode of `f` at `bits` bits, as eight-bit storage
    /// holds it.
    fn unorm_ref(f: f64, bits: u32) -> u8 {
        let max = ((1u32 << bits) - 1) as f64;
        let q = (f.clamp(0.0, 1.0) * max + 0.5).floor() / max;
        (q * 255.0 + 0.5).floor() as u8
    }

    fn rgba8(x: u32, y: u32) -> Vec<u8> {
        vec![(x * 30 + 7) as u8, (y * 40 + 3) as u8, (x * y * 9) as u8, (200 - x - y) as u8]
    }

    /* ---- coherence and the plain copy ---------------------------------- */

    /// A same-format copy is exact; the source is read after the GPU wrote
    /// it (the mock's CPU view is stale until invalidated), and the
    /// destination reaches memory (the GPU's view), with nothing left
    /// unflushed.
    #[test]
    fn a_copy_reads_what_the_gpu_rendered_and_writes_memory_back() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 8, 8);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, 8, 8);
        render(&gl, rb_gpu(&gl, src_rb), 8, 8, 4, rgba8);
        bind(src_fb, dst_fb);
        blit([0, 0, 8, 8], [0, 0, 8, 8], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let g = rb_gpu(&gl, dst_rb);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(stored(&gl, g, 8, 4, x, y), rgba8(x, y), "({x},{y})");
            }
        }
        assert_eq!(gl.unflushed(g, (7 * 8 + 8) * 4), None);
        /* and glReadPixels of the draw framebuffer agrees */
        bind(dst_fb, dst_fb);
        let mut px = [0u8; 4];
        // SAFETY: room for one RGBA pixel.
        unsafe { glReadPixels(3, 5, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(px.to_vec(), rgba8(3, 5));
        assert!(gl.flushes() > 0 && gl.finishes() > 0, "the GPU work was waited for");
    }

    /// A clear still pending for the draw framebuffer goes out before the
    /// copy (it comes first in GL's order and would otherwise wipe it).
    #[test]
    fn a_pending_clear_goes_out_before_the_copy() {
        let gl = TestGl::new(8, 8);
        let (src_fb, _) = rbo_fbo(GL_RGBA8, 4, 4);
        let (dst_fb, _) = rbo_fbo(GL_RGBA8, 4, 4);
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(1.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let p = crate::gl::draw::pending_clear();
        assert!(p.color, "glClear is held for the next draw");
        bind(src_fb, dst_fb);
        gl.clear_recordings();
        blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(!crate::gl::draw::pending_clear().color, "the clear went out");
        assert!(!gl.draws().is_empty(), "as a draw of its own, before the copy");
    }

    /// Lost GPU work leaves the draw buffer as it was and raises nothing
    /// (as glReadPixels).
    #[test]
    fn lost_gpu_work_leaves_the_destination_alone() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 2, 2);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, 2, 2);
        render(&gl, rb_gpu(&gl, src_rb), 2, 2, 4, |_, _| vec![9; 4]);
        render(&gl, rb_gpu(&gl, dst_rb), 2, 2, 4, |_, _| vec![1; 4]);
        bind(src_fb, dst_fb);
        gl.set_finish_result(-crate::util::EIO);
        blit([0, 0, 2, 2], [0, 0, 2, 2], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        gl.set_finish_result(0);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 2, 4, 1, 1), vec![1; 4]);
    }

    /* ---- conversion ------------------------------------------------------ */

    /// RGBA8 into RGB565: each channel rounded to the format's 5/6/5 bits
    /// (stored at eight), alpha 1 (the format has none).
    #[test]
    fn rgba8_into_rgb565_rounds_to_its_bits() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 4, 1);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGB565, 4, 1);
        let vals: [[u8; 4]; 4] = [[0, 0, 0, 0], [255, 255, 255, 7], [100, 101, 102, 50], [4, 2, 250, 128]];
        render(&gl, rb_gpu(&gl, src_rb), 4, 1, 4, |x, _| vals[x as usize].to_vec());
        bind(src_fb, dst_fb);
        blit([0, 0, 4, 1], [0, 0, 4, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for (x, v) in vals.iter().enumerate() {
            let f = |c: u8| c as f64 / 255.0;
            let want = vec![unorm_ref(f(v[0]), 5), unorm_ref(f(v[1]), 6), unorm_ref(f(v[2]), 5), 255];
            assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 4, 4, x as u32, 0), want, "pixel {x}");
        }
        /* RGBA4 and RGB5_A1 the same way, alpha included */
        let (dst4, rb4) = rbo_fbo(GL_RGBA4, 4, 1);
        let (dst51, rb51) = rbo_fbo(GL_RGB5_A1, 4, 1);
        for (fb, rb, bits) in [(dst4, rb4, [4, 4, 4, 4]), (dst51, rb51, [5, 5, 5, 1])] {
            bind(src_fb, fb);
            blit([0, 0, 4, 1], [0, 0, 4, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
            for (x, v) in vals.iter().enumerate() {
                let want: Vec<u8> = (0..4).map(|ch| unorm_ref(v[ch] as f64 / 255.0, bits[ch])).collect();
                assert_eq!(stored(&gl, rb_gpu(&gl, rb), 4, 4, x as u32, 0), want, "pixel {x} at {bits:?}");
            }
        }
    }

    fn half(f: f32) -> [u8; 2] {
        crate::gl::texture::float_to_half(f).to_le_bytes()
    }

    /// A half-float source into RGBA8 clamps to [0, 1] and rounds; RGBA8
    /// into RGBA16F widens (v / 255, as a half).
    #[test]
    fn float_and_unorm_convert_both_ways() {
        let gl = TestGl::new(8, 8);
        let (f_fb, f_rb) = rbo_fbo(GL_RGBA16F, 2, 1);
        let (u_fb, u_rb) = rbo_fbo(GL_RGBA8, 2, 1);
        let src = [[2.0f32, -1.0, 0.5, 0.25], [0.0, 1.0, 0.75, 0.1]];
        render(&gl, rb_gpu(&gl, f_rb), 2, 1, 8, |x, _| src[x as usize].iter().flat_map(|&v| half(v)).collect());
        bind(f_fb, u_fb);
        blit([0, 0, 2, 1], [0, 0, 2, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for (x, v) in src.iter().enumerate() {
            let h = |f: f32| framebuffer::half_to_float(u16::from_le_bytes(half(f))) as f64;
            let want: Vec<u8> = v.iter().map(|&f| unorm_ref(h(f), 8)).collect();
            assert_eq!(stored(&gl, rb_gpu(&gl, u_rb), 2, 4, x as u32, 0), want, "pixel {x}");
        }
        assert_eq!(stored(&gl, rb_gpu(&gl, u_rb), 2, 4, 0, 0), vec![255, 0, 128, 64]);
        /* and back: 255 is 1.0, 0 is 0.0, 128 is 128/255 as a half */
        render(&gl, rb_gpu(&gl, u_rb), 2, 1, 4, |_, _| vec![255, 0, 128, 64]);
        bind(u_fb, f_fb);
        blit([0, 0, 2, 1], [0, 0, 2, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        let want: Vec<u8> = [1.0f32, 0.0, 128.0 / 255.0, 64.0 / 255.0].iter().flat_map(|&v| half(v)).collect();
        assert_eq!(stored(&gl, rb_gpu(&gl, f_rb), 2, 8, 1, 0), want);
    }

    /// A channel the format lacks reads 0 (alpha 1) on the source whatever
    /// its storage holds there, and is written 0 (alpha 1) on the
    /// destination.
    #[test]
    fn channels_a_format_lacks_read_and_write_zero_and_alpha_one() {
        let gl = TestGl::new(8, 8);
        let (r16_fb, r16_rb) = rbo_fbo(GL_R16F, 1, 1);
        let (rg8_fb, rg8_rb) = rbo_fbo(GL_RG8, 1, 1);
        let (out_fb, out_rb) = rbo_fbo(GL_RGBA8, 1, 1);
        let (r8_fb, r8_rb) = rbo_fbo(GL_R8, 1, 1);
        render(&gl, rb_gpu(&gl, r16_rb), 1, 1, 2, |_, _| half(0.5).to_vec());
        /* the shader wrote B and A too; RG8 has neither */
        render(&gl, rb_gpu(&gl, rg8_rb), 1, 1, 4, |_, _| vec![10, 20, 30, 40]);
        render(&gl, rb_gpu(&gl, out_rb), 1, 1, 4, |_, _| vec![77; 4]);
        bind(r16_fb, out_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(stored(&gl, rb_gpu(&gl, out_rb), 1, 4, 0, 0), vec![128, 0, 0, 255]);
        bind(rg8_fb, out_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(stored(&gl, rb_gpu(&gl, out_rb), 1, 4, 0, 0), vec![10, 20, 0, 255]);
        render(&gl, rb_gpu(&gl, out_rb), 1, 1, 4, |_, _| vec![50, 60, 70, 80]);
        bind(out_fb, r8_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(stored(&gl, rb_gpu(&gl, r8_rb), 1, 4, 0, 0), vec![50, 0, 0, 255]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// Integer to integer copies values, saturated to the destination's
    /// width and signedness; the channels the destination lacks are not
    /// stored.
    #[test]
    fn integer_copies_saturate_to_the_destination() {
        let gl = TestGl::new(8, 8);
        let (i32_fb, i32_rb) = rbo_fbo(GL_RGBA32I, 1, 1);
        let (i8_fb, i8_rb) = rbo_fbo(GL_R8I, 1, 1);
        let (i16_fb, i16_rb) = rbo_fbo(GL_RG16I, 1, 1);
        let words: [i32; 4] = [-200, 40000, 5, -1];
        render(&gl, rb_gpu(&gl, i32_rb), 1, 1, 16, |_, _| words.iter().flat_map(|w| w.to_le_bytes()).collect());
        bind(i32_fb, i8_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(stored(&gl, rb_gpu(&gl, i8_rb), 1, 1, 0, 0), vec![(-128i8) as u8]);
        bind(i32_fb, i16_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        let want: Vec<u8> = [-200i16, 32767].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(stored(&gl, rb_gpu(&gl, i16_rb), 1, 4, 0, 0), want);
        /* unsigned: RGBA16UI into RGBA8UI and RGB10_A2UI */
        let (u16_fb, u16_rb) = rbo_fbo(GL_RGBA16UI, 1, 1);
        let (u8_fb, u8_rb) = rbo_fbo(GL_RGBA8UI, 1, 1);
        let (a2_fb, a2_rb) = rbo_fbo(GL_RGB10_A2UI, 1, 1);
        let v: [u16; 4] = [1028, 7, 255, 9];
        render(&gl, rb_gpu(&gl, u16_rb), 1, 1, 8, |_, _| v.iter().flat_map(|w| w.to_le_bytes()).collect());
        bind(u16_fb, u8_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(stored(&gl, rb_gpu(&gl, u8_rb), 1, 4, 0, 0), vec![255, 7, 255, 9]);
        bind(u16_fb, a2_fb);
        blit([0, 0, 1, 1], [0, 0, 1, 1], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        let w = 1023 | 7 << 10 | 255 << 20 | 3u32 << 30;
        assert_eq!(stored(&gl, rb_gpu(&gl, a2_rb), 1, 4, 0, 0), w.to_le_bytes().to_vec());
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- validation ---------------------------------------------------- */

    /// ES 3.0.6 4.3.3's errors, in Mesa's order; none of them writes.
    #[test]
    fn the_errors_of_4_3_3() {
        let gl = TestGl::new(8, 8);
        let (f_fb, _) = rbo_fbo(GL_RGBA8, 4, 4);
        let (f2_fb, f2_rb) = rbo_fbo(GL_RGBA8, 4, 4);
        let (u_fb, _) = rbo_fbo(GL_RGBA8UI, 4, 4);
        let (u2_fb, _) = rbo_fbo(GL_RGBA16UI, 4, 4);
        let (i_fb, _) = rbo_fbo(GL_RGBA8I, 4, 4);
        render(&gl, rb_gpu(&gl, f2_rb), 4, 4, 4, |_, _| vec![3; 4]);
        let all = [0, 0, 4, 4];
        let cases: &[(GLuint, GLuint, GLbitfield, GLenum, GLenum, &str)] = &[
            (f_fb, f2_fb, GL_COLOR_BUFFER_BIT, GL_LINEAR_MIPMAP_NEAREST, GL_INVALID_ENUM, "filter"),
            (f_fb, f2_fb, GL_COLOR_BUFFER_BIT | 0x8, GL_NEAREST, GL_INVALID_VALUE, "mask"),
            (f_fb, f2_fb, GL_DEPTH_BUFFER_BIT, GL_LINEAR, GL_INVALID_OPERATION, "LINEAR depth"),
            (f_fb, f2_fb, GL_STENCIL_BUFFER_BIT, GL_LINEAR, GL_INVALID_OPERATION, "LINEAR stencil"),
            (u_fb, f2_fb, GL_COLOR_BUFFER_BIT, GL_NEAREST, GL_INVALID_OPERATION, "uint into fixed"),
            (f_fb, u_fb, GL_COLOR_BUFFER_BIT, GL_NEAREST, GL_INVALID_OPERATION, "fixed into uint"),
            (i_fb, u_fb, GL_COLOR_BUFFER_BIT, GL_NEAREST, GL_INVALID_OPERATION, "int into uint"),
            (u_fb, i_fb, GL_COLOR_BUFFER_BIT, GL_NEAREST, GL_INVALID_OPERATION, "uint into int"),
            (u_fb, u2_fb, GL_COLOR_BUFFER_BIT, GL_LINEAR, GL_INVALID_OPERATION, "LINEAR integer"),
            (f2_fb, f2_fb, GL_COLOR_BUFFER_BIT, GL_NEAREST, GL_INVALID_OPERATION, "identical buffers"),
        ];
        for &(r, d, mask, filter, err, what) in cases {
            bind(r, d);
            blit(all, all, mask, filter);
            assert_eq!(gl.error(), err, "{what}");
        }
        assert_eq!(stored(&gl, rb_gpu(&gl, f2_rb), 4, 4, 0, 0), vec![3; 4], "no error wrote");
        /* what is allowed: uint into a wider uint, LINEAR between fixed formats */
        bind(u_fb, u2_fb);
        blit(all, all, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        bind(f_fb, f2_fb);
        blit(all, [0, 0, 2, 2], GL_COLOR_BUFFER_BIT, GL_LINEAR);
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* an incomplete framebuffer, read or draw, outranks the rest */
        let mut empty = 0;
        // SAFETY: room for one name.
        unsafe { glGenFramebuffers(1, &mut empty) };
        bind(empty, f2_fb);
        blit(all, all, GL_COLOR_BUFFER_BIT, 0);
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
        bind(f_fb, empty);
        blit(all, all, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_INVALID_FRAMEBUFFER_OPERATION);
    }

    /// A draw framebuffer with sample buffers is refused; a read one only
    /// resolves between identical rectangles of one format.
    #[test]
    fn sample_buffers_restrict_the_blit() {
        let gl = TestGl::new(8, 8);
        let (ms_fb, ms_rb) = rbo_fbo(GL_RGBA8, 4, 4);
        // SAFETY: plain GL calls; the renderbuffer is bound.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, ms_rb);
            glRenderbufferStorageMultisample(GL_RENDERBUFFER, 1, GL_RGBA8, 4, 4);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let (f_fb, _) = rbo_fbo(GL_RGBA8, 4, 4);
        let (f4_fb, _) = rbo_fbo(GL_RGBA4, 4, 4);
        let all = [0, 0, 4, 4];
        bind(f_fb, ms_fb);
        blit(all, all, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "draw framebuffer with sample buffers");
        bind(ms_fb, f_fb);
        blit(all, [1, 0, 5, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "a resolve between other rectangles");
        bind(ms_fb, f4_fb);
        blit(all, all, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "a resolve into another format");
        bind(ms_fb, f_fb);
        blit(all, all, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR, "a resolve");
    }

    /// A buffer named in the mask that one side lacks is ignored: colour
    /// with the read buffer GL_NONE, depth with no depth attachment.
    /// Depth/stencil blits between matching buffers are refused until the
    /// back ends can write their depth/stencil (said once); mismatching
    /// formats are the spec's GL_INVALID_OPERATION.
    #[test]
    fn missing_buffers_are_ignored_and_depth_stencil_is_refused() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 2, 2);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, 2, 2);
        render(&gl, rb_gpu(&gl, src_rb), 2, 2, 4, |_, _| vec![9; 4]);
        render(&gl, rb_gpu(&gl, dst_rb), 2, 2, 4, |_, _| vec![1; 4]);
        // SAFETY: plain GL call on the bound (destination) framebuffer, then the source's.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, src_fb);
            glReadBuffer(GL_NONE);
        }
        bind(src_fb, dst_fb);
        blit([0, 0, 2, 2], [0, 0, 2, 2], GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 2, 4, 0, 0), vec![1; 4]);
        // SAFETY: as above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, src_fb);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
        }
        /* matching D24S8 on both sides */
        let ds = |fb: GLuint, format: GLenum| {
            let mut rb = 0;
            // SAFETY: plain GL calls with room for one name.
            unsafe {
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, format, 2, 2);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
            }
        };
        ds(src_fb, GL_DEPTH24_STENCIL8);
        ds(dst_fb, GL_DEPTH24_STENCIL8);
        gl.logs();
        bind(src_fb, dst_fb);
        for _ in 0..2 {
            blit([0, 0, 2, 2], [0, 0, 2, 2], GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT, GL_NEAREST);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        let logs = gl.logs();
        assert_eq!(logs.iter().filter(|l| l.contains("depth/stencil blits are not implemented")).count(), 1);
        assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 2, 4, 0, 0), vec![1; 4], "the refused call wrote nothing");
        ds(dst_fb, GL_DEPTH32F_STENCIL8);
        bind(src_fb, dst_fb);
        blit([0, 0, 2, 2], [0, 0, 2, 2], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "stencil with mismatching depth formats");
    }

    /* ---- rectangles ---------------------------------------------------- */

    /// The software model: dst(x, y) = src(floor(map(x + 0.5)), ...) for the
    /// destination pixels of the cut rectangle whose sample point lies in the
    /// cut source, every other pixel `keep`.
    #[allow(clippy::too_many_arguments)]
    fn nearest_model(
        src: &dyn Fn(u32, u32) -> Vec<u8>,
        sw: u32,
        sh: u32,
        s: [i32; 4],
        dw: u32,
        dh: u32,
        d: [i32; 4],
        keep: &[u8],
    ) -> Vec<Vec<u8>> {
        let mut out = vec![keep.to_vec(); (dw * dh) as usize];
        for y in 0..dh as i32 {
            for x in 0..dw as i32 {
                let inside = |v: i32, a: i32, b: i32| v >= a.min(b) && v < a.max(b);
                if !inside(x, d[0], d[2]) || !inside(y, d[1], d[3]) {
                    continue;
                }
                let map = |v: i32, d0: i32, d1: i32, s0: i32, s1: i32| {
                    s0 as f64 + (v as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64
                };
                let (sx, sy) = (map(x, d[0], d[2], s[0], s[2]), map(y, d[1], d[3], s[1], s[3]));
                let inside_src =
                    |v: f64, a: i32, b: i32, n: u32| v >= a.min(b).max(0) as f64 && v < a.max(b).min(n as i32) as f64;
                if !inside_src(sx, s[0], s[2], sw) || !inside_src(sy, s[1], s[3], sh) {
                    continue;
                }
                out[(y as u32 * dw + x as u32) as usize] = src(sx.floor() as u32, sy.floor() as u32);
            }
        }
        out
    }

    fn check_rect(s: [i32; 4], d: [i32; 4], (sw, sh): (u32, u32), (dw, dh): (u32, u32)) {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, sw as i32, sh as i32);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, dw as i32, dh as i32);
        render(&gl, rb_gpu(&gl, src_rb), sw, sh, 4, rgba8);
        let keep = vec![0xEE; 4];
        render(&gl, rb_gpu(&gl, dst_rb), dw, dh, 4, |_, _| keep.clone());
        bind(src_fb, dst_fb);
        blit(s, d, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let want = nearest_model(&rgba8, sw, sh, s, dw, dh, d, &keep);
        for y in 0..dh {
            for x in 0..dw {
                let got = stored(&gl, rb_gpu(&gl, dst_rb), dw, 4, x, y);
                assert_eq!(got, want[(y * dw + x) as usize], "{s:?} -> {d:?} at ({x},{y})");
            }
        }
    }

    /// Reversed rectangles flip; x and y independently, on either side.
    #[test]
    fn reversed_rectangles_flip() {
        check_rect([4, 0, 0, 4], [0, 0, 4, 4], (4, 4), (4, 4));
        check_rect([0, 4, 4, 0], [0, 0, 4, 4], (4, 4), (4, 4));
        check_rect([0, 0, 4, 4], [4, 4, 0, 0], (4, 4), (4, 4));
        check_rect([4, 4, 0, 0], [4, 4, 0, 0], (4, 4), (4, 4));
    }

    /// NEAREST scaling up (each texel covers 2x2) and down (the texel under
    /// the centre).
    #[test]
    fn nearest_scales_up_and_down() {
        check_rect([0, 0, 2, 2], [0, 0, 4, 4], (2, 2), (4, 4));
        check_rect([0, 0, 4, 4], [1, 1, 3, 3], (4, 4), (4, 4));
        check_rect([1, 0, 3, 4], [0, 0, 8, 2], (4, 4), (8, 2));
    }

    /// Rectangles partly outside: the source is cut to the read buffer and
    /// the destination pixels that map outside it keep their colour; the
    /// destination is cut to the draw buffer; the mapping is that of the
    /// uncut rectangles (proportional), not of the cut ones.
    #[test]
    fn out_of_bounds_rectangles_cut_and_keep_the_scale() {
        check_rect([-2, -2, 6, 6], [0, 0, 8, 8], (4, 4), (8, 8));
        check_rect([-2, -2, 6, 6], [0, 0, 4, 4], (4, 4), (4, 4));
        check_rect([0, 0, 4, 4], [-4, 2, 4, 10], (4, 4), (8, 8));
        check_rect([6, -1, -2, 3], [8, 0, 0, 8], (4, 4), (8, 8));
    }

    /// The scissor box cuts the destination; the colour mask does not apply
    /// (a blit passes only the pixel ownership and scissor tests).
    #[test]
    fn the_scissor_cuts_the_destination() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 4, 4);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, 4, 4);
        render(&gl, rb_gpu(&gl, src_rb), 4, 4, 4, rgba8);
        render(&gl, rb_gpu(&gl, dst_rb), 4, 4, 4, |_, _| vec![0xEE; 4]);
        bind(src_fb, dst_fb);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(1, 2, 2, 5);
            crate::gl::state::glColorMask(0, 0, 0, 0);
        }
        blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        for y in 0..4 {
            for x in 0..4 {
                let inside = (1..3).contains(&x) && (2..4).contains(&y);
                let want = if inside { rgba8(x, y) } else { vec![0xEE; 4] };
                assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 4, 4, x, y), want, "({x},{y})");
            }
        }
    }

    /// LINEAR filters with texel centres at i + 0.5 and clamp-to-edge inside
    /// the cut source, only where the sizes differ: a 2-texel ramp doubled
    /// reads 0, 1/4, 3/4, 1 of the way; a same-size flip stays NEAREST.
    #[test]
    fn linear_filters_where_the_sizes_differ() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 2, 1);
        let (dst_fb, dst_rb) = rbo_fbo(GL_RGBA8, 4, 1);
        render(&gl, rb_gpu(&gl, src_rb), 2, 1, 4, |x, _| vec![if x == 0 { 0 } else { 200 }, 0, 0, 255]);
        bind(src_fb, dst_fb);
        blit([0, 0, 2, 1], [0, 0, 4, 1], GL_COLOR_BUFFER_BIT, GL_LINEAR);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let want = [0.0, 0.25, 0.75, 1.0].map(|t: f64| unorm_ref(t * 200.0 / 255.0, 8));
        for (x, w) in want.iter().enumerate() {
            assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 4, 4, x as u32, 0)[0], *w, "pixel {x}");
        }
        blit([2, 0, 0, 1], [0, 0, 2, 1], GL_COLOR_BUFFER_BIT, GL_LINEAR);
        assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 4, 4, 0, 0)[0], 200);
        assert_eq!(stored(&gl, rb_gpu(&gl, dst_rb), 4, 4, 1, 0)[0], 0);
    }

    /* ---- the default framebuffer, draw buffers, textures ---------------- */

    /// The window surface is stored top-down: GL row y is its row h-1-y, as
    /// source and as destination.
    #[test]
    fn the_default_framebuffer_is_flipped_in_memory() {
        let gl = TestGl::new(4, 4);
        let (fb, rb) = rbo_fbo(GL_RGBA8, 4, 4);
        render(&gl, rb_gpu(&gl, rb), 4, 4, 4, rgba8);
        bind(fb, 0);
        blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let surf = gl.ctx().default_surface.gpu;
        for y in 0..4 {
            assert_eq!(stored(&gl, surf, 4, 4, 1, 3 - y), rgba8(1, y), "GL row {y}");
        }
        /* and back into a cleared FBO */
        render(&gl, rb_gpu(&gl, rb), 4, 4, 4, |_, _| vec![0; 4]);
        bind(0, fb);
        blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(stored(&gl, rb_gpu(&gl, rb), 4, 4, 2, 1), rgba8(2, 1));
    }

    /// Every draw buffer is written; one that is GL_NONE is not.
    #[test]
    fn every_draw_buffer_is_written() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 2, 2);
        render(&gl, rb_gpu(&gl, src_rb), 2, 2, 4, rgba8);
        let (dst_fb, rb0) = rbo_fbo(GL_RGBA8, 2, 2);
        let mut rbs = [0; 2];
        // SAFETY: plain GL calls with room for the names.
        unsafe {
            glGenRenderbuffers(2, rbs.as_mut_ptr());
            for (i, &rb) in rbs.iter().enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA16F, 2, 2);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1 + i as GLenum, GL_RENDERBUFFER, rb);
            }
            let bufs = [GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2];
            glDrawBuffers(3, bufs.as_ptr());
        }
        for &rb in &rbs {
            render(&gl, rb_gpu(&gl, rb), 2, 2, 8, |_, _| vec![0x11; 8]);
        }
        bind(src_fb, dst_fb);
        blit([0, 0, 2, 2], [0, 0, 2, 2], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(stored(&gl, rb_gpu(&gl, rb0), 2, 4, 1, 1), rgba8(1, 1));
        assert_eq!(stored(&gl, rb_gpu(&gl, rbs[0]), 2, 8, 1, 1), vec![0x11; 8], "GL_NONE");
        let want: Vec<u8> = rgba8(1, 1).iter().flat_map(|&c| half(c as f32 / 255.0)).collect();
        assert_eq!(stored(&gl, rb_gpu(&gl, rbs[1]), 2, 8, 1, 1), want);
    }

    /// A texture destination takes a new texel stamp (a backend copy of it
    /// is made again) and counts its level as written.
    #[test]
    fn a_texture_destination_is_restamped() {
        let gl = TestGl::new(8, 8);
        let (src_fb, src_rb) = rbo_fbo(GL_RGBA8, 4, 4);
        render(&gl, rb_gpu(&gl, src_rb), 4, 4, 4, rgba8);
        let (mut tex, mut fb) = (0, 0);
        // SAFETY: plain GL calls; null pixels only allocate.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        }
        let before = gl.ctx().objs_ref().textures[tex as usize].content_gen;
        bind(src_fb, fb);
        blit([0, 0, 4, 4], [0, 0, 4, 4], GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let t = &gl.ctx().objs_ref().textures[tex as usize];
        assert_ne!(t.content_gen, before);
        assert!(t.levels_written >= 1);
        assert_eq!(stored(&gl, t.mem.gpu, 4, 4, 3, 2), rgba8(3, 2));
    }
}
