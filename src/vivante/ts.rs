// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): tile-status (TS) buffer sizes and fast-clear words as
//! pure functions (groundwork for `docs/notes/gc-tiled-targets.md`).
//!
//! Everything here is what Mesa 26.2.1's etnaviv computes for this part
//! (hwdb `gc7000ULN_v123`: FAST_CLEAR, CACHE128B256BPERLINE, AUTO_DISABLE,
//! no DEC400, one pixel pipe, no BLT), worked out without a device. Citations
//! are files under `src/gallium/drivers/etnaviv/` unless a path says
//! otherwise. Nothing is wired to the back end yet.
//!
//! The module is self-contained: no `crate::` or `super::` path; the host
//! test `tests/gc_ts_words.rs` takes it from the crate as
//! `yakogl::vivante::ts`. The register addresses are copies of the `regs.rs`
//! constants named beside each one; the test file compares them against
//! `regs.rs`.

#![allow(dead_code)]

/// `regs.rs` `GC_PE_MEM_CONFIG`.
pub const PE_MEM_CONFIG: u32 = 0x0000_14bc;
/// `regs.rs` `GC_TS_FLUSH_CACHE`.
pub const TS_FLUSH_CACHE: u32 = 0x0000_1650;
/// `regs.rs` `GC_TS_MEM_CONFIG`.
pub const TS_MEM_CONFIG: u32 = 0x0000_1654;
/// `regs.rs` `GC_RS_CLEAR_CONTROL`.
pub const RS_CLEAR_CONTROL: u32 = 0x0000_163c;

// PE_MEM_CONFIG fields (hw/state_3d.xml.h:1180-1185).
pub const PE_MEM_CONFIG_COLOR_TS_MODE_SHIFT: u32 = 24;
pub const PE_MEM_CONFIG_COLOR_TS_MODE_MASK: u32 = 0x0100_0000;
pub const PE_MEM_CONFIG_DEPTH_TS_MODE_SHIFT: u32 = 26;
pub const PE_MEM_CONFIG_DEPTH_TS_MODE_MASK: u32 = 0x0400_0000;

// TS_MEM_CONFIG bits (hw/state_3d.xml.h:1456-1470).
pub const TS_MEM_DEPTH_FAST_CLEAR: u32 = 0x0000_0001;
pub const TS_MEM_COLOR_FAST_CLEAR: u32 = 0x0000_0002;
pub const TS_MEM_DEPTH_16BPP: u32 = 0x0000_0008;
pub const TS_MEM_DEPTH_AUTO_DISABLE: u32 = 0x0000_0010;
pub const TS_MEM_COLOR_AUTO_DISABLE: u32 = 0x0000_0020;
pub const TS_MEM_64BPP_FORMAT: u32 = 0x4000_0000;

// RS_CLEAR_CONTROL (hw/state_3d.xml.h:1399-1406).
pub const RS_CLEAR_BITS_MASK: u32 = 0x0000_ffff;
pub const RS_CLEAR_MODE_ENABLED1: u32 = 0x0001_0000;

/// `TS_MODE_128B` / `TS_MODE_256B` (hw/common_3d.xml.h:128-129): the value
/// `PE_MEM_CONFIG_*_TS_MODE` takes. Only 128B applies here: the 256B mode is
/// chosen only with compression (etnaviv_resource.c:206-217), and this part
/// has none outside MSAA.
pub const TS_MODE_128B: u32 = 0;
pub const TS_MODE_256B: u32 = 1;

/// Bytes of surface per TS tile on this part: `etna_screen_get_tile_size`
/// (etnaviv_screen.h:113-126) with CACHE128B256BPERLINE set.
pub const fn tile_size(mode: u32) -> u32 {
    if mode == TS_MODE_256B {
        256
    } else {
        128
    }
}

/// Bits of TS per tile: 4 when CACHE128B256BPERLINE is set
/// (etnaviv_screen.c:880-882).
pub const BITS_PER_TILE: u32 = 4;

/// The word the RS fills the TS buffer with to mark every tile "cleared":
/// `0x11111111` for 4 bits a tile without DEC400 (etnaviv_screen.c:884-886).
pub const TS_CLEAR_VALUE: u32 = 0x1111_1111;

/// Pixel pipes of the GC7000UL (hwdb `NumPixelPipes` 1).
pub const PIXEL_PIPES: u32 = 1;

const fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

/// TS buffer size in bytes for one surface layer of `layer_stride` bytes
/// (the padded surface size), as `etna_screen_resource_alloc_ts` computes it
/// (etnaviv_resource.c:241-249):
/// `align(ceil(layer_stride / (tile_size * 8 / bits_per_tile)), 0x100 * pipes)`.
/// With 128-byte tiles and 4 bits a tile that is one TS byte per 256 bytes of
/// surface. A zero-size surface has no TS (`ts_size == 0`, :252-253).
pub const fn ts_layer_stride(layer_stride: u32, mode: u32) -> u32 {
    let bytes_per_ts_byte = tile_size(mode) * 8 / BITS_PER_TILE;
    align(layer_stride.div_ceil(bytes_per_ts_byte), 0x100 * PIXEL_PIPES)
}

/// Padded size of a TILED or SUPER_TILED render surface: Mesa pads a render
/// target to the RS alignment, 16x4 for TILED (`etna_layout_multiple`,
/// etnaviv_resource.c:384-388) and 64x64 for SUPER_TILED (:389-393).
pub const fn padded_dims(width: u32, height: u32, super_tiled: bool) -> (u32, u32) {
    if super_tiled {
        (align(width, 64), align(height, 64))
    } else {
        (align(width, 16), align(height, 4))
    }
}

