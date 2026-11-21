// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a full-target glClear as a resolve-engine (RS) fill,
//! as a pure function (docs/notes/gc-slow-stream-diff.md, finding 1).
//!
//! The backend draws every glClear as a quad (`backend.rs` `emit_clear`).
//! Mesa etnaviv clears a whole surface through the RS instead: no shader, no
//! vertex fetch, the fill value a literal in the stream. This module decides
//! whether a clear may take that path and writes its command words;
//! `backend.rs` `emit_rs_clear` builds the [`Clear`] of a draw and appends
//! the words (tests/gc_rs_clear_stream.rs). It depends on `super::regs`
//! only, so a host test includes both by `#[path]`.
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

/// A render surface the clear may fill. The padded size is the window the
/// fill covers: Mesa's is the allocation's, not the GL size, so the fill
/// covers the padding as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// GPU address of the first byte.
    pub addr: u32,
    pub padded_width: u32,
    pub padded_height: u32,
    /// Bytes a pixel: 2 or 4 take the RS, anything else the quad.
    pub bpp: u32,
    /// Bytes from one row to the next when the rows are wider than the
    /// window (a window surface that is a view into a wider scan-out
    /// buffer, whose columns past the window are not this target's); 0:
    /// `padded_width * bpp`, Mesa's.
    pub pitch: u32,
}

impl Surface {
    /// Bytes from one row to the next (linear).
    pub const fn stride(&self) -> u32 {
        if self.pitch != 0 {
            self.pitch
        } else {
            self.padded_width * self.bpp
        }
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
        && (s.pitch == 0 || s.pitch >= s.padded_width * s.bpp)
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

// --- TS fast clear and in-place resolve (W8/W9 groundwork) -------------------
//
// Pure word builders for `docs/notes/gc-tiled-wire.md` W8 (colour fast clear),
// W9 (depth fast clear, RS in-place resolve). Nothing calls them: with the
// `gc_tiled_rt` knob off they are unreachable and no stream changes. Each
// returns the words of one step in Mesa's order (`etnaviv_rs.c:337-387`
// colour, `:467-480` depth, `:164-181` in place); the TS register run is what
// the golden stream `tools/vivante/testdata/ts-clear.golden` carries for the
// same clear (the golden's one `TS_MEM_CONFIG` run joins the colour and the
// depth plane; here each plane writes its own part, with `keep` the bits of
// the other plane).
//
// This file depends on `regs.rs` only (tests/gc_rs_clear.rs includes it by
// `#[path]` with nothing else), so the field maths of `ts.rs` is not called
// from here: the clear words come in as inputs (`ts::color_clear_words`,
// `ts::depth_clear_word`, `ts::depth_clear_bits`) and the few formulas used
// below are copied (`ts::ts_clear_job`, `ts::auto_disable_count`), with
// tests/gc_ts_fast_clear.rs comparing the copies with `ts.rs`.

/// `ts::TS_CLEAR_VALUE`: the RS fill word of a TS buffer, every tile "cleared".
const TS_CLEAR_VALUE: u32 = 0x1111_1111;

/// `ts::auto_disable_count`: `TS_*_AUTO_DISABLE_COUNT` after a fast clear
/// (`etnaviv_rs.c:382-387`, `:469-475`).
pub const fn auto_disable_count(padded_width: u32, padded_height: u32) -> u32 {
    padded_width * padded_height / 16
}

/// `ts::ts_clear_job`'s window height: `align(ts_layer_stride / 0x40, 4)`.
const fn ts_fill_height(ts_layer_stride: u32) -> u32 {
    (ts_layer_stride / 0x40 + 3) & !3
}

/// `ts::ts_mem_config` for the bits a fast clear sets.
const fn mem_config(fast_clear: u32, auto_disable: u32, auto: bool) -> u32 {
    fast_clear | if auto { auto_disable } else { 0 }
}

/// A render target with a tile-status buffer: where the TS and the surface
/// are, and the surface's padded size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsTarget {
    /// GPU address of the TS buffer.
    pub ts_addr: u32,
    /// `ts::ts_layer_stride` of the surface (the TS buffer's bytes).
    pub ts_layer_stride: u32,
    /// GPU address of the (tiled) surface.
    pub surface_addr: u32,
    pub padded_width: u32,
    pub padded_height: u32,
}

/// A colour fast clear of a 32-bit TS-backed target (`etna_blit_clear_color_rs`,
/// `etnaviv_rs.c:337-387`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourFastClear {
    pub target: TsTarget,
    /// `(TS_COLOR_CLEAR_VALUE, TS_COLOR_CLEAR_VALUE_EXT)`, `ts::color_clear_words`.
    pub clear: (u32, u32),
    /// `TS_MEM_CONFIG` bits the clear leaves alone (the depth plane's).
    pub keep: u32,
    /// The core has AUTO_DISABLE (`TS_COLOR_AUTO_DISABLE_COUNT` and the
    /// config bit). ts-clear.golden's shim core has none.
    pub auto_disable: bool,
}

