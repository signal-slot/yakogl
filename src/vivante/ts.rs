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
            let low = (n % 2 == 0) == low_nibble_first;
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