/// TS bytes of a `width` x `height` surface of `bpp` bytes a pixel in the
/// given layout, in the 128B mode: the padded size through
/// [`ts_layer_stride`]. `lvl->layer_stride` of a single-level, single-layer
/// surface is `padded_width * padded_height * bpp`.
pub const fn ts_size(width: u32, height: u32, bpp: u32, super_tiled: bool) -> u32 {
    let (pw, ph) = padded_dims(width, height, super_tiled);
    ts_layer_stride(pw * ph * bpp, TS_MODE_128B)
}

/// The RS job that fills the TS buffer with [`TS_CLEAR_VALUE`]
/// (`etna_rs_gen_ts_clear_cmd`, etnaviv_rs.c:339-358): destination TILED,
/// stride `0x40`, 16 wide, `align(ts_layer_stride / 0x40, 4)` high, format
/// A8R8G8B8, clear bits `0xffff`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsClearJob {
    pub dest_stride: u32,
    pub width: u32,
    pub height: u32,
    pub fill: u32,
    /// `RS_CLEAR_CONTROL`: mode ENABLED1 | bits `0xffff`.
    pub clear_control: u32,
}

pub const fn ts_clear_job(ts_layer_stride: u32) -> TsClearJob {
    TsClearJob {
        dest_stride: 0x40,
        width: 16,
        height: align(ts_layer_stride / 0x40, 4),
        fill: TS_CLEAR_VALUE,
        clear_control: RS_CLEAR_MODE_ENABLED1 | RS_CLEAR_BITS_MASK,
    }
}

/// `TS_COLOR_AUTO_DISABLE_COUNT` / `TS_DEPTH_AUTO_DISABLE_COUNT` after a fast
/// clear: `padded_width * padded_height / 16` (etnaviv_rs.c:382-387, :469-475;
/// needs AUTO_DISABLE, which this part has).
pub const fn auto_disable_count(padded_width: u32, padded_height: u32) -> u32 {
    padded_width * padded_height / 16
}

// --- fast-clear words ------------------------------------------------------

/// Colour formats of a render target the fast clear is packed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    /// A8R8G8B8 in the PE (RGBA8 data goes through the RB-swapped internal
    /// format, `translate_pe_internal_format`, etnaviv_format.c:428-441).
    Argb8888,
    /// X8R8G8B8: the unused byte packs as 0xff.
    Xrgb8888,
    Rgb565,
    Argb4444,
    Argb1555,
    R8,
}

fn unorm(v: f32, max: u32) -> u32 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    (v * max as f32 + 0.5) as u32
}

/// `etna_clear_blit_pack_rgba` (etnaviv_clear_blit.c:91-111): pack the colour
/// in the PE format, then replicate to fill 32 bits (1-byte and 2-byte texels
/// are repeated), and return the `(TS_COLOR_CLEAR_VALUE,
/// TS_COLOR_CLEAR_VALUE_EXT)` pair; for texels of up to 32 bits the extension
/// word equals the low one (:106-109).
pub fn color_clear_words(fmt: ColorFormat, rgba: [f32; 4]) -> (u32, u32) {
    let [r, g, b, a] = rgba;
    let (v, bytes) = match fmt {
        ColorFormat::Argb8888 => (unorm(a, 255) << 24 | unorm(r, 255) << 16 | unorm(g, 255) << 8 | unorm(b, 255), 4),
        ColorFormat::Xrgb8888 => (0xff00_0000 | unorm(r, 255) << 16 | unorm(g, 255) << 8 | unorm(b, 255), 4),
        ColorFormat::Rgb565 => (unorm(r, 31) << 11 | unorm(g, 63) << 5 | unorm(b, 31), 2),
        ColorFormat::Argb4444 => (unorm(a, 15) << 12 | unorm(r, 15) << 8 | unorm(g, 15) << 4 | unorm(b, 15), 2),
        ColorFormat::Argb1555 => (unorm(a, 1) << 15 | unorm(r, 31) << 10 | unorm(g, 31) << 5 | unorm(b, 31), 2),
        ColorFormat::R8 => (unorm(r, 255), 1),
    };
    let mut lo = v;
    if bytes == 1 {
        lo = lo << 8 | (lo & 0xff);
    }
    if bytes <= 2 {
        lo = lo << 16 | (lo & 0xffff);
    }
    (lo, lo)
}

/// Depth/stencil surface formats of the PE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthFormat {
    /// `PIPE_FORMAT_Z16_UNORM`.
    Z16,
    /// `PIPE_FORMAT_X8Z24_UNORM`: depth in bits 31..8, no stencil.
    X8Z24,
    /// `PIPE_FORMAT_S8_UINT_Z24_UNORM` (the PE's D24S8): depth in 31..8,
    /// stencil in 7..0. This is the surface the back end allocates.
    S8Z24,
}

/// Which buffers a clear touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearMask {
    pub depth: bool,
    pub stencil: bool,
}