/// A depth fast clear (`etnaviv_rs.c:467-480`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthFastClear {
    pub target: TsTarget,
    /// The RS clear bits of the clear, `ts::depth_clear_bits`: the TS path
    /// needs the whole word, 0xffff (`:467`; D24S8 depth 0xeeee | stencil
    /// 0x1111).
    pub bits: u32,
    /// `TS_DEPTH_CLEAR_VALUE`, `ts::depth_clear_word`.
    pub clear_word: u32,
    /// The surface is Z16 (`TS_MEM_CONFIG` `DEPTH_16BPP`).
    pub depth_16bpp: bool,
    /// `TS_MEM_CONFIG` bits the clear leaves alone (the colour plane's).
    pub keep: u32,
    pub auto_disable: bool,
}

/// `RS_CONFIG` of the RS fill of a TS buffer: A8R8G8B8 to a TILED destination
/// (`etna_rs_gen_ts_clear_cmd`, `etnaviv_rs.c:339-358`).
pub const fn ts_fill_rs_config() -> u32 {
    rs_config(GC_RS_FORMAT_A8R8G8B8) | GC_RS_DEST_TILED
}

/// The RS fill of a TS buffer with `TS_CLEAR_VALUE`: [`FILL_WORDS`] words. The destination is TILED, so its stride is the job's `0x40` times
/// 4; the fill values are `{v, 0, 0, 0}` (not `rs_clear`'s `{v, 0, v, 0}`).
pub fn ts_fill(w: &mut Words, ts_addr: u32, ts_layer_stride: u32) {
    let mut c = Coalesce::new(w);

    c.emit(GC_RS_CONFIG, ts_fill_rs_config());
    c.emit(GC_RS_SOURCE_STRIDE, 0);
    c.emit(GC_RS_DEST_STRIDE, 0x40 << 2);
    c.emit(rs_pipe_dest_addr(0), ts_addr);
    c.emit(rs_pipe_offset(0), 0);
    c.emit(rs_pipe_offset(1), 0);
    c.emit(GC_RS_WINDOW_SIZE, rs_window_width(16) | rs_window_height(ts_fill_height(ts_layer_stride)));
    c.emit(rs_dither(0), 0xffff_ffff);
    c.emit(rs_dither(1), 0xffff_ffff);
    c.emit(GC_RS_CLEAR_CONTROL, RS_CLEAR_MODE_ENABLED1 | RS_CLEAR_BITS_ALL);
    c.emit(rs_fill_value(0), TS_CLEAR_VALUE);
    for i in 1..4 {
        c.emit(rs_fill_value(i), 0);
    }
    c.emit(GC_RS_EXTRA_CONFIG, 0);
    c.emit(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    c.emit(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    c.emit(GC_RS_SINGLE_BUFFER, 0);
    c.end();
}

/// Before a TS fill: the PE caches out, RA waits for PE, the TS cache is
/// flushed (the golden's first three records).
fn ts_fill_prologue(w: &mut Words) {
    w.state(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH);
    w.stall(GC_SYNC_RA, GC_SYNC_PE);
    w.state(GC_TS_FLUSH_CACHE, GC_TS_FLUSH);
}

/// The colour fast clear: prologue, the TS fill, a stall RA->PE, then the
/// registers that make the TS valid: `TS_MEM_CONFIG` (`COLOR_FAST_CLEAR`,
/// `COLOR_AUTO_DISABLE`), `TS_COLOR_STATUS_BASE`, `_SURFACE_BASE`,
/// `_CLEAR_VALUE` (one run), `_CLEAR_VALUE_EXT` (the same word for a texel of
/// 32 bits or less, `etna_clear_blit_pack_rgba`) and, with AUTO_DISABLE,
/// `TS_COLOR_AUTO_DISABLE_COUNT`.
pub fn colour_fast_clear(c: &ColourFastClear) -> Words {
    let t = &c.target;
    let (lo, ext) = c.clear;
    let cfg =
        mem_config(GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR, GC_TS_MEM_CONFIG_COLOR_AUTO_DISABLE, c.auto_disable) | c.keep;
    let mut w = Words::new();

    ts_fill_prologue(&mut w);
    ts_fill(&mut w, t.ts_addr, t.ts_layer_stride);
    w.stall(GC_SYNC_RA, GC_SYNC_PE);
    let mut r = Coalesce::new(&mut w);
    r.emit(GC_TS_MEM_CONFIG, cfg);
    r.emit(GC_TS_COLOR_STATUS_BASE, t.ts_addr);
    r.emit(GC_TS_COLOR_SURFACE_BASE, t.surface_addr);
    r.emit(GC_TS_COLOR_CLEAR_VALUE, lo);
    r.emit(GC_TS_COLOR_CLEAR_VALUE_EXT, ext);
    if c.auto_disable {
        r.emit(GC_TS_COLOR_AUTO_DISABLE_COUNT, auto_disable_count(t.padded_width, t.padded_height));
    }
    r.end();
    w
}

/// The depth fast clear, or `None` when the clear does not cover the whole
/// word (a depth-only clear of D24S8, bits 0xeeee: a plain RS fill, `fill`).
/// Prologue, the TS fill, a stall, then `TS_MEM_CONFIG` (`DEPTH_FAST_CLEAR`,
/// `DEPTH_16BPP` for Z16, `DEPTH_AUTO_DISABLE`) and `TS_DEPTH_STATUS_BASE`,
/// `_SURFACE_BASE`, `_CLEAR_VALUE` and, with AUTO_DISABLE,
/// `TS_DEPTH_AUTO_DISABLE_COUNT` as one run.
pub fn depth_fast_clear(c: &DepthFastClear) -> Option<Words> {
    if c.bits != RS_CLEAR_BITS_ALL {
        return None;
    }
    let t = &c.target;
    let cfg = mem_config(GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR, GC_TS_MEM_CONFIG_DEPTH_AUTO_DISABLE, c.auto_disable)
        | if c.depth_16bpp { GC_TS_MEM_CONFIG_DEPTH_16BPP } else { 0 }
        | c.keep;
    let mut w = Words::new();

    ts_fill_prologue(&mut w);
    ts_fill(&mut w, t.ts_addr, t.ts_layer_stride);
    w.stall(GC_SYNC_RA, GC_SYNC_PE);
    let mut r = Coalesce::new(&mut w);
    r.emit(GC_TS_MEM_CONFIG, cfg);
    r.end();
    r.emit(GC_TS_DEPTH_STATUS_BASE, t.ts_addr);
    r.emit(GC_TS_DEPTH_SURFACE_BASE, t.surface_addr);
    r.emit(GC_TS_DEPTH_CLEAR_VALUE, c.clear_word);
    if c.auto_disable {
        r.emit(GC_TS_DEPTH_AUTO_DISABLE_COUNT, auto_disable_count(t.padded_width, t.padded_height));
    }
    r.end();
    Some(w)
}

/// The tile count of `RS_KICKER_INPLACE`. UNVERIFIED: Mesa's source is not in
/// the tree; this is the TS tile count `auto_disable_count` uses, and a
/// board or Mesa check must confirm it before the words are wired.
pub const fn inplace_tile_count(padded_width: u32, padded_height: u32) -> u32 {
    auto_disable_count(padded_width, padded_height)
}

/// The plane whose TS the in-place resolve reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InplacePlane {
    /// `TS_COLOR_*`: status, surface and the 32-bit clear value.
    Colour { clear_value: u32 },
    /// `TS_DEPTH_*`: status, surface and the clear word; `depth_16bpp` sets
    /// `TS_MEM_CONFIG` `DEPTH_16BPP`.
    Depth { clear_value: u32, depth_16bpp: bool },
}

/// An RS in-place resolve of a super-tiled surface with a valid TS
/// (`etnaviv_rs.c:164-181`): source = destination, same format and stride,
/// and the RS writes only the tiles the TS marks cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InplaceResolve {
    pub target: TsTarget,
    /// RS format of the surface (`rs_format`).
    pub format: u32,
    /// Bytes from one row to the next of the surface, untiled (the stride
    /// register takes it times 4, as for a tiled side).
    pub stride: u32,
    /// `RS_KICKER_INPLACE` value, [`inplace_tile_count`].
    pub tile_count: u32,
    pub plane: InplacePlane,
    /// `TS_MEM_CONFIG` bits of the other plane (kept).
    pub keep: u32,
}

