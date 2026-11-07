// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a full-target glClear as a resolve-engine (RS) fill,
//! as a pure function (docs/notes/gc-slow-stream-diff.md, finding 1).
//!
//! The backend draws every glClear as a quad (`backend.rs` `emit_clear`).
//! Mesa etnaviv clears a whole surface through the RS instead: no shader, no
//! vertex fetch, the fill value a literal in the stream. This module decides
//! whether a clear may take that path and writes its command words; joining
//! it to the backend is item rs-clear-wire. It is not in `mod.rs` yet. It
//! depends on `super::regs` only, so a host test includes both by `#[path]`.
//!
//! Sources: Mesa 26.2.1 etnaviv, `src/gallium/drivers/etnaviv/`:
//!
//! - `etnaviv_rs.c:504-581` (`etna_clear_rs`): GL_FLUSH_CACHE COLOR|DEPTH
//!   and a stall RA->PE, the colour fills, GL_FLUSH_CACHE again when depth
//!   follows colour, the depth fill, and a last stall RA->PE.
//! - `:284-335` (`etna_rs_gen_clear_cmd`): RS format by bits a pixel (16
//!   A4R4G4B4, 32 A8R8G8B8), window = the padded size, dither all ones,
//!   CLEAR_CONTROL `BITS(bits) | MODE_ENABLED1`, fill values
//!   `{lo, hi, lo, hi}` of a 64-bit clear value.
//! - `:362-433` (`etna_blit_clear_color_rs`) and `etnaviv_clear_blit.c:91-111`
//!   (`etna_clear_blit_pack_rgba`): a colour of 4 bytes or less is replicated
//!   into both halves, so all four fill values are the pixel word (a 2-byte
//!   pixel doubled first).
//! - `:435-502` (`etna_blit_clear_zs_rs`): bits 0xffff for Z16 and X8Z24;
//!   for S8Z24 depth 0xeeee and stencil 0x1111, or-ed by what is cleared.
//!   The value is a 32-bit word (`translate_clear_depth_stencil`), so its
//!   fill values are `{v, 0, v, 0}`.
//! - `:68-187` (`etna_compile_rs_state`) and `:198-282`
//!   (`etna_submit_rs_state`, the RS_NEW_BASEADDR branch this core takes):
//!   the register order, the in-stream coalescing of consecutive registers
//!   into one LOAD_STATE (`etnaviv_emit.h:170-245`), RS_SINGLE_BUFFER around
//!   the kick on a SINGLE_BUFFER core. A clear has no source buffer, so
//!   RS_PIPE_SOURCE_ADDR is not written (`etna_coalsence_emit_reloc` skips a
//!   relocation without a bo) and RS_SOURCE_STRIDE is 0.
//! - `src/util/u_pack_color.h:546-627` (`util_pack_z`, `util_pack_z_stencil`):
//!   the depth word, `lrint(z * 0xffffff) << 8 | stencil` (round half to even).
//!
//! Our bring-up clear, `driver/src/vivante/gc_draw.c:217-259` (`gc_draw_clear`,
//! sample `gc_clear` PASS on the board), is the same sequence one register a
//! LOAD_STATE, with source = destination. The words here follow Mesa instead,
//! so that a `cs_ours`/`cs_golden` diff matches RS block for RS block.
//!
//! # What is and is not here
//!
//! - **Linear targets only**: RS_CONFIG has no TILED bit and the strides are
//!   the byte strides, unshifted. That is how this backend lays out colour and
//!   depth today. A uniform fill writes the same bytes in any layout, but the
//!   RS still walks the surface by the layout it is told; tiled targets are
//!   item tiled-depth and later, and change [`rs_config`] and the strides.
//! - **No tile status, no fast clear**: no TS_FLUSH_CACHE, no TS fill.
//! - **All or nothing**: if any plane of the clear may not take the RS, the
//!   whole clear stays a quad ([`NotRs`]). Mesa's state tracker splits per
//!   plane; this backend's clear quad already handles every mix.
//! - The pad word after a LOAD_STATE with an even count is [`PAD`]
//!   (GC_OP_NOP, as `cmd.rs` writes); Mesa writes 0xdeadbeef. The front end
//!   ignores it.

use super::regs::*;

/// The RS needs the window width a multiple of 16 and the height of 4
/// (`etnaviv_internal.h:55-57`, `ETNA_RS_WIDTH_MASK` / `ETNA_RS_HEIGHT_MASK`;
/// Mesa aborts on a width that is not, "or bad things happen").
pub const RS_WIDTH_ALIGN: u32 = 16;
pub const RS_HEIGHT_ALIGN: u32 = 4;