/// Mesa's `lrint` for the scaled depth of [`depth_clear_word`]: round to
/// nearest, ties to even (`util/u_pack_color.h:546-580`), on a value below
/// 2^24. `f64::round_ties_even` is that rounding, but a `std` method absent
/// from the `core` this module must compile against, so the rounding is
/// done here: truncate, then step to the even neighbour on an exact half.
/// NaN, 0 and negatives land on 0, as `lrint` followed by the `as u32` cast
/// did.
fn round_ties_even_u32(x: f64) -> u32 {
    if x.is_nan() || x <= 0.0 {
        return 0;
    }
    let i = x as u32; // truncation toward zero; x is below 2^24 in both uses
    let t = x - i as f64;
    if t > 0.5 || (t == 0.5 && i & 1 == 1) {
        i + 1
    } else {
        i
    }
}

/// `translate_clear_depth_stencil` (etnaviv_translate.h:481-492) over
/// `util_pack_z_stencil` (util/u_pack_color.h:546-580, :613-632): the clear
/// word `TS_DEPTH_CLEAR_VALUE` and the RS fill take. Z16 is repeated in both
/// halves; `z >= 1.0` saturates like Mesa's special case; X8Z24 takes no
/// stencil.
pub fn depth_clear_word(fmt: DepthFormat, depth: f64, stencil: u8) -> u32 {
    match fmt {
        DepthFormat::Z16 => {
            let z = if depth >= 1.0 { 0xffff } else { round_ties_even_u32(depth * 65535.0) };
            z | z << 16
        }
        DepthFormat::X8Z24 | DepthFormat::S8Z24 => {
            let z = if depth >= 1.0 { 0xff_ffff } else { round_ties_even_u32(depth * 16_777_215.0) };
            let w = z << 8;
            if fmt == DepthFormat::S8Z24 {
                w | stencil as u32
            } else {
                w
            }
        }
    }
}

/// The RS clear bits a depth/stencil clear uses (`etna_blit_clear_zs_rs`,
/// etnaviv_rs.c:446-465): Z16 and X8Z24 `0xffff` for depth; S8Z24 `0xeeee`
/// for depth plus `0x1111` for stencil. Zero for an empty mask.
pub const fn depth_clear_bits(fmt: DepthFormat, mask: ClearMask) -> u32 {
    let (d, s) = match fmt {
        DepthFormat::Z16 | DepthFormat::X8Z24 => (0xffff, 0),
        DepthFormat::S8Z24 => (0xeeee, 0x1111),
    };
    (if mask.depth { d } else { 0 }) | (if mask.stencil { s } else { 0 })
}

/// A depth clear takes the TS path only when the whole word is cleared
/// (`new_clear_bits == 0xffff`, etnaviv_rs.c:467). A depth-only clear of S8Z24
/// is a plain RS fill with `RS_CLEAR_CONTROL = 0x0001eeee`.
pub const fn depth_clear_uses_ts(fmt: DepthFormat, mask: ClearMask) -> bool {
    depth_clear_bits(fmt, mask) == 0xffff
}

/// `RS_CLEAR_CONTROL` of a plain (non-TS) RS fill with `bits`.
pub const fn rs_clear_control(bits: u32) -> u32 {
    RS_CLEAR_MODE_ENABLED1 | (bits & RS_CLEAR_BITS_MASK)
}

// --- register values -------------------------------------------------------

/// `PE_MEM_CONFIG` for a framebuffer (`etna_set_framebuffer_state`,
/// etnaviv_state.c:233-235, :458): the TS mode of the colour target when it
/// has TS, and of the depth target when it has TS.
pub const fn pe_mem_config(color_ts_mode: Option<u32>, depth_ts_mode: Option<u32>) -> u32 {
    let c = match color_ts_mode {
        Some(m) => (m << PE_MEM_CONFIG_COLOR_TS_MODE_SHIFT) & PE_MEM_CONFIG_COLOR_TS_MODE_MASK,
        None => 0,
    };
    let d = match depth_ts_mode {
        Some(m) => (m << PE_MEM_CONFIG_DEPTH_TS_MODE_SHIFT) & PE_MEM_CONFIG_DEPTH_TS_MODE_MASK,
        None => 0,
    };
    c | d
}

/// What the framebuffer has for `TS_MEM_CONFIG` (etnaviv_state.c:296-330,
/// :440-470, `etna_update_ts_config` :877-945; the AUTO_DISABLE bits from
/// etnaviv_rs.c:382-387, :469-475).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TsMemConfig {
    /// The colour target is 64 bits a texel.
    pub color_64bpp: bool,
    /// The depth target is 16 bits (`depth_bits == 16`, :468).
    pub depth_16bpp: bool,
    /// The colour level's TS is valid (a fast clear has happened).
    pub color_valid: bool,
    /// The depth level's TS is valid.
    pub depth_valid: bool,
    /// A colour fast clear was queued (AUTO_DISABLE set).
    pub color_auto_disable: bool,
    pub depth_auto_disable: bool,
}

pub const fn ts_mem_config(c: TsMemConfig) -> u32 {
    (if c.color_64bpp { TS_MEM_64BPP_FORMAT } else { 0 })
        | (if c.depth_16bpp { TS_MEM_DEPTH_16BPP } else { 0 })
        | (if c.color_valid { TS_MEM_COLOR_FAST_CLEAR } else { 0 })
        | (if c.depth_valid { TS_MEM_DEPTH_FAST_CLEAR } else { 0 })
        | (if c.color_auto_disable { TS_MEM_COLOR_AUTO_DISABLE } else { 0 })
        | (if c.depth_auto_disable { TS_MEM_DEPTH_AUTO_DISABLE } else { 0 })
}