/// The in-place resolve: `TS_FLUSH_CACHE`, the TS source registers (the
/// plane's `FAST_CLEAR` in `TS_MEM_CONFIG`, status, surface, clear value),
/// the RS block with a super-tiled source and destination at one address, and
/// the kick as `RS_KICKER_INPLACE` (no `RS_SINGLE_BUFFER`, no `RS_KICKER`).
/// The source stride's bit 29, which the golden's TS-sourced resolve sets,
/// is not named in `regs.rs` and is not written here.
pub fn inplace_resolve(c: &InplaceResolve) -> Words {
    let t = &c.target;
    let mut w = Words::new();

    w.state(GC_TS_FLUSH_CACHE, GC_TS_FLUSH);
    let mut r = Coalesce::new(&mut w);
    match c.plane {
        InplacePlane::Colour { clear_value } => {
            r.emit(GC_TS_MEM_CONFIG, GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR | c.keep);
            r.emit(GC_TS_COLOR_STATUS_BASE, t.ts_addr);
            r.emit(GC_TS_COLOR_SURFACE_BASE, t.surface_addr);
            r.emit(GC_TS_COLOR_CLEAR_VALUE, clear_value);
            r.emit(GC_TS_COLOR_CLEAR_VALUE_EXT, clear_value);
        }
        InplacePlane::Depth { clear_value, depth_16bpp } => {
            let bpp = if depth_16bpp { GC_TS_MEM_CONFIG_DEPTH_16BPP } else { 0 };
            r.emit(GC_TS_MEM_CONFIG, GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR | bpp | c.keep);
            r.end();
            r.emit(GC_TS_DEPTH_STATUS_BASE, t.ts_addr);
            r.emit(GC_TS_DEPTH_SURFACE_BASE, t.surface_addr);
            r.emit(GC_TS_DEPTH_CLEAR_VALUE, clear_value);
        }
    }
    r.end();
    let stride = (c.stride << 2) | GC_RS_SOURCE_STRIDE_TILING | GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW;
    let dest_stride = (c.stride << 2) | GC_RS_DEST_STRIDE_TILING | GC_RS_DEST_STRIDE_SUPER_TILED_NEW;
    let mut r = Coalesce::new(&mut w);
    r.emit(GC_RS_CONFIG, rs_config(c.format) | GC_RS_SOURCE_TILED | GC_RS_DEST_TILED);
    r.emit(GC_RS_SOURCE_STRIDE, stride);
    r.emit(GC_RS_DEST_STRIDE, dest_stride);
    r.emit(rs_pipe_source_addr(0), t.surface_addr);
    r.emit(rs_pipe_dest_addr(0), t.surface_addr);
    r.emit(rs_pipe_offset(0), 0);
    r.emit(rs_pipe_offset(1), 0);
    r.emit(GC_RS_WINDOW_SIZE, rs_window_width(t.padded_width) | rs_window_height(t.padded_height));
    r.emit(rs_dither(0), 0xffff_ffff);
    r.emit(rs_dither(1), 0xffff_ffff);
    r.emit(GC_RS_CLEAR_CONTROL, 0);
    for i in 0..4 {
        r.emit(rs_fill_value(i), 0);
    }
    r.emit(GC_RS_EXTRA_CONFIG, 0);
    r.emit(GC_RS_KICKER_INPLACE, c.tile_count);
    r.end();
    w
}