/// RS_CLEAR_CONTROL (state_3d.xml.h `VIVS_RS_CLEAR_CONTROL_*`).
pub const RS_CLEAR_MODE_ENABLED1: u32 = 0x0001_0000;
/// Every bit of the pixel.
pub const RS_CLEAR_BITS_ALL: u32 = 0xffff;
/// S8Z24: the depth bytes of each pixel, leaving the stencil byte.
pub const RS_CLEAR_BITS_S8Z24_DEPTH: u32 = 0xeeee;
/// S8Z24: the stencil byte of each pixel.
pub const RS_CLEAR_BITS_S8Z24_STENCIL: u32 = 0x1111;

/// The word after a LOAD_STATE of an even count (keeps 64-bit alignment).
pub const PAD: u32 = GC_OP_NOP;

/// The longest sequence [`rs_clear`] writes: two flushes (2 words each), two
/// stalls (4 each), two fills ([`FILL_WORDS`] each).
pub const MAX_WORDS: usize = 2 * 2 + 2 * 4 + 2 * FILL_WORDS;
/// One RS fill block, as coalesced (see `fill`).
pub const FILL_WORDS: usize = 32;

/// A render surface the clear may fill. The padded size and the stride are
/// the allocation's, not the GL size: the fill covers the padding as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// GPU address of the first byte.
    pub addr: u32,
    pub padded_width: u32,
    pub padded_height: u32,
    /// Bytes a pixel: 2 or 4 take the RS, anything else the quad.
    pub bpp: u32,
}

impl Surface {
    /// Bytes from one row to the next (linear).
    pub const fn stride(&self) -> u32 {
        self.padded_width * self.bpp
    }
}

/// The colour plane of a clear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourClear {
    pub surface: Surface,
    /// The clear colour packed in the surface's pixel format (the low 16
    /// bits for a 2-byte pixel).
    pub pixel: u32,
    /// glColorMask as RGBA bits 0..3; the RS needs all four (0xf).
    pub write_mask: u8,
}

/// The depth and stencil planes of a clear: one surface, D24S8 (S8Z24 in
/// Mesa's naming: stencil in the low byte), X8Z24 or Z16.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthStencilClear {
    pub surface: Surface,
    /// The surface has a stencil byte (D24S8). Only with 4 bytes a pixel.
    pub has_stencil: bool,
    pub clear_depth: bool,
    pub clear_stencil: bool,
    /// glClearDepthf, 0..1.
    pub depth: f32,
    /// glClearStencil. Packed into the word even when stencil is not
    /// cleared, as Mesa does; the bits then leave it unwritten.
    pub stencil: u8,
    /// glDepthMask.
    pub depth_write: bool,
    /// glStencilMask (front); the RS needs 0xff.
    pub stencil_write_mask: u8,
}

/// One glClear of the bound target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clear {
    /// The GL size of the target (what a scissor box has to cover).
    pub width: u32,
    pub height: u32,
    /// The scissor box `(x, y, w, h)` when GL_SCISSOR_TEST is on.
    pub scissor: Option<(i32, i32, i32, i32)>,
    pub colour: Option<ColourClear>,
    pub depth_stencil: Option<DepthStencilClear>,
}

/// Why a clear stays a quad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotRs {
    /// No plane to clear.
    Nothing,
    /// The scissor box does not cover the target.
    Scissored,
    /// A colour channel is masked.
    ColourMasked,
    /// glDepthMask(GL_FALSE) with depth cleared.
    DepthMasked,
    /// A stencil write mask other than 0xff with stencil cleared.
    StencilMasked,
    /// A padded width not a multiple of 16 or height not of 4, or zero.
    Unaligned,
    /// A pixel size the RS fill does not take here (or stencil without 4
    /// bytes a pixel).
    Bpp,
}

/// The words of an RS clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub words: [u32; MAX_WORDS],
    pub len: usize,
}

impl Words {
    pub const fn new() -> Self {
        Words { words: [0; MAX_WORDS], len: 0 }
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.words[..self.len]
    }

    fn push(&mut self, w: u32) {
        self.words[self.len] = w;
        self.len += 1;
    }

    /// LOAD_STATE of one register (no pad: header and value are two words).
    fn state(&mut self, reg: u32, v: u32) {
        self.push(load_state(reg, 1, false));
        self.push(v);
    }

    /// Mesa's `etna_stall` for a recipient other than the FE: the semaphore,
    /// then the stall token (`etnaviv_emit.c:55-77`).
    fn stall(&mut self, from: u32, to: u32) {
        self.state(GC_GL_SEMAPHORE_TOKEN, stall_token(from, to));
        self.state(GC_GL_STALL_TOKEN, stall_token(from, to));
    }
}