// ---------------------------------------------------------------------------
// Board probe case 243 `tsprobe` (docs/notes/gc-ts-probe.md, "Part (iii)").
// Pure: what the case reads back becomes an outcome code and a page word.
// No stream is built here.
// ---------------------------------------------------------------------------

/// The page tag of arm `a`'s result page (`0xd0 + a`, a = 0..=3).
pub const PROBE_TAG_ARM: u32 = 0xd0;
/// The page tag of arm `a`'s K3 map page: `0xd4 + a`.
pub const PROBE_TAG_K3: u32 = 0xd4;
/// The page tag of arm `a`'s first-difference page: `0xd8 + a`.
pub const PROBE_TAG_DIFF: u32 = 0xd8;
/// K1's fill value, read back from `T`.
pub const PROBE_FILL: u32 = TS_CLEAR_VALUE;
/// Dwords of the 64x64 A8R8G8B8 surface.
pub const PROBE_DWORDS: usize = 64 * 64;
/// Dwords of `T` checked after K1.
pub const PROBE_TS_DWORDS: usize = 64;
/// The value `D` holds before a resolve.
pub const PROBE_DEST_INIT: u32 = 0xdead_beef;
/// `C`: Mesa's clear word for (0.25, 0.5, 0.75, 1.0) in A8R8G8B8.
pub const PROBE_CLEAR: u32 = 0xff40_80bf;
/// A guard dword's value.
pub const PROBE_GUARD: u32 = 0xa5a5_a5a5;

/// The outcome of one arm; the value is page word w1's low nibble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Outcome {
    /// A fence timeout in any step.
    Hang = 0xf,
    /// A guard dword changed, or `S` changed in K1/K2.
    Guard = 1,
    /// K0's `D` is not `detile(B)`: the RS does not read this layout.
    NoControl = 2,
    /// After K1, `T[0..64]` is not `0x11111111`.
    FillMiss = 3,
    /// K2's `D` is `C` everywhere.
    Yes = 4,
    /// K2's `D` is `detile(B)`: the TS was ignored.
    Ignored = 5,
    /// K2's `D` is still `0xdeadbeef`.
    NoWrite = 6,
    /// Anything else.
    Mixed = 7,
}

/// What one arm reads back. Slices may be empty for a step that did not run
/// (only after a hang, which is classified first).
#[derive(Clone, Copy, Debug)]
pub struct ProbeReadback<'a> {
    /// Fence timeouts over all steps of the arm.
    pub fence_timeouts: u32,
    /// Guard dwords that differ from [`PROBE_GUARD`], all objects.
    pub guard_changed: u32,
    /// `S` differs from `B` after K1 or K2.
    pub surface_changed: bool,
    /// `detile(B)`: the 4096 dwords K0 and an ignoring K2 must give.
    pub expected_b: &'a [u32],
    /// K0's `D`.
    pub k0_d: &'a [u32],
    /// `T[0..64]` after K1.
    pub k1_t: &'a [u32],
    /// K2's `D`.
    pub k2_d: &'a [u32],
    /// `C`, the clear word K2 should return.
    pub clear: u32,
}

/// The outcome code: the first row of the note's table that holds.
pub fn classify(r: &ProbeReadback) -> Outcome {
    if r.fence_timeouts != 0 {
        return Outcome::Hang;
    }
    if r.guard_changed != 0 || r.surface_changed {
        return Outcome::Guard;
    }
    if r.k0_d.len() != PROBE_DWORDS || r.k0_d != r.expected_b {
        return Outcome::NoControl;
    }
    if r.k1_t.len() < PROBE_TS_DWORDS || r.k1_t[..PROBE_TS_DWORDS].iter().any(|&w| w != PROBE_FILL) {
        return Outcome::FillMiss;
    }
    let d = r.k2_d;
    if d.len() == PROBE_DWORDS {
        if d.iter().all(|&w| w == r.clear) {
            return Outcome::Yes;
        }
        if d == r.expected_b {
            return Outcome::Ignored;
        }
        if d.iter().all(|&w| w == PROBE_DEST_INIT) {
            return Outcome::NoWrite;
        }
    }
    Outcome::Mixed
}

/// `count(D == C) << 16 | count(D == detile(B))` (page w3 of the arm and
/// K3 pages); each count is of 4096 and fits 16 bits.
pub fn probe_counts(d: &[u32], clear: u32, expected_b: &[u32]) -> u32 {
    let c = d.iter().filter(|&&w| w == clear).count() as u32;
    let b = d.iter().zip(expected_b).filter(|(a, b)| a == b).count() as u32;
    (c << 16) | (b & 0xffff)
}

/// The tag of arm `arm`'s result page (`0xd0 + arm`).
pub const fn probe_tag(arm: u32) -> u32 {
    PROBE_TAG_ARM + arm
}

/// Page w1 of an arm page: `code | arm << 4 | steps_done << 8 |
/// fence_timeouts << 16`.
pub const fn probe_w1(code: Outcome, arm: u32, steps_done: u32, fence_timeouts: u32) -> u32 {
    (code as u32) | (arm << 4) | (steps_done << 8) | (fence_timeouts << 16)
}

/// `nibble_order` bit 0 of a K3 page: the low nibble covers the first granule.
pub const K3_LOW_NIBBLE_FIRST: u32 = 1;
/// `nibble_order` bit 1 of a K3 page: arm 3 matched `SuperOrder::Interleaved`.
pub const K3_INTERLEAVED: u32 = 2;