// --- TS-sourced resolve into linear storage (W8 groundwork) -------------------

/// `RS_SOURCE_STRIDE` `TS_MODE` (bit 29): the golden's resolve sets it
/// (`RS_SOURCE_STRIDE` 0xa8000400); `regs.rs` has no name for it.
const RS_SOURCE_STRIDE_TS_MODE_SHIFT: u32 = 29;

/// How the source surface of a TS-sourced resolve is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TsSourceLayout {
    /// `TILED`: `RS_SOURCE_STRIDE` `TILING` only (W8's shadow).
    Tiled,
    /// `SUPER_TILED`: `TILING | SUPER_TILED_NEW` (the golden's source).
    SuperTiled,
}

/// A copy of a TS-backed colour target to linear storage, the TS read by the
/// RS (`etna_try_rs_blit`, `etnaviv_rs.c:852-891`): cleared tiles read as the
/// clear value, so the linear destination needs no prior fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsResolve {
    /// The source's TS buffer and surface; its padded size is the RS window.
    pub target: TsTarget,
    pub layout: TsSourceLayout,
    /// RS format of both sides (`rs_format`).
    pub format: u32,
    /// `TS_COLOR_CLEAR_VALUE` of the source (`TS_COLOR_CLEAR_VALUE_EXT` is the
    /// same word, as for the clear).
    pub clear_value: u32,
    /// GPU address of the linear destination.
    pub dest_addr: u32,
    /// Bytes per row of the linear destination (not shifted, `RS_DEST_STRIDE`).
    pub dest_stride: u32,
    /// Bytes per row of the source, untiled (the register takes it times 4).
    pub source_stride: u32,
    /// `RS_CONFIG` `SWAP_RB` (the golden's RGBA8 target).
    pub swap_rb: bool,
    /// `RS_SOURCE_STRIDE` bit 29 (golden 1, the MSAA resolve's `TS_MODE(0)`).
    pub source_ts_mode: u32,
    /// `RS_EXTRA_CONFIG` `TS_MODE` (`etnaviv_rs.c:852-891`; the golden has 0).
    pub extra_ts_mode: u32,
    /// `TS_MEM_CONFIG` bits of the depth plane (kept).
    pub keep: u32,
}