impl Default for Words {
    fn default() -> Self {
        Self::new()
    }
}

/// Consecutive registers into one LOAD_STATE, as Mesa's `etna_coalesce_*`
/// (`etnaviv_emit.h:170-245`): a new header when the register is not the one
/// after the last, its count patched in at the end of the run, then a pad
/// when the stream is at an odd word.
struct Coalesce<'a> {
    w: &'a mut Words,
    hdr: Option<usize>,
    last: u32,
}

impl<'a> Coalesce<'a> {
    fn new(w: &'a mut Words) -> Self {
        Coalesce { w, hdr: None, last: 0 }
    }

    fn emit(&mut self, reg: u32, v: u32) {
        if self.hdr.is_none() || self.last + 4 != reg {
            self.end();
            self.hdr = Some(self.w.len);
            self.w.push(load_state(reg, 0, false));
        }
        self.w.push(v);
        self.last = reg;
    }

    fn end(&mut self) {
        if let Some(h) = self.hdr.take() {
            let count = (self.w.len - h - 1) as u32;
            self.w.words[h] |= count << 16;
            if self.w.len % 2 == 1 {
                self.w.push(PAD);
            }
        }
    }
}

/// The RS format for a pixel of `bpp` bytes, or `None` if the fill does not
/// take it here (`etna_rs_gen_clear_cmd`; 1 byte needs the S8 feature and
/// 8/16 bytes the 64BPP_CLEAR format, neither used by this backend).
pub const fn rs_format(bpp: u32) -> Option<u32> {
    match bpp {
        2 => Some(GC_RS_FORMAT_A4R4G4B4),
        4 => Some(GC_RS_FORMAT_A8R8G8B8),
        _ => None,
    }
}

/// RS_CONFIG of a linear fill: source and destination format, no tiling.
pub const fn rs_config(format: u32) -> u32 {
    rs_source_format(format) | rs_dest_format(format)
}

/// `lrint(z * max)` for `z` in 0..1, rounding half to even as the C default
/// rounding mode does; 0 below the range (and for NaN), `max` above it.
fn unorm(z: f32, max: u32) -> u32 {
    let z = z as f64;
    if z.is_nan() || z <= 0.0 {
        return 0;
    }
    if z >= 1.0 {
        return max;
    }
    let x = z * max as f64;
    let i = x as u32; /* x is positive: truncation is floor */
    let frac = x - i as f64;
    if frac > 0.5 || (frac == 0.5 && i & 1 == 1) {
        i + 1
    } else {
        i
    }
}

/// The D24S8 (S8Z24) or X8Z24 clear word: depth in the top 24 bits, stencil
/// in the low byte (`util_pack_z_stencil`, `PIPE_FORMAT_S8_UINT_Z24_UNORM`).
pub fn d24s8_word(depth: f32, stencil: u8) -> u32 {
    (unorm(depth, 0xff_ffff) << 8) | stencil as u32
}

/// The Z16 clear word, doubled into both halves
/// (`translate_clear_depth_stencil`).
pub fn z16_word(depth: f32) -> u32 {
    let v = unorm(depth, 0xffff);
    v | (v << 16)
}