/// Page w1 of a K3 page (`0xd4 + arm`): the row-0 map | `nibble_order << 8`.
pub const fn probe_k3_w1(row0_map: u32, nibble_order: u32) -> u32 {
    (row0_map & 0xff) | (nibble_order << 8)
}

/// Page w1 of a first-difference page (`0xd8 + arm`): the index of the first
/// dword of K2's `D` that is neither `C` nor `detile(B)`, `<< 16`, with the
/// low half of the value read; `0xffff_ffff` when there is none.
pub fn probe_diff_w1(d: &[u32], clear: u32, expected_b: &[u32]) -> u32 {
    for (i, (&w, &b)) in d.iter().zip(expected_b).enumerate() {
        if w != clear && w != b {
            return ((i as u32) << 16) | (w & 0xffff);
        }
    }
    0xffff_ffff
}

/// Page w3 of a first-difference page: the first index of `T` that is not
/// `0x11111111` after K1 (`0xffff` when none) `<< 16 | ` the count of changed
/// guard dwords (low 16 bits).
pub fn probe_diff_w3(k1_t: &[u32], guard_changed: u32) -> u32 {
    let first = k1_t.iter().take(PROBE_TS_DWORDS).position(|&w| w != PROBE_FILL).map_or(0xffff, |i| i as u32);
    (first << 16) | (guard_changed & 0xffff)
}

/// A surface layout for K3: a copy of `layout.rs`'s `Layout::Tiled` and
/// `Layout::SuperTiled` with `SuperOrder::Drm` / `Interleaved` (4x4 tiles,
/// 64x64 super-tiles). The test compares [`k3_offset`] with
/// `yakogl::vivante::layout::offset_in`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K3Layout {
    Tiled,
    SuperTiledDrm,
    SuperTiledInterleaved,
}

/// Byte offset of pixel `(x, y)` of a 32-bit surface with `stride` bytes per
/// padded linear row (`layout.rs`: `offset_in`, `tile_index`, `super_index`).
pub const fn k3_offset(layout: K3Layout, x: u32, y: u32, stride: u32) -> usize {
    let (xs, ys, s) = (x as usize, y as usize, stride as usize);
    let tile = ((y & 3) * 4 + (x & 3)) as usize;
    match layout {
        K3Layout::Tiled => (ys & !3) * s + 4 * ((xs & !3) << 2) + 4 * tile,
        _ => {
            let (px, py) = (x & 63, y & 63);
            let hi = if matches!(layout, K3Layout::SuperTiledDrm) {
                ((px & 0x04) << 2) | ((py & 0x0c) << 3) | ((px & 0x38) << 4) | ((py & 0x30) << 6)
            } else {
                ((px & 0x04) << 2)
                    | ((py & 0x04) << 3)
                    | ((px & 0x08) << 3)
                    | ((py & 0x08) << 4)
                    | ((px & 0x10) << 4)
                    | ((py & 0x10) << 5)
                    | ((px & 0x20) << 5)
                    | ((py & 0x20) << 6)
            };
            (ys & !63) * s + 4 * ((xs & !63) << 6) + 4 * (tile + hi as usize)
        }
    }
}

/// K3's expected map: entry `y * width + x` is true when pixel `(x, y)` of
/// the 32-bit `width` x `height` surface lies in a TS granule of `ts` whose
/// nibble reads "cleared" (1), so the resolve returns `C` there and `B`
/// elsewhere. One nibble covers `tile_size(mode)` bytes of surface in offset
/// order; granule n's nibble is the low half of byte n/2 for even n when
/// `low_nibble_first`, else the high half (and the other way for odd n).
/// Host tests only: the image's library has no allocator.
#[cfg(feature = "host-test")]
pub fn k3_expected_clear(
    layout: K3Layout,
    width: u32,
    height: u32,
    mode: u32,
    ts: &[u8],
    low_nibble_first: bool,
) -> Vec<bool> {
    let stride = width * 4;
    let mut out = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let n = k3_offset(layout, x, y, stride) / tile_size(mode) as usize;
            let byte = ts.get(n / 2).copied().unwrap_or(0);
            let low = n.is_multiple_of(2) == low_nibble_first;
            let nib = if low { byte & 0xf } else { byte >> 4 };
            out.push(nib == 1);
        }
    }
    out
}

/// Row 0's map for the page: bit k set when dword `8k` of row 0 (k = 0..8)
/// is cleared in [`k3_expected_clear`]'s result for a surface `width` wide.
pub fn k3_row0_map(clear: &[bool], width: u32) -> u32 {
    let mut m = 0;
    for k in 0..8u32 {
        if 8 * k < width && clear[(8 * k) as usize] {
            m |= 1 << k;
        }
    }
    m
}

// ---------------------------------------------------------------------------
// Case 243's streams: the (register, value) writes of K0 to K3, one arm at a
// time (gc-ts-probe.md, "The steps of one arm" and "The golden diff"). Pure:
// the caller encodes them (`LOAD_STATE` of one register each) and submits
// one kick a step. The words follow `tools/vivante/testdata/ts-clear.golden`
// (mode 29) apart from the diff the note lists.
// ---------------------------------------------------------------------------