/// `TS_FLUSH_CACHE`, `TS_MEM_CONFIG` `COLOR_FAST_CLEAR`, the source's
/// `TS_COLOR_STATUS_BASE`, `_SURFACE_BASE`, `_CLEAR_VALUE` (+ `_EXT`),
/// then the RS block (source tiled, destination linear, the TS read through
/// `RS_EXTRA_CONFIG` `TS_MODE`) and the single-buffer kick.
pub fn ts_resolve(c: &TsResolve) -> Words {
    let t = &c.target;
    let mut w = Words::new();

    w.state(GC_TS_FLUSH_CACHE, GC_TS_FLUSH);
    let mut r = Coalesce::new(&mut w);
    r.emit(GC_TS_MEM_CONFIG, GC_TS_MEM_CONFIG_COLOR_FAST_CLEAR | c.keep);
    r.emit(GC_TS_COLOR_STATUS_BASE, t.ts_addr);
    r.emit(GC_TS_COLOR_SURFACE_BASE, t.surface_addr);
    r.emit(GC_TS_COLOR_CLEAR_VALUE, c.clear_value);
    r.emit(GC_TS_COLOR_CLEAR_VALUE_EXT, c.clear_value);
    r.end();
    let super_tiled = match c.layout {
        TsSourceLayout::Tiled => 0,
        TsSourceLayout::SuperTiled => GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW,
    };
    let mut r = Coalesce::new(&mut w);
    r.emit(GC_RS_CONFIG, rs_config(c.format) | GC_RS_SOURCE_TILED | if c.swap_rb { GC_RS_SWAP_RB } else { 0 });
    r.emit(
        GC_RS_SOURCE_STRIDE,
        (c.source_stride << 2)
            | GC_RS_SOURCE_STRIDE_TILING
            | ((c.source_ts_mode & 1) << RS_SOURCE_STRIDE_TS_MODE_SHIFT)
            | super_tiled,
    );
    r.emit(GC_RS_DEST_STRIDE, c.dest_stride);
    r.emit(rs_pipe_source_addr(0), t.surface_addr);
    r.emit(rs_pipe_dest_addr(0), c.dest_addr);
    r.emit(rs_pipe_offset(0), 0);
    r.emit(rs_pipe_offset(1), 0);
    r.emit(GC_RS_WINDOW_SIZE, rs_window_width(t.padded_width) | rs_window_height(t.padded_height));
    r.emit(rs_dither(0), 0xffff_ffff);
    r.emit(rs_dither(1), 0xffff_ffff);
    r.emit(GC_RS_CLEAR_CONTROL, 0);
    for i in 0..4 {
        r.emit(rs_fill_value(i), 0);
    }
    r.emit(GC_RS_EXTRA_CONFIG, rs_extra_ts_mode(c.extra_ts_mode));
    r.emit(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    r.emit(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    r.emit(GC_RS_SINGLE_BUFFER, 0);
    r.end();
    w
}

// --- W13: the swap resolve of a TILED window (groundwork, no wiring) ----------
//
// `docs/notes/gc-tiled-window.md` W13: the window's colour buffer is a 4x4
// TILED shadow and the RS resolves it into the linear buffer the display
// reads, in the frame's last stream before eglSwapBuffers presents it.
// Nothing calls these builders: egl.rs and backend.rs are unchanged, and no
// stream moves. tests/gc_swap_resolve.rs runs the words through `rs_interp`
// over a tiled shadow and compares the result with an untile through
// `layout::offset`; this file keeps its `regs.rs`-only dependency (the host
// tests that include it by `#[path]`).
//
// The words are Mesa 26.2.1's for the flush of a window resource with a
// tiled render shadow: `etna_flush_resource` (`etnaviv_clear_blit.c:286-295`)
// -> `etna_copy_resource` (`:337-395`, the whole level) -> `etna_try_rs_blit`
// (`etnaviv_rs.c:678-943`, no tile status) -> `etna_compile_rs_state`
// (`:68-187`) -> `etna_submit_rs_state` (`:221-257`, the RS_NEW_BASEADDR and
// SINGLE_BUFFER arm of this core). In order: GL_FLUSH_CACHE COLOR|DEPTH and a
// stall RA->PE (`:845-850`), `TS_MEM_CONFIG` 0 because the source has no
// valid tile status (`:888-890`), then the coalesced RS block.
//
// - A TILED side's stride is its row stride times 4 with RS_CONFIG's
//   `SOURCE_TILED`, and no stride `TILING` bit (that bit is the super-tiled
//   sides', `:104-110`); `TS_MODE(0)` of the CACHE128B256BPERLINE core is 0.
// - The window is the GL size, widened to the RS alignment (16 x 4) when the
//   destination's padding holds the widened rectangle (`:806-837`); otherwise
//   Mesa leaves the RS (`goto manual`, which for a linear destination returns
//   false), and here the builder refuses ([`NotSwapResolve`]). The shadow's
//   own padding always holds it: its stride is the 16-padded width
//   ([`window_shadow_stride`]) and its rows the 4-padded height
//   (`layout::padding`, TILED with `rs_align`).
// - `swap_rb` is Mesa's `blit_rb_swap` of a shared RB_SWAP format
//   (`etnaviv_clear_blit.c:283-284`): an RGBA8 window resolved from the PE's
//   BGRA order.
//
// Where gc-tiled-window.md differs from Mesa: the note refuses a window whose
// width is not a multiple of 16 or height not of 4 (its 1366x768 example,
// after `tiled_rt::spec_of`); Mesa resolves such a window when the
// destination is padded for the widened rectangle, and so does
// [`swap_resolve`] given [`WindowResolve::dst_pitch`] and
// [`WindowResolve::dst_rows`]. The note names no size floor of the LCDIF:
// its "frame floor" is the latch wait in `board_show`, a time, so no size is
// refused for it. The in-place (scanned-out) window the note excludes is a
// window class, the caller's decision, not a size.

/// `VIVS_RS_SOURCE_STRIDE_STRIDE__MASK` and `VIVS_RS_DEST_STRIDE_STRIDE__MASK`
/// (state_3d.xml.h): the stride field of both stride registers.
pub const RS_STRIDE_MAX: u32 = 0x0003_ffff;
/// `VIVS_RS_WINDOW_SIZE_WIDTH__MASK` / `_HEIGHT__MASK`: the window fields.
pub const RS_WINDOW_MAX: u32 = 0xffff;
/// Bytes a pixel of a window this resolve takes: 32-bit only (the note's
/// window shadow is `tiled_rt::BPP`'s 4 bytes; RGB565 is not in the design).
pub const WINDOW_BPP: u32 = 4;

/// The swap resolve of one window buffer: the tiled shadow into the linear
/// buffer of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowResolve {
    /// GPU address of the 4x4 TILED shadow, [`window_shadow_stride`] bytes a
    /// row of pixels (the RS's 16-padded width), rows padded to 4.
    pub src_addr: u32,
    /// GPU address of the linear window buffer.
    pub dst_addr: u32,
    /// The GL size of the window.
    pub width: u32,
    pub height: u32,
    /// RS format of both sides: `GC_RS_FORMAT_A8R8G8B8` or
    /// `GC_RS_FORMAT_X8R8G8B8`. Any other is refused.
    pub format: u32,
    /// `RS_CONFIG` `SWAP_RB`: R and B exchanged on the way (an RGBA8 window).
    pub swap_rb: bool,
    /// Bytes from one row of the destination to the next; 0: `width * 4`.
    pub dst_pitch: u32,
    /// Rows the destination buffer holds; 0: `height`. More rows than
    /// `height` let a height that is not a multiple of 4 be widened.
    pub dst_rows: u32,
}

/// Why the RS does not resolve a window (the caller keeps the linear path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSwapResolve {
    /// A width or height of 0.
    Empty,
    /// An RS format other than A8R8G8B8 and X8R8G8B8 (RGB565 included).
    Format,
    /// `dst_pitch` shorter than a row of `width` pixels, or `dst_rows` fewer
    /// than `height`.
    Destination,
    /// The window widened to 16 x 4 does not fit the destination's padding
    /// (Mesa's `goto manual`, `etnaviv_rs.c:832-837`).
    Unaligned,
    /// A window or a stride past its register field.
    TooLarge,
}