/// One RS fill of the whole padded `s`: `etna_compile_rs_state` +
/// `etna_submit_rs_state` for a clear of a linear surface on a one-pipe,
/// RS_NEW_BASEADDR, SINGLE_BUFFER core. [`FILL_WORDS`] words.
pub fn fill(w: &mut Words, s: &Surface, format: u32, bits: u32, values: [u32; 4]) {
    let mut c = Coalesce::new(w);

    c.emit(GC_RS_CONFIG, rs_config(format));
    c.emit(GC_RS_SOURCE_STRIDE, 0); /* no source: a fill */
    c.emit(GC_RS_DEST_STRIDE, s.stride());
    c.emit(rs_pipe_dest_addr(0), s.addr);
    c.emit(rs_pipe_offset(0), 0);
    c.emit(rs_pipe_offset(1), 0);
    c.emit(GC_RS_WINDOW_SIZE, rs_window_width(s.padded_width) | rs_window_height(s.padded_height));
    c.emit(rs_dither(0), 0xffff_ffff);
    c.emit(rs_dither(1), 0xffff_ffff);
    c.emit(GC_RS_CLEAR_CONTROL, (bits & RS_CLEAR_BITS_ALL) | RS_CLEAR_MODE_ENABLED1);
    for (i, v) in values.iter().enumerate() {
        c.emit(rs_fill_value(i as u32), *v);
    }
    c.emit(GC_RS_EXTRA_CONFIG, 0); /* AA 0, endian no swap */
    c.emit(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    c.emit(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    c.emit(GC_RS_SINGLE_BUFFER, 0);
    c.end();
}

fn aligned(s: &Surface) -> bool {
    s.padded_width != 0
        && s.padded_height != 0
        && s.padded_width <= 0xffff
        && s.padded_height <= 0xffff
        && s.padded_width.is_multiple_of(RS_WIDTH_ALIGN)
        && s.padded_height.is_multiple_of(RS_HEIGHT_ALIGN)
}

/// Does the scissor box, if any, cover the whole `width` x `height` target?
pub fn scissor_covers(scissor: Option<(i32, i32, i32, i32)>, width: u32, height: u32) -> bool {
    match scissor {
        None => true,
        Some((x, y, w, h)) => {
            x <= 0 && y <= 0 && (x as i64 + w as i64) >= width as i64 && (y as i64 + h as i64) >= height as i64
        }
    }
}

/// What a depth/stencil fill writes: (format, bits, fill word), or `None`
/// when nothing of the surface is cleared.
fn ds_fill(d: &DepthStencilClear) -> Result<Option<(u32, u32, u32)>, NotRs> {
    let stencil = d.clear_stencil && d.has_stencil;

    if !d.clear_depth && !stencil {
        return Ok(None);
    }
    if d.clear_depth && !d.depth_write {
        return Err(NotRs::DepthMasked);
    }
    if stencil && d.stencil_write_mask != 0xff {
        return Err(NotRs::StencilMasked);
    }
    match (d.surface.bpp, d.has_stencil) {
        (4, true) => {
            let mut bits = 0;
            if d.clear_depth {
                bits |= RS_CLEAR_BITS_S8Z24_DEPTH;
            }
            if stencil {
                bits |= RS_CLEAR_BITS_S8Z24_STENCIL;
            }
            Ok(Some((GC_RS_FORMAT_A8R8G8B8, bits, d24s8_word(d.depth, d.stencil))))
        }
        (4, false) => Ok(Some((GC_RS_FORMAT_A8R8G8B8, RS_CLEAR_BITS_ALL, d24s8_word(d.depth, 0)))),
        (2, false) => Ok(Some((GC_RS_FORMAT_A4R4G4B4, RS_CLEAR_BITS_ALL, z16_word(d.depth)))),
        _ => Err(NotRs::Bpp),
    }
}

/// The RS fill of a full-target clear, or why it stays a quad.
///
/// The words are `etna_clear_rs` without tile status: GL_FLUSH_CACHE
/// COLOR|DEPTH, a stall RA->PE, the colour fill, GL_FLUSH_CACHE again when a
/// depth fill follows, the depth/stencil fill, a stall RA->PE.
pub fn rs_clear(c: &Clear) -> Result<Words, NotRs> {
    let colour = match &c.colour {
        Some(col) => {
            if col.write_mask & 0xf != 0xf {
                return Err(NotRs::ColourMasked);
            }
            let Some(format) = rs_format(col.surface.bpp) else {
                return Err(NotRs::Bpp);
            };
            Some((col, format))
        }
        None => None,
    };
    let ds = match &c.depth_stencil {
        Some(d) => ds_fill(d)?.map(|f| (d, f)),
        None => None,
    };
    if colour.is_none() && ds.is_none() {
        return Err(NotRs::Nothing);
    }
    if !scissor_covers(c.scissor, c.width, c.height) {
        return Err(NotRs::Scissored);
    }
    if colour.is_some_and(|(col, _)| !aligned(&col.surface)) || ds.is_some_and(|(d, _)| !aligned(&d.surface)) {
        return Err(NotRs::Unaligned);
    }

    let mut w = Words::new();
    let flush = GC_FLUSH_COLOR | GC_FLUSH_DEPTH;

    w.state(GC_GL_FLUSH_CACHE, flush);
    w.stall(GC_SYNC_RA, GC_SYNC_PE);
    if let Some((col, format)) = colour {
        let p = if col.surface.bpp == 2 { (col.pixel & 0xffff) | (col.pixel << 16) } else { col.pixel };
        fill(&mut w, &col.surface, format, RS_CLEAR_BITS_ALL, [p, p, p, p]);
    }
    if let Some((d, (format, bits, v))) = ds {
        if colour.is_some() {
            w.state(GC_GL_FLUSH_CACHE, flush);
        }
        fill(&mut w, &d.surface, format, bits, [v, 0, v, 0]);
    }
    w.stall(GC_SYNC_RA, GC_SYNC_PE);
    Ok(w)
}