/// `regs.rs` `GC_GL_FLUSH_CACHE`.
pub const GL_FLUSH_CACHE: u32 = 0x0000_380c;
/// `regs.rs` `GC_GL_SEMAPHORE_TOKEN`.
pub const GL_SEMAPHORE_TOKEN: u32 = 0x0000_3808;
/// `regs.rs` `GC_GL_STALL_TOKEN`.
pub const GL_STALL_TOKEN: u32 = 0x0000_3c00;
/// `regs.rs` `GC_TS_COLOR_STATUS_BASE`.
pub const TS_COLOR_STATUS_BASE: u32 = 0x0000_1658;
/// `regs.rs` `GC_TS_COLOR_SURFACE_BASE`.
pub const TS_COLOR_SURFACE_BASE: u32 = 0x0000_165c;
/// `regs.rs` `GC_TS_COLOR_CLEAR_VALUE`.
pub const TS_COLOR_CLEAR_VALUE: u32 = 0x0000_1660;
/// `regs.rs` `GC_TS_COLOR_AUTO_DISABLE_COUNT`.
pub const TS_COLOR_AUTO_DISABLE_COUNT: u32 = 0x0000_1674;
/// `regs.rs` `GC_TS_COLOR_CLEAR_VALUE_EXT`.
pub const TS_COLOR_CLEAR_VALUE_EXT: u32 = 0x0000_16bc;
/// `regs.rs` `GC_RS_KICKER`.
pub const RS_KICKER: u32 = 0x0000_1600;
/// `regs.rs` `GC_RS_CONFIG`.
pub const RS_CONFIG: u32 = 0x0000_1604;
/// `regs.rs` `GC_RS_SOURCE_STRIDE`.
pub const RS_SOURCE_STRIDE: u32 = 0x0000_160c;
/// `regs.rs` `GC_RS_DEST_STRIDE`.
pub const RS_DEST_STRIDE: u32 = 0x0000_1614;
/// `regs.rs` `GC_RS_WINDOW_SIZE`.
pub const RS_WINDOW_SIZE: u32 = 0x0000_1620;
/// `regs.rs` `rs_dither(0)`; `RS_DITHER[1]` follows at +4.
pub const RS_DITHER0: u32 = 0x0000_1630;
/// `regs.rs` `rs_fill_value(0)`; `RS_FILL_VALUE[1..4]` follow at +4.
pub const RS_FILL_VALUE0: u32 = 0x0000_1640;
/// `regs.rs` `GC_RS_EXTRA_CONFIG`.
pub const RS_EXTRA_CONFIG: u32 = 0x0000_16a0;
/// `regs.rs` `GC_RS_SINGLE_BUFFER`.
pub const RS_SINGLE_BUFFER: u32 = 0x0000_16b8;
/// `regs.rs` `rs_pipe_source_addr(0)`.
pub const RS_PIPE_SOURCE_ADDR0: u32 = 0x0000_16c0;
/// `regs.rs` `rs_pipe_dest_addr(0)`.
pub const RS_PIPE_DEST_ADDR0: u32 = 0x0000_16e0;
/// `regs.rs` `rs_pipe_offset(0)`; `RS_PIPE_OFFSET[1]` follows at +4.
pub const RS_PIPE_OFFSET0: u32 = 0x0000_1700;

/// `regs.rs` `GC_FLUSH_DEPTH | GC_FLUSH_COLOR`.
pub const GL_FLUSH_COLOR_DEPTH: u32 = 0x0000_0003;
/// `regs.rs` `stall_token(GC_SYNC_RA, GC_SYNC_PE)`: the golden's semaphore
/// and stall word.
pub const STALL_RA_PE: u32 = 0x0000_0705;
/// `regs.rs` `GC_TS_FLUSH`.
pub const TS_FLUSH: u32 = 0x0000_0001;
/// `regs.rs` `GC_RS_KICKER_VALUE`.
pub const RS_KICKER_VALUE: u32 = 0xbeeb_beeb;
/// `regs.rs` `GC_RS_SINGLE_BUFFER_ENABLE`.
pub const RS_SINGLE_BUFFER_ENABLE: u32 = 0x0000_0001;
/// `regs.rs` `GC_RS_SOURCE_TILED`.
pub const RS_CONFIG_SOURCE_TILED: u32 = 0x0000_0080;
/// `regs.rs` `GC_RS_DEST_TILED`.
pub const RS_CONFIG_DEST_TILED: u32 = 0x0000_4000;
/// `regs.rs` `GC_RS_FORMAT_A8R8G8B8`.
pub const RS_FORMAT_A8R8G8B8: u32 = 0x0000_0006;
/// `regs.rs` `GC_RS_SOURCE_STRIDE_TILING`.
pub const RS_STRIDE_TILING: u32 = 0x8000_0000;
/// `regs.rs` `GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW`.
pub const RS_STRIDE_SUPER_TILED_NEW: u32 = 0x0800_0000;
/// `VIVS_RS_SOURCE_STRIDE_TS_MODE` (hw/state_3d.xml.h:1371-1373; not in
/// `regs.rs`): the source's TS mode, 1 = 256B granules. The golden's resolve
/// sets it (`0xa8000400`) and leaves `RS_EXTRA_CONFIG` at 0.
pub const RS_SOURCE_STRIDE_TS_MODE: u32 = 0x2000_0000;
/// `VIVS_TS_MEM_CONFIG_COLOR_COMPRESSION` (hw/state_3d.xml.h:1456-1470; not
/// in `regs.rs`).
pub const TS_MEM_COLOR_COMPRESSION: u32 = 0x0000_0080;
/// `VIVS_TS_MEM_CONFIG_COLOR_COMPRESSION_FORMAT(3)` (hw/state_3d.xml.h:
/// 1456-1470; not in `regs.rs`): the format field of the golden's `0x382`.
pub const TS_MEM_COLOR_COMPRESSION_FORMAT_3: u32 = 0x0000_0300;