/// The row stride in bytes of a window shadow `width` pixels wide:
/// `layout::stride(Layout::Tiled, width, 4, true)` (the 16-padded width
/// times 4, `etnaviv_resource.c:327-329`), computed here because this file
/// depends on `regs.rs` only.
pub const fn window_shadow_stride(width: u32) -> u32 {
    width.div_ceil(RS_WIDTH_ALIGN) * RS_WIDTH_ALIGN * WINDOW_BPP
}

/// The destination's row pitch in bytes.
const fn window_pitch(r: &WindowResolve) -> u32 {
    if r.dst_pitch != 0 {
        r.dst_pitch
    } else {
        r.width * WINDOW_BPP
    }
}

/// The RS window `(w, h)` of a swap resolve: the GL size, each side widened
/// to the RS alignment when the destination's padding holds it
/// (`etnaviv_rs.c:806-837`, one sample, a whole level).
pub const fn swap_resolve_window(r: &WindowResolve) -> Result<(u32, u32), NotSwapResolve> {
    if r.width == 0 || r.height == 0 {
        return Err(NotSwapResolve::Empty);
    }
    if r.format != GC_RS_FORMAT_A8R8G8B8 && r.format != GC_RS_FORMAT_X8R8G8B8 {
        return Err(NotSwapResolve::Format);
    }
    /* the aligned window must fit the 16-bit fields (and the products below
     * a u32) */
    if r.width > RS_WINDOW_MAX - (RS_WIDTH_ALIGN - 1) || r.height > RS_WINDOW_MAX - (RS_HEIGHT_ALIGN - 1) {
        return Err(NotSwapResolve::TooLarge);
    }
    let pitch = window_pitch(r);
    let rows = if r.dst_rows != 0 { r.dst_rows } else { r.height };
    if pitch < r.width * WINDOW_BPP || rows < r.height {
        return Err(NotSwapResolve::Destination);
    }
    let w = r.width.div_ceil(RS_WIDTH_ALIGN) * RS_WIDTH_ALIGN;
    let h = r.height.div_ceil(RS_HEIGHT_ALIGN) * RS_HEIGHT_ALIGN;
    if w * WINDOW_BPP > pitch || h > rows {
        return Err(NotSwapResolve::Unaligned);
    }
    if window_shadow_stride(r.width) << 2 > RS_STRIDE_MAX || pitch > RS_STRIDE_MAX {
        return Err(NotSwapResolve::TooLarge);
    }
    Ok((w, h))
}

/// The words of the swap resolve of `r`, or why the RS does not take it:
/// GL_FLUSH_CACHE COLOR|DEPTH, a stall RA->PE, `TS_MEM_CONFIG` 0, then
/// RS_CONFIG (TILED source, linear destination, `SWAP_RB` as asked), the
/// strides, the pipe addresses and offsets, the window, dither off, no clear,
/// `RS_EXTRA_CONFIG` 0 and the kick inside RS_SINGLE_BUFFER, coalesced as
/// Mesa's `etna_submit_rs_state`: [`SWAP_RESOLVE_WORDS`] words.
///
/// `TS_MEM_CONFIG` 0 turns off any tile status the stream had set up (Mesa
/// marks the TS state dirty after a blit, `etnaviv_rs.c:891`): a caller with
/// a live TS writes it again after these words.
pub fn swap_resolve(r: &WindowResolve) -> Result<Words, NotSwapResolve> {
    let (w, h) = swap_resolve_window(r)?;
    let mut words = Words::new();

    words.state(GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH);
    words.stall(GC_SYNC_RA, GC_SYNC_PE);
    words.state(GC_TS_MEM_CONFIG, 0);
    let mut c = Coalesce::new(&mut words);
    c.emit(GC_RS_CONFIG, rs_config(r.format) | GC_RS_SOURCE_TILED | if r.swap_rb { GC_RS_SWAP_RB } else { 0 });
    c.emit(GC_RS_SOURCE_STRIDE, window_shadow_stride(r.width) << 2);
    c.emit(GC_RS_DEST_STRIDE, window_pitch(r));
    c.emit(rs_pipe_source_addr(0), r.src_addr);
    c.emit(rs_pipe_dest_addr(0), r.dst_addr);
    c.emit(rs_pipe_offset(0), 0);
    c.emit(rs_pipe_offset(1), 0);
    c.emit(GC_RS_WINDOW_SIZE, rs_window_width(w) | rs_window_height(h));
    c.emit(rs_dither(0), 0xffff_ffff);
    c.emit(rs_dither(1), 0xffff_ffff);
    c.emit(GC_RS_CLEAR_CONTROL, 0);
    for i in 0..4 {
        c.emit(rs_fill_value(i), 0);
    }
    c.emit(GC_RS_EXTRA_CONFIG, 0);
    c.emit(GC_RS_SINGLE_BUFFER, GC_RS_SINGLE_BUFFER_ENABLE);
    c.emit(GC_RS_KICKER, GC_RS_KICKER_VALUE);
    c.emit(GC_RS_SINGLE_BUFFER, 0);
    c.end();
    Ok(words)
}