/// The probe surface's side in pixels (64x64 A8R8G8B8, 16 KiB).
pub const PROBE_SIDE: u32 = 64;
/// Bytes of `S` and of `D`.
pub const PROBE_SURFACE_BYTES: u32 = PROBE_SIDE * PROBE_SIDE * 4;
/// Bytes of `T`: `ts_size(64, 64, 4, _)`, both layouts.
pub const PROBE_TS_BYTES: u32 = 256;

/// The GPU addresses of the case's objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeAddrs {
    /// `S`, the 64x64 colour surface.
    pub s: u32,
    /// `T`, its TS buffer.
    pub t: u32,
    /// `D`, the linear destination.
    pub d: u32,
}

/// What one arm programs (gc-ts-probe.md, "Arms").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeArm {
    /// `S` is SUPER_TILED (arm 3), else TILED.
    pub super_tiled: bool,
    /// [`TS_MODE_128B`] or [`TS_MODE_256B`] (`RS_SOURCE_STRIDE` bit 29).
    pub ts_mode: u32,
    /// `TS_MEM_CONFIG` of K2 and K3.
    pub mem_config: u32,
    /// `TS_COLOR_AUTO_DISABLE_COUNT`, written by arm 1 only.
    pub auto_disable_count: Option<u32>,
}

/// Arms 0 to 3 (tags `0xd0..0xd3`); `None` past 3.
pub const fn probe_arm(arm: u32) -> Option<ProbeArm> {
    let (pw, ph) = padded_dims(PROBE_SIDE, PROBE_SIDE, false);
    let fast = TS_MEM_COLOR_FAST_CLEAR;
    Some(match arm {
        0 => ProbeArm { super_tiled: false, ts_mode: TS_MODE_128B, mem_config: fast, auto_disable_count: None },
        1 => ProbeArm {
            super_tiled: false,
            ts_mode: TS_MODE_128B,
            mem_config: fast | TS_MEM_COLOR_AUTO_DISABLE,
            auto_disable_count: Some(auto_disable_count(pw, ph)),
        },
        2 => ProbeArm {
            super_tiled: false,
            ts_mode: TS_MODE_256B,
            mem_config: fast | TS_MEM_COLOR_COMPRESSION | TS_MEM_COLOR_COMPRESSION_FORMAT_3,
            auto_disable_count: None,
        },
        3 => ProbeArm { super_tiled: true, ts_mode: TS_MODE_128B, mem_config: fast, auto_disable_count: None },
        _ => return None,
    })
}

/// `RS_SOURCE_STRIDE` of a resolve of `S`: the 64-pixel row times 4 (a tiled
/// side, `source_stride_shift` 2), `TILING | SUPER_TILED_NEW` for a
/// super-tiled `S` and `TS_MODE` for 256B granules, as Mesa's
/// `etna_compile_rs_state` (etnaviv_rs.c:95-135) gives them: TILED `0x400`,
/// TILED 256B `0x20000400`, SUPER_TILED `0x88000400`. K0 passes mode 128B.
///
/// gc-ts-probe.md writes TILED as `0x80000400`; that `TILING` bit is Mesa's
/// super-tiled bit (`COND(src_super, VIVS_RS_SOURCE_STRIDE_TILING)`, and
/// `rs_interp.rs` and `tiled_copy.rs` read it so), so the TILED arms leave
/// it clear.
pub const fn probe_source_stride(arm: &ProbeArm, ts_mode: u32) -> u32 {
    let mut v = (PROBE_SIDE * 4) << 2;
    if arm.super_tiled {
        v |= RS_STRIDE_TILING | RS_STRIDE_SUPER_TILED_NEW;
    }
    if ts_mode == TS_MODE_256B {
        v |= RS_SOURCE_STRIDE_TS_MODE;
    }
    v
}

/// The flush and RA-to-PE stall that opens every step, then `TS_FLUSH_CACHE`.
#[cfg(feature = "host-test")]
fn probe_prologue(w: &mut Vec<(u32, u32)>) {
    w.extend([
        (GL_FLUSH_CACHE, GL_FLUSH_COLOR_DEPTH),
        (GL_SEMAPHORE_TOKEN, STALL_RA_PE),
        (GL_STALL_TOKEN, STALL_RA_PE),
        (TS_FLUSH_CACHE, TS_FLUSH),
    ]);
}

/// One side of an RS block: the address and the stride word.
#[cfg(feature = "host-test")]
#[derive(Clone, Copy)]
struct RsSide {
    addr: u32,
    stride: u32,
}