/// The length of [`swap_resolve`]'s words.
pub const SWAP_RESOLVE_WORDS: usize = 42;

/// The `(register, value)` writes of `words`, a run of LOAD_STATE records
/// and their pad words (what every builder here writes), in stream order:
/// the input of `rs_interp::RsInterp::run`. Panics on a word that is neither
/// a LOAD_STATE header nor a pad where a header is due.
pub fn state_writes(words: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    let mut at = 0;
    let mut run = (0u32, 0u32); /* (next register, values left) */
    core::iter::from_fn(move || loop {
        if run.1 > 0 {
            let v = words[at];
            let w = (run.0, v);
            at += 1;
            run = (run.0 + 4, run.1 - 1);
            return Some(w);
        }
        let hdr = *words.get(at)?;
        at += 1;
        if hdr == PAD {
            continue;
        }
        assert_eq!(hdr & 0xf800_0000, GC_OP_LOAD_STATE, "rs_clear: word {hdr:#010x} is not a LOAD_STATE");
        run = ((hdr & 0xffff) << 2, (hdr >> 16) & 0x3ff);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: u32 = 0x1000_0000;
    const DST: u32 = 0x2000_0000;

    fn window(width: u32, height: u32, format: u32, swap_rb: bool) -> WindowResolve {
        WindowResolve { src_addr: SRC, dst_addr: DST, width, height, format, swap_rb, dst_pitch: 0, dst_rows: 0 }
    }

    /// The swap resolve's words for a window, derived by hand from Mesa
    /// 26.2.1: `etna_try_rs_blit` (`etnaviv_rs.c:845-850` flush and stall,
    /// `:888-890` `TS_MEM_CONFIG` 0), `etna_compile_rs_state` (`:95-162`) and
    /// `etna_submit_rs_state` (`:221-257`), coalesced as `etnaviv_emit.h`'s
    /// `etna_coalesce_*` (a header per run of consecutive registers, a pad
    /// after an odd end). Only the four values that depend on the window
    /// vary: RS_CONFIG, the two strides and the window size.
    #[rustfmt::skip]
    fn mesa_words(config: u32, src_stride: u32, dst_stride: u32, window: u32) -> [u32; SWAP_RESOLVE_WORDS] {
        [
            0x0801_0e03, 0x0000_0003,           /* GL_FLUSH_CACHE (0x380c) COLOR | DEPTH */
            0x0801_0e02, 0x0000_0705,           /* GL_SEMAPHORE_TOKEN (0x3808) RA -> PE */
            0x0801_0f00, 0x0000_0705,           /* GL_STALL_TOKEN (0x3c00) RA -> PE */
            0x0801_0595, 0x0000_0000,           /* TS_MEM_CONFIG (0x1654): no TS */
            0x0801_0581, config,                /* RS_CONFIG (0x1604) */
            0x0801_0583, src_stride,            /* RS_SOURCE_STRIDE (0x160c) */
            0x0801_0585, dst_stride,            /* RS_DEST_STRIDE (0x1614) */
            0x0801_05b0, SRC,                   /* RS_PIPE_SOURCE_ADDR(0) (0x16c0) */
            0x0801_05b8, DST,                   /* RS_PIPE_DEST_ADDR(0) (0x16e0) */
            0x0802_05c0, 0, 0, PAD,             /* RS_PIPE_OFFSET(0..1) (0x1700) */
            0x0801_0588, window,                /* RS_WINDOW_SIZE (0x1620) */
            0x0802_058c, 0xffff_ffff, 0xffff_ffff, PAD, /* RS_DITHER(0..1) (0x1630) */
            0x0805_058f, 0, 0, 0, 0, 0,         /* RS_CLEAR_CONTROL (0x163c), RS_FILL_VALUE(0..3) */
            0x0801_05a8, 0,                     /* RS_EXTRA_CONFIG (0x16a0) */
            0x0801_05ae, 1,                     /* RS_SINGLE_BUFFER (0x16b8) ENABLE */
            0x0801_0580, 0xbeeb_beeb,           /* RS_KICKER (0x1600) */
            0x0801_05ae, 0,                     /* RS_SINGLE_BUFFER */
        ]
    }

    #[test]
    fn the_800x600_swap_resolve_is_mesas_words() {
        // RS_CONFIG: SOURCE_FORMAT(A8R8G8B8 6) | SOURCE_TILED 0x80 |
        // DEST_FORMAT(6) << 8; source stride 800 * 4 = 0xc80, times 4, no
        // TILING bit (TILED, not super-tiled); destination 0xc80; window
        // 600 << 16 | 800.
        let w = swap_resolve(&window(800, 600, GC_RS_FORMAT_A8R8G8B8, false)).unwrap();
        assert_eq!(w.as_slice(), &mesa_words(0x0000_0686, 0x3200, 0x0c80, 0x0258_0320));
    }

    #[test]
    fn the_1920x1080_swap_resolve_is_mesas_words() {
        // X8R8G8B8 (5) with SWAP_RB 0x20000000; stride 1920 * 4 = 0x1e00,
        // times 4 = 0x7800; window 1080 << 16 | 1920.
        let w = swap_resolve(&window(1920, 1080, GC_RS_FORMAT_X8R8G8B8, true)).unwrap();
        assert_eq!(w.as_slice(), &mesa_words(0x2000_0585, 0x7800, 0x1e00, 0x0438_0780));
        let w = swap_resolve(&window(1920, 1080, GC_RS_FORMAT_A8R8G8B8, false)).unwrap();
        assert_eq!(w.as_slice(), &mesa_words(0x0000_0686, 0x7800, 0x1e00, 0x0438_0780));
    }

    #[test]
    fn a_padded_destination_widens_the_window_as_mesa_does() {
        // 1366 is not a multiple of 16: Mesa widens to 1376 when both
        // levels' padding hold it (etnaviv_rs.c:814-815); the shadow's
        // stride is the 16-padded row.
        let r = WindowResolve { dst_pitch: 1376 * 4, ..window(1366, 768, GC_RS_FORMAT_A8R8G8B8, false) };
        assert_eq!(swap_resolve_window(&r), Ok((1376, 768)));
        let w = swap_resolve(&r).unwrap();
        assert_eq!(w.as_slice(), &mesa_words(0x0000_0686, (1376 * 4) << 2, 1376 * 4, 768 << 16 | 1376));
        // and a height not a multiple of 4 with room for the padded rows
        let r = WindowResolve { dst_rows: 604, ..window(800, 601, GC_RS_FORMAT_A8R8G8B8, false) };
        assert_eq!(swap_resolve_window(&r), Ok((800, 604)));
    }

    #[test]
    fn the_sizes_and_formats_the_rs_does_not_resolve_are_refused() {
        let a8 = GC_RS_FORMAT_A8R8G8B8;
        let refused = |r: WindowResolve| swap_resolve(&r).err();
        assert_eq!(refused(window(0, 600, a8, false)), Some(NotSwapResolve::Empty));
        assert_eq!(refused(window(800, 0, a8, false)), Some(NotSwapResolve::Empty));
        // 16-bit windows are not in the design (gc-tiled-window.md: one
        // 4-byte target), RGB565 included
        assert_eq!(refused(window(800, 600, GC_RS_FORMAT_R5G6B5, false)), Some(NotSwapResolve::Format));
        assert_eq!(refused(window(800, 600, GC_RS_FORMAT_A4R4G4B4, false)), Some(NotSwapResolve::Format));
        // a destination smaller than the window
        assert_eq!(
            refused(WindowResolve { dst_pitch: 799 * 4, ..window(800, 600, a8, false) }),
            Some(NotSwapResolve::Destination)
        );
        assert_eq!(
            refused(WindowResolve { dst_rows: 599, ..window(800, 600, a8, false) }),
            Some(NotSwapResolve::Destination)
        );
        // no padding for the widened window (the note's 1366x768 panel with
        // a tight pitch; a height of 601 with no spare rows): Mesa's
        // `goto manual`
        assert_eq!(refused(window(1366, 768, a8, false)), Some(NotSwapResolve::Unaligned));
        assert_eq!(
            refused(WindowResolve { dst_pitch: 1372 * 4, ..window(1366, 768, a8, false) }),
            Some(NotSwapResolve::Unaligned)
        );
        assert_eq!(refused(window(800, 601, a8, false)), Some(NotSwapResolve::Unaligned));
        assert_eq!(refused(window(8, 4, a8, false)), Some(NotSwapResolve::Unaligned));
        // past the register fields: the source stride field (18 bits) holds
        // a row of 16383 pixels times 4 times 4, the window fields 16 bits
        assert!(swap_resolve(&window(16368, 4, a8, false)).is_ok());
        assert_eq!(refused(window(16384, 4, a8, false)), Some(NotSwapResolve::TooLarge));
        assert_eq!(refused(window(0xffff, 4, a8, false)), Some(NotSwapResolve::TooLarge));
        assert_eq!(refused(window(16, 0xffff, a8, false)), Some(NotSwapResolve::TooLarge));
        assert_eq!(
            refused(WindowResolve { dst_pitch: RS_STRIDE_MAX + 1, ..window(800, 600, a8, false) }),
            Some(NotSwapResolve::TooLarge)
        );
    }

    #[test]
    fn state_writes_reads_the_swap_resolve_back() {
        let w = swap_resolve(&window(800, 600, GC_RS_FORMAT_A8R8G8B8, false)).unwrap();
        let writes: [(u32, u32); 23] = {
            let mut it = state_writes(w.as_slice());
            let a = core::array::from_fn(|_| it.next().unwrap());
            assert_eq!(it.next(), None);
            a
        };
        assert_eq!(writes[0], (GC_GL_FLUSH_CACHE, GC_FLUSH_COLOR | GC_FLUSH_DEPTH));
        assert_eq!(writes[3], (GC_TS_MEM_CONFIG, 0));
        assert_eq!(writes[4], (GC_RS_CONFIG, 0x0686));
        assert_eq!(writes[9], (rs_pipe_offset(0), 0));
        assert_eq!(writes[10], (rs_pipe_offset(1), 0));
        assert_eq!(writes[11], (GC_RS_WINDOW_SIZE, 0x0258_0320));
        assert_eq!(writes[18], (rs_fill_value(3), 0));
        assert_eq!(writes[21], (GC_RS_KICKER, GC_RS_KICKER_VALUE));
        assert_eq!(writes[22], (GC_RS_SINGLE_BUFFER, 0));
    }
}