/// One RS block, from `RS_CONFIG` to the last `RS_SINGLE_BUFFER`, in the
/// golden's order. A fill has no source address (the golden writes none) and
/// a source stride of 0.
#[cfg(feature = "host-test")]
fn probe_rs_block(
    w: &mut Vec<(u32, u32)>,
    config: u32,
    src: Option<RsSide>,
    dst: RsSide,
    (width, height): (u32, u32),
    (clear_control, fill0): (u32, u32),
) {
    let src_stride = src.map_or(0, |s| s.stride);
    w.extend([(RS_CONFIG, config), (RS_SOURCE_STRIDE, src_stride), (RS_DEST_STRIDE, dst.stride)]);
    if let Some(s) = src {
        w.push((RS_PIPE_SOURCE_ADDR0, s.addr));
    }
    w.extend([
        (RS_PIPE_DEST_ADDR0, dst.addr),
        (RS_PIPE_OFFSET0, 0),
        (RS_PIPE_OFFSET0 + 4, 0),
        (RS_WINDOW_SIZE, height << 16 | width),
        (RS_DITHER0, 0xffff_ffff),
        (RS_DITHER0 + 4, 0xffff_ffff),
        (RS_CLEAR_CONTROL, clear_control),
        (RS_FILL_VALUE0, fill0),
        (RS_FILL_VALUE0 + 4, 0),
        (RS_FILL_VALUE0 + 8, 0),
        (RS_FILL_VALUE0 + 12, 0),
        (RS_EXTRA_CONFIG, 0),
        (RS_SINGLE_BUFFER, RS_SINGLE_BUFFER_ENABLE),
        (RS_KICKER, RS_KICKER_VALUE),
        (RS_SINGLE_BUFFER, 0),
    ]);
}

/// `RS_CONFIG` of the resolve: A8R8G8B8 both ways, the source tiled, no
/// `SWAP_RB` (a bit copy: `D` gets `S`'s dwords as they are).
pub const PROBE_RESOLVE_CONFIG: u32 = RS_FORMAT_A8R8G8B8 | RS_CONFIG_SOURCE_TILED | RS_FORMAT_A8R8G8B8 << 8;
/// `RS_CONFIG` of the TS fill: A8R8G8B8 both ways, `DEST_TILED`.
pub const PROBE_FILL_CONFIG: u32 = RS_FORMAT_A8R8G8B8 | RS_FORMAT_A8R8G8B8 << 8 | RS_CONFIG_DEST_TILED;

#[cfg(feature = "host-test")]
fn probe_resolve(w: &mut Vec<(u32, u32)>, arm: &ProbeArm, a: &ProbeAddrs, ts_mode: u32) {
    let src = RsSide { addr: a.s, stride: probe_source_stride(arm, ts_mode) };
    let dst = RsSide { addr: a.d, stride: PROBE_SIDE * 4 };
    probe_rs_block(w, PROBE_RESOLVE_CONFIG, Some(src), dst, (PROBE_SIDE, PROBE_SIDE), (0, 0));
}

/// K0, the control resolve: `S` to `D` with `TS_MEM_CONFIG` 0.
#[cfg(feature = "host-test")]
pub fn probe_k0(arm: &ProbeArm, a: &ProbeAddrs) -> Vec<(u32, u32)> {
    let mut w = Vec::new();
    probe_prologue(&mut w);
    w.push((TS_MEM_CONFIG, 0));
    probe_resolve(&mut w, arm, a, TS_MODE_128B);
    w
}

/// K1, the TS fill: [`ts_clear_job`] of `S`'s TS written to `T`
/// (`etna_rs_gen_ts_clear_cmd`; the golden's first fill block). The same for
/// every arm.
#[cfg(feature = "host-test")]
pub fn probe_k1(a: &ProbeAddrs) -> Vec<(u32, u32)> {
    let job = ts_clear_job(ts_layer_stride(PROBE_SURFACE_BYTES, TS_MODE_128B));
    let mut w = Vec::new();
    probe_prologue(&mut w);
    let dst = RsSide { addr: a.t, stride: job.dest_stride << 2 };
    probe_rs_block(&mut w, PROBE_FILL_CONFIG, None, dst, (job.width, job.height), (job.clear_control, job.fill));
    w
}

/// K2, the TS-sourced resolve: K0 with the arm's TS words before the RS
/// block.
#[cfg(feature = "host-test")]
pub fn probe_k2(arm: &ProbeArm, a: &ProbeAddrs) -> Vec<(u32, u32)> {
    let mut w = Vec::new();
    probe_prologue(&mut w);
    w.extend([
        (TS_MEM_CONFIG, arm.mem_config),
        (TS_COLOR_STATUS_BASE, a.t),
        (TS_COLOR_SURFACE_BASE, a.s),
        (TS_COLOR_CLEAR_VALUE, PROBE_CLEAR),
        (TS_COLOR_CLEAR_VALUE_EXT, PROBE_CLEAR),
    ]);
    if let Some(n) = arm.auto_disable_count {
        w.push((TS_COLOR_AUTO_DISABLE_COUNT, n));
    }
    probe_resolve(&mut w, arm, a, arm.ts_mode);
    w
}

/// K3, the pattern resolve: K2's stream again, after the CPU wrote `T` as
/// [`probe_k3_ts`] and reset `D`.
#[cfg(feature = "host-test")]
pub fn probe_k3(arm: &ProbeArm, a: &ProbeAddrs) -> Vec<(u32, u32)> {
    probe_k2(arm, a)
}

/// `T` as the CPU writes it before K3: 64 bytes of `0x01` (low nibble
/// cleared, high nibble not), then 192 bytes of 0.
pub fn probe_k3_ts() -> [u8; PROBE_TS_BYTES as usize] {
    let mut t = [0u8; PROBE_TS_BYTES as usize];
    t[..64].fill(0x01);
    t
}

/// `B(i)`: dword `i` of `S` as the CPU writes it before K0.
pub const fn probe_b(i: u32) -> u32 {
    0xa500_0000 | i
}
