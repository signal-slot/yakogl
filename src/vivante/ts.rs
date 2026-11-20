// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): tile-status (TS) buffer sizes and fast-clear words as
//! pure functions (groundwork for `docs/notes/gc-tiled-targets.md`).
//!
//! Everything here is what Mesa 26.2.1's etnaviv computes for this part
//! (hwdb `gc7000ULN_v123`: FAST_CLEAR, CACHE128B256BPERLINE, AUTO_DISABLE,
//! no DEC400, one pixel pipe, no BLT), worked out without a device. Citations
//! are files under `src/gallium/drivers/etnaviv/` unless a path says
//! otherwise. Nothing is wired to the back end's draws.
//!
//! The pure part is self-contained: no `crate::` or `super::` path; the
//! host test `tests/gc_ts_words.rs` takes it from the crate as
//! `yakogl::vivante::ts`. The register addresses are copies of the `regs.rs`
//! constants named beside each one; the test file compares them against
//! `regs.rs`. The one exception is the `test-knobs` hook of the board probe
//! (`gc_debug_ts_probe`, the private `hook` module at the end), which
//! allocates through the OSAL and submits its own kicks.

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

/// Where a step's `(register, value)` writes go, one at a time: a `Vec` in
/// the host tests ([`probe_k0`] and the others), the command buffer on the
/// board (the `test-knobs` hook [`gc_debug_ts_probe`], which has no
/// allocator).
pub type ProbeSink<'a> = &'a mut dyn FnMut(u32, u32);

/// The flush and RA-to-PE stall that opens every step, then `TS_FLUSH_CACHE`.
fn probe_prologue(w: ProbeSink) {
    for (r, v) in [
        (GL_FLUSH_CACHE, GL_FLUSH_COLOR_DEPTH),
        (GL_SEMAPHORE_TOKEN, STALL_RA_PE),
        (GL_STALL_TOKEN, STALL_RA_PE),
        (TS_FLUSH_CACHE, TS_FLUSH),
    ] {
        w(r, v);
    }
}

/// One side of an RS block: the address and the stride word.
#[derive(Clone, Copy)]
struct RsSide {
    addr: u32,
    stride: u32,
}

/// One RS block, from `RS_CONFIG` to the last `RS_SINGLE_BUFFER`, in the
/// golden's order. A fill has no source address (the golden writes none) and
/// a source stride of 0.
fn probe_rs_block(
    w: ProbeSink,
    config: u32,
    src: Option<RsSide>,
    dst: RsSide,
    (width, height): (u32, u32),
    (clear_control, fill0): (u32, u32),
) {
    let src_stride = src.map_or(0, |s| s.stride);
    w(RS_CONFIG, config);
    w(RS_SOURCE_STRIDE, src_stride);
    w(RS_DEST_STRIDE, dst.stride);
    if let Some(s) = src {
        w(RS_PIPE_SOURCE_ADDR0, s.addr);
    }
    for (r, v) in [
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
    ] {
        w(r, v);
    }
}

/// The writes of a sink-built step, collected (host tests).
#[cfg(feature = "host-test")]
fn probe_collect(build: impl FnOnce(ProbeSink)) -> Vec<(u32, u32)> {
    let mut w = Vec::new();
    build(&mut |r, v| w.push((r, v)));
    w
}

/// `RS_CONFIG` of the resolve: A8R8G8B8 both ways, the source tiled, no
/// `SWAP_RB` (a bit copy: `D` gets `S`'s dwords as they are).
pub const PROBE_RESOLVE_CONFIG: u32 = RS_FORMAT_A8R8G8B8 | RS_CONFIG_SOURCE_TILED | RS_FORMAT_A8R8G8B8 << 8;
/// `RS_CONFIG` of the TS fill: A8R8G8B8 both ways, `DEST_TILED`.
pub const PROBE_FILL_CONFIG: u32 = RS_FORMAT_A8R8G8B8 | RS_FORMAT_A8R8G8B8 << 8 | RS_CONFIG_DEST_TILED;

fn probe_resolve(w: ProbeSink, arm: &ProbeArm, a: &ProbeAddrs, ts_mode: u32) {
    let src = RsSide { addr: a.s, stride: probe_source_stride(arm, ts_mode) };
    let dst = RsSide { addr: a.d, stride: PROBE_SIDE * 4 };
    probe_rs_block(w, PROBE_RESOLVE_CONFIG, Some(src), dst, (PROBE_SIDE, PROBE_SIDE), (0, 0));
}

/// K0's writes into `w` (see [`probe_k0`]).
pub fn probe_k0_into(w: ProbeSink, arm: &ProbeArm, a: &ProbeAddrs) {
    probe_prologue(w);
    w(TS_MEM_CONFIG, 0);
    probe_resolve(w, arm, a, TS_MODE_128B);
}

/// K1's writes into `w` (see [`probe_k1`]).
pub fn probe_k1_into(w: ProbeSink, a: &ProbeAddrs) {
    let job = ts_clear_job(ts_layer_stride(PROBE_SURFACE_BYTES, TS_MODE_128B));
    probe_prologue(w);
    let dst = RsSide { addr: a.t, stride: job.dest_stride << 2 };
    probe_rs_block(w, PROBE_FILL_CONFIG, None, dst, (job.width, job.height), (job.clear_control, job.fill));
}

/// K2's writes into `w` (see [`probe_k2`]); K3 is the same stream.
pub fn probe_k2_into(w: ProbeSink, arm: &ProbeArm, a: &ProbeAddrs) {
    probe_prologue(w);
    for (r, v) in [
        (TS_MEM_CONFIG, arm.mem_config),
        (TS_COLOR_STATUS_BASE, a.t),
        (TS_COLOR_SURFACE_BASE, a.s),
        (TS_COLOR_CLEAR_VALUE, PROBE_CLEAR),
        (TS_COLOR_CLEAR_VALUE_EXT, PROBE_CLEAR),
    ] {
        w(r, v);
    }
    if let Some(n) = arm.auto_disable_count {
        w(TS_COLOR_AUTO_DISABLE_COUNT, n);
    }
    probe_resolve(w, arm, a, arm.ts_mode);
}

/// K0, the control resolve: `S` to `D` with `TS_MEM_CONFIG` 0.
#[cfg(feature = "host-test")]
pub fn probe_k0(arm: &ProbeArm, a: &ProbeAddrs) -> Vec<(u32, u32)> {
    probe_collect(|w| probe_k0_into(w, arm, a))
}

/// K1, the TS fill: [`ts_clear_job`] of `S`'s TS written to `T`
/// (`etna_rs_gen_ts_clear_cmd`; the golden's first fill block). The same for
/// every arm.
#[cfg(feature = "host-test")]
pub fn probe_k1(a: &ProbeAddrs) -> Vec<(u32, u32)> {
    probe_collect(|w| probe_k1_into(w, a))
}

/// K2, the TS-sourced resolve: K0 with the arm's TS words before the RS
/// block.
#[cfg(feature = "host-test")]
pub fn probe_k2(arm: &ProbeArm, a: &ProbeAddrs) -> Vec<(u32, u32)> {
    probe_collect(|w| probe_k2_into(w, arm, a))
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

// ---------------------------------------------------------------------------
// Case 246 `tsprobe` on the board (gc-ts-probe.md's case 243: 243 to 245 were
// taken by then). What one arm read, reduced to the numbers the C twin
// (`driver/tools/pvrlab/gl_test_probe_decode.inc`, `struct tsp_readback`)
// classifies and packs. Pure and without allocation, so the board's library
// has it.
// ---------------------------------------------------------------------------

/// "No such index" in [`TspReadback`]'s first-index fields (`TSP_NOT_FOUND`).
pub const TSP_NOT_FOUND: u32 = 0xffff;
/// `nibble_order` bit 7 of a K3 page: the row-0 map is none of the candidate
/// maps (the CRC is kept for the host to decode). Bits 0 and 1 are
/// [`K3_LOW_NIBBLE_FIRST`] and [`K3_INTERLEAVED`].
pub const K3_NO_MATCH: u32 = 0x80;

/// `struct tsp_readback` of `gl_test_probe_decode.inc`, field for field: what
/// the `test-knobs` hook `gc_debug_ts_probe` hands the case. The C's
/// `tsp_classify` over it is [`classify`] over the slices it was reduced
/// from.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TspReadback {
    /// 0..=3.
    pub arm: u32,
    /// K0..K3 kicks that completed.
    pub steps_done: u32,
    /// Fence timeouts in any step (no kick runs after the first).
    pub timeouts: u32,
    /// Guard dwords that changed, all objects.
    pub guard_changed: u32,
    /// 1: `S` changed in K1 or K2.
    pub s_changed: u32,
    /// 1: K0's `D` is `detile(B)`.
    pub k0_ok: u32,
    /// First index of `T[0..64]` that is not `0x11111111` after K1,
    /// [`TSP_NOT_FOUND`] if none (0 when K1 did not run).
    pub t_first_bad: u32,
    /// K2: `count(D == C)`, of 4096.
    pub n_clear: u32,
    /// K2: `count(D == detile(B))`.
    pub n_detile: u32,
    /// K2: `count(D == 0xdeadbeef)`.
    pub n_dead: u32,
    /// CRC-32 of K2's `D` (of K0's when K2 did not run, 0 when neither did).
    pub crc: u32,
    /// K2: the first dword of `D` that is neither `C` nor `detile(B)`,
    /// [`TSP_NOT_FOUND`] if none.
    pub first_idx: u32,
    /// The value read there.
    pub first_val: u32,
    /// 1: K3 ran (only after K2 read `YES`).
    pub k3_present: u32,
    /// Row-0 map of K3's `D`: bit k = dword `8k` of row 0 is `C`.
    pub k3_map: u32,
    /// [`k3_nibble_order`].
    pub k3_nibble_order: u32,
    pub k3_crc: u32,
    pub k3_n_clear: u32,
    pub k3_n_detile: u32,
}

/// The layout `detile(B)` is read in: TILED, or SUPER_TILED in one of the
/// two super-tile orders (arm 3).
pub const fn probe_layout(arm: &ProbeArm, interleaved: bool) -> K3Layout {
    if !arm.super_tiled {
        K3Layout::Tiled
    } else if interleaved {
        K3Layout::SuperTiledInterleaved
    } else {
        K3Layout::SuperTiledDrm
    }
}

/// Dword `lin` (`y * 64 + x`) of `detile(B)`: `B` at the dword `layout`
/// puts pixel `(x, y)` at (`tsp_detile_expect`).
pub const fn probe_expected_b(layout: K3Layout, lin: u32) -> u32 {
    let (x, y) = (lin % PROBE_SIDE, lin / PROBE_SIDE);
    probe_b((k3_offset(layout, x, y, PROBE_SIDE * 4) / 4) as u32)
}

/// Row 0's map of `k3_expected_clear` without its allocation: bit k set
/// when pixel `(8k, 0)` lies in a granule whose nibble of `ts` is 1.
pub fn k3_expected_row0(layout: K3Layout, mode: u32, ts: &[u8], low_nibble_first: bool) -> u32 {
    let mut m = 0;
    for k in 0..8u32 {
        let n = k3_offset(layout, 8 * k, 0, PROBE_SIDE * 4) / tile_size(mode) as usize;
        let byte = ts.get(n / 2).copied().unwrap_or(0);
        let low = n.is_multiple_of(2) == low_nibble_first;
        if (if low { byte & 0xf } else { byte >> 4 }) == 1 {
            m |= 1 << k;
        }
    }
    m
}

/// A K3 page's `nibble_order`: which candidate `map` is, for `T` as
/// [`probe_k3_ts`] writes it. [`K3_LOW_NIBBLE_FIRST`] when the low nibble
/// covers the first granule, 0 for the high one, plus [`K3_INTERLEAVED`] when
/// arm 3's map is the `Interleaved` super-tile order's (`Drm` is tried
/// first); [`K3_NO_MATCH`] when it is none.
pub fn k3_nibble_order(map: u32, arm: &ProbeArm) -> u32 {
    let ts = probe_k3_ts();
    let orders: &[(bool, u32)] = if arm.super_tiled { &[(false, 0), (true, K3_INTERLEAVED)] } else { &[(false, 0)] };
    for &(interleaved, bit) in orders {
        for (low, lbit) in [(true, K3_LOW_NIBBLE_FIRST), (false, 0)] {
            if k3_expected_row0(probe_layout(arm, interleaved), arm.ts_mode, &ts, low) == map {
                return bit | lbit;
            }
        }
    }
    K3_NO_MATCH
}

/// CRC-32 (IEEE, reflected, as zlib and the C's `pd_crc32`) of `words` as
/// little-endian bytes.
pub fn probe_crc32(words: &[u32]) -> u32 {
    let mut crc = !0u32;
    for b in words.iter().flat_map(|w| w.to_le_bytes()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// What one arm read, step by step: a slice is empty for a step that did
/// not run.
#[derive(Clone, Copy, Debug)]
pub struct ArmReads<'a> {
    pub arm: u32,
    pub steps_done: u32,
    pub fence_timeouts: u32,
    pub guard_changed: u32,
    pub surface_changed: bool,
    /// `detile(B)` in the layout K0 matched (else the first candidate).
    pub expected_b: &'a [u32],
    pub k0_d: &'a [u32],
    /// `T[0..64]` after K1.
    pub k1_t: &'a [u32],
    pub k2_d: &'a [u32],
    pub k3_d: &'a [u32],
}

/// The outcome ([`classify`]) and the record the C twin packs.
pub fn probe_readback(r: &ArmReads, arm: &ProbeArm) -> (Outcome, TspReadback) {
    let outcome = classify(&ProbeReadback {
        fence_timeouts: r.fence_timeouts,
        guard_changed: r.guard_changed,
        surface_changed: r.surface_changed,
        expected_b: r.expected_b,
        k0_d: r.k0_d,
        k1_t: r.k1_t,
        k2_d: r.k2_d,
        clear: PROBE_CLEAR,
    });
    let is_b = |i: usize, w: u32| r.expected_b.get(i) == Some(&w);
    let count =
        |d: &[u32], f: &dyn Fn(usize, u32) -> bool| d.iter().enumerate().filter(|&(i, &w)| f(i, w)).count() as u32;
    let d = r.k2_d;
    let first = d.iter().enumerate().position(|(i, &w)| w != PROBE_CLEAR && !is_b(i, w));
    let mut out = TspReadback {
        arm: r.arm,
        steps_done: r.steps_done,
        timeouts: r.fence_timeouts,
        guard_changed: r.guard_changed,
        s_changed: r.surface_changed as u32,
        k0_ok: (r.k0_d.len() == PROBE_DWORDS && r.k0_d == r.expected_b) as u32,
        t_first_bad: if r.k1_t.len() >= PROBE_TS_DWORDS {
            r.k1_t[..PROBE_TS_DWORDS].iter().position(|&w| w != PROBE_FILL).map_or(TSP_NOT_FOUND, |i| i as u32)
        } else {
            0
        },
        n_clear: count(d, &|_, w| w == PROBE_CLEAR),
        n_detile: count(d, &is_b),
        n_dead: count(d, &|_, w| w == PROBE_DEST_INIT),
        crc: if !d.is_empty() {
            probe_crc32(d)
        } else if !r.k0_d.is_empty() {
            probe_crc32(r.k0_d)
        } else {
            0
        },
        first_idx: first.map_or(TSP_NOT_FOUND, |i| i as u32),
        first_val: first.map_or(0, |i| d[i]),
        ..TspReadback::default()
    };
    let k3 = r.k3_d;
    if !k3.is_empty() {
        let map = (0..8usize).filter(|&k| k3.get(8 * k) == Some(&PROBE_CLEAR)).fold(0u32, |m, k| m | 1 << k);
        out.k3_present = 1;
        out.k3_map = map;
        out.k3_nibble_order = k3_nibble_order(map, arm);
        out.k3_crc = probe_crc32(k3);
        out.k3_n_clear = count(k3, &|_, w| w == PROBE_CLEAR);
        out.k3_n_detile = count(k3, &is_b);
    }
    (outcome, out)
}

// ---------------------------------------------------------------------------
// The hook (`test-knobs`): case 246 calls it once a boot, for one arm.
// ---------------------------------------------------------------------------

/// `gc_debug_ts_probe`'s refusals, for a step that could not run: bad
/// arguments. A probe that ran (a hang included) returns its [`Outcome`]
/// code instead.
pub const PROBE_ERR_ARGS: i32 = -1;
/// The arm is past 3.
pub const PROBE_ERR_ARM: i32 = -2;
/// The OSAL has no `submit` or no `wait`.
pub const PROBE_ERR_OSAL: i32 = -3;
/// An allocation failed.
pub const PROBE_ERR_ALLOC: i32 = -4;
/// An object's GPU address does not fit 32 bits.
pub const PROBE_ERR_ADDR: i32 = -5;
/// An allocation has no CPU view.
pub const PROBE_ERR_CPU: i32 = -6;
/// The back end's own kicks did not finish first.
pub const PROBE_ERR_FINISH: i32 = -7;
/// The OSAL refused a submit.
pub const PROBE_ERR_SUBMIT: i32 = -8;

#[cfg(feature = "test-knobs")]
pub use self::hook::gc_debug_ts_probe;

#[cfg(feature = "test-knobs")]
mod hook {
    use super::*;
    use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
    use crate::vivante::backend;
    use crate::vivante::kick::{GcKick, GC_KICK_MAGIC, GC_KICK_TAIL_BYTES};
    use crate::vivante::regs::load_state;
    use crate::vivante::state::cache_flush_raw;
    use core::ffi::c_void;

    /// The guard on each side of an object, and the objects' alignment.
    const GUARD: usize = 0x1000;
    /// `backend.rs`'s `GC_HEAP_COMMAND` (private there): the heap of every
    /// command stream the back end submits.
    const HEAP_COMMAND: u64 = 0x6000_0000;
    /// Words of the probe's command buffer: K2, the longest step, is 64.
    const CMD_WORDS: usize = 256;
    /// A fence that has not signalled after this long is a hang
    /// (gc-ts-probe.md: 2 s, as the image's `s_hang_case`).
    const WAIT_US: i64 = 2_000_000;
    const N: usize = PROBE_DWORDS;

    /// One guarded object: `size` bytes at a 0x1000-aligned GPU address
    /// below 4 GiB, with at least [`GUARD`] bytes of allocation either side.
    struct Obj {
        mem: OsalGpuMem,
        off: usize,
        size: usize,
    }

    impl Obj {
        fn new(heap: u64, size: usize) -> Result<Obj, i32> {
            let mut o = Obj { mem: OsalGpuMem::ZERO, off: 0, size };
            if osal::gpu_alloc(heap, size + 3 * GUARD, OSAL_MEM_DATA, &mut o.mem) != 0 {
                return Err(PROBE_ERR_ALLOC);
            }
            if o.mem.cpu.is_null() || !(o.mem.cpu as usize).is_multiple_of(4) {
                return Err(PROBE_ERR_CPU);
            }
            let at = (o.mem.gpu + GUARD as u64).next_multiple_of(GUARD as u64);
            o.off = (at - o.mem.gpu) as usize;
            if at + (size + GUARD) as u64 > u32::MAX as u64 {
                return Err(PROBE_ERR_ADDR);
            }
            Ok(o)
        }

        fn gpu(&self) -> u32 {
            (self.mem.gpu + self.off as u64) as u32
        }

        /// The whole allocation as dwords, as the CPU sees it now.
        fn all(&mut self) -> &mut [u32] {
            if let Some(f) = osal::ops().and_then(|o| o.cache_invd) {
                // SAFETY: the allocation's own mapping and size.
                unsafe { f(self.mem.cpu, self.mem.size) };
            }
            // SAFETY: the allocation is `mem.size` bytes long, mapped at
            // `mem.cpu` (4-aligned, checked in `new`), and no other view of
            // it is live: every caller drops the slice before the next kick.
            unsafe { core::slice::from_raw_parts_mut(self.mem.cpu as *mut u32, self.mem.size / 4) }
        }

        /// The object's dwords.
        fn words(&mut self) -> &mut [u32] {
            let (off, size) = (self.off / 4, self.size / 4);
            &mut self.all()[off..off + size]
        }

        /// The guards' dwords that are not [`PROBE_GUARD`].
        fn guard_changed(&mut self) -> u32 {
            let (off, size, g) = (self.off / 4, self.size / 4, GUARD / 4);
            let all = self.all();
            all[off - g..off].iter().chain(&all[off + size..off + size + g]).filter(|&&w| w != PROBE_GUARD).count()
                as u32
        }

        /// Hand the CPU's writes to the GPU.
        fn flush(&self) {
            // SAFETY: the allocation's own mapping and size.
            unsafe { cache_flush_raw(self.mem.cpu, self.mem.size) };
        }
    }

    impl Drop for Obj {
        fn drop(&mut self) {
            osal::gpu_free(&mut self.mem);
        }
    }

    /// Submit the writes `build` makes as one kick of `cmd` and wait for it:
    /// the OSAL submit and fence wait every back-end kick takes
    /// (`frame_submit`, `wait_fence`), with the wait bounded. `Ok(true)` when
    /// the fence signalled, `Ok(false)` on a timeout or a wait error.
    fn kick(cmd: &mut Obj, build: impl FnOnce(ProbeSink)) -> Result<bool, i32> {
        let (Some(submit), Some(wait)) = (osal::ops().and_then(|o| o.submit), osal::ops().and_then(|o| o.wait)) else {
            return Err(PROBE_ERR_OSAL);
        };
        let cap = CMD_WORDS - GC_KICK_TAIL_BYTES as usize / 4;
        let words = cmd.words();
        let mut n = 0usize;
        build(&mut |reg, value| {
            /* one register a LOAD_STATE: header and value, 64 bits, so the
             * stream is always an even number of words */
            assert!(n + 2 <= cap, "the probe's stream outgrew its buffer");
            words[n] = load_state(reg, 1, false);
            words[n + 1] = value;
            n += 2;
        });
        cmd.flush();
        let kick = GcKick { magic: GC_KICK_MAGIC, stream_bytes: (n * 4) as u32, stream_gpu: cmd.gpu() as u64 };
        // SAFETY: `kick` is a live GcKick of the size passed.
        let fence = unsafe { submit(&kick as *const GcKick as *const c_void, core::mem::size_of::<GcKick>()) };
        if fence < 0 {
            return Err(PROBE_ERR_SUBMIT);
        }
        // SAFETY: a plain wait on the fence the OSAL just returned.
        Ok(unsafe { wait(fence, WAIT_US) } == 0)
    }

    /// Case 246's arm `arm` (gc-ts-probe.md "Part (iii)"): K0 to K2, and K3
    /// when K2 read `YES`, each one kick of the probe's own, waited on with a
    /// 2 s bound; no kick after a timeout. `progress(step)` is called before
    /// each step (1 set-up done, 2 K0, 3 K1, 4 K2, 5 K3, 6 the readback), so
    /// that the case can write `gc_result_progress(0x200 | arm << 4 | step)`
    /// before the kick that may hang. Fills `*out` (`struct tsp_readback`,
    /// `out_bytes` its size) and returns the [`Outcome`] code, or a
    /// `PROBE_ERR_*` for a step that could not run (`*out` untouched). Waits
    /// for the back end's own kicks first and touches none of its state.
    ///
    /// # Safety
    /// `out` is null or points at `out_bytes` writable bytes; `progress` is
    /// null or a function safe to call with any `u32`.
    #[no_mangle]
    pub unsafe extern "C" fn gc_debug_ts_probe(
        arm: u32,
        out: *mut TspReadback,
        out_bytes: u32,
        progress: Option<unsafe extern "C" fn(step: u32)>,
    ) -> i32 {
        if out.is_null() || out_bytes as usize != core::mem::size_of::<TspReadback>() {
            return PROBE_ERR_ARGS;
        }
        let Some(a) = probe_arm(arm) else { return PROBE_ERR_ARM };
        let step = |s: u32| {
            if let Some(f) = progress {
                // SAFETY: the caller's contract.
                unsafe { f(s) }
            }
        };
        match run(arm, &a, &step) {
            Ok((outcome, rb)) => {
                // SAFETY: non-null and of the struct's size (checked above).
                unsafe { out.write_unaligned(rb) };
                outcome as i32
            }
            Err(e) => e,
        }
    }

    /// Lengths of the reads that happened (0: the step did not run).
    #[derive(Clone, Copy, Default)]
    struct Lens {
        k0: usize,
        k1: usize,
        k2: usize,
        k3: usize,
    }

    /// `scratch` holds `detile(B)`, K0's, K2's and K3's `D`, 4096 dwords each.
    fn readback(sc: &[u32], k1_t: &[u32], l: Lens, base: ArmReads, arm: &ProbeArm) -> (Outcome, TspReadback) {
        let r = ArmReads {
            expected_b: &sc[..N],
            k0_d: &sc[N..N + l.k0],
            k1_t: &k1_t[..l.k1],
            k2_d: &sc[2 * N..2 * N + l.k2],
            k3_d: &sc[3 * N..3 * N + l.k3],
            ..base
        };
        probe_readback(&r, arm)
    }

    fn run(n: u32, arm: &ProbeArm, step: &dyn Fn(u32)) -> Result<(Outcome, TspReadback), i32> {
        if osal::ops().and_then(|o| o.submit).is_none() || osal::ops().and_then(|o| o.wait).is_none() {
            return Err(PROBE_ERR_OSAL);
        }
        if backend::finish() != 0 {
            return Err(PROBE_ERR_FINISH);
        }
        let heap = backend::heap_data();
        let bytes = PROBE_SURFACE_BYTES as usize;
        let mut s = Obj::new(heap, bytes)?;
        let mut t = Obj::new(heap, PROBE_TS_BYTES as usize)?;
        let mut d = Obj::new(heap, bytes)?;
        /* CPU-only scratch: detile(B), K0's D, K2's D, K3's D */
        let mut scratch = Obj::new(heap, 4 * bytes)?;
        let mut cmd = Obj::new(HEAP_COMMAND, CMD_WORDS * 4)?;
        let a = ProbeAddrs { s: s.gpu(), t: t.gpu(), d: d.gpu() };

        for o in [&mut s, &mut t, &mut d] {
            o.all().fill(PROBE_GUARD);
        }
        for (i, w) in s.words().iter_mut().enumerate() {
            *w = probe_b(i as u32);
        }
        t.words().fill(0);
        d.words().fill(PROBE_DEST_INIT);
        for o in [&s, &t, &d] {
            o.flush();
        }
        let s_changed = |s: &mut Obj| s.words().iter().enumerate().any(|(i, &w)| w != probe_b(i as u32));
        let mut base = ArmReads {
            arm: n,
            steps_done: 0,
            fence_timeouts: 0,
            guard_changed: 0,
            surface_changed: false,
            expected_b: &[],
            k0_d: &[],
            k1_t: &[],
            k2_d: &[],
            k3_d: &[],
        };
        let mut l = Lens::default();
        let mut k1_t = [0u32; PROBE_TS_DWORDS];
        let mut interleaved = false;
        step(1);

        /* K0, the control resolve; K1, the TS fill; K2, the TS-sourced
         * resolve. A timeout ends the arm: no later kick runs. */
        let mut timed_out = false;
        step(2);
        if kick(&mut cmd, |w| probe_k0_into(w, arm, &a))? {
            base.steps_done += 1;
            l.k0 = N;
            scratch.words()[N..2 * N].copy_from_slice(d.words());
            /* arm 3: whichever super-tile order K0 matches is detile(B) */
            let k0 = &scratch.words()[N..2 * N];
            let fits = |il: bool| (0..N as u32).all(|i| k0[i as usize] == probe_expected_b(probe_layout(arm, il), i));
            interleaved = arm.super_tiled && !fits(false) && fits(true);
            step(3);
            if kick(&mut cmd, |w| probe_k1_into(w, &a))? {
                base.steps_done += 1;
                l.k1 = PROBE_TS_DWORDS;
                k1_t.copy_from_slice(&t.words()[..PROBE_TS_DWORDS]);
                base.surface_changed |= s_changed(&mut s);
                d.words().fill(PROBE_DEST_INIT);
                d.flush();
                step(4);
                if kick(&mut cmd, |w| probe_k2_into(w, arm, &a))? {
                    base.steps_done += 1;
                    l.k2 = N;
                    scratch.words()[2 * N..3 * N].copy_from_slice(d.words());
                    base.surface_changed |= s_changed(&mut s);
                } else {
                    timed_out = true;
                }
            } else {
                timed_out = true;
            }
        } else {
            timed_out = true;
        }
        base.fence_timeouts = timed_out as u32;
        let layout = probe_layout(arm, interleaved);
        for (i, w) in scratch.words()[..N].iter_mut().enumerate() {
            *w = probe_expected_b(layout, i as u32);
        }
        base.guard_changed = s.guard_changed() + t.guard_changed() + d.guard_changed();

        /* K3, the pattern resolve, only after a YES: T as probe_k3_ts, D
         * reset, K2's stream again */
        if readback(scratch.words(), &k1_t, l, base, arm).0 == Outcome::Yes {
            let ts = probe_k3_ts();
            for (w, b) in t.words().iter_mut().zip(ts.as_chunks::<4>().0) {
                *w = u32::from_le_bytes(*b);
            }
            t.flush();
            d.words().fill(PROBE_DEST_INIT);
            d.flush();
            step(5);
            if kick(&mut cmd, |w| probe_k2_into(w, arm, &a))? {
                base.steps_done += 1;
                l.k3 = N;
                scratch.words()[3 * N..4 * N].copy_from_slice(d.words());
                base.guard_changed = s.guard_changed() + t.guard_changed() + d.guard_changed();
            } else {
                base.fence_timeouts += 1;
            }
        }
        step(6);
        Ok(readback(scratch.words(), &k1_t, l, base, arm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vivante::layout::{self, Layout, SuperOrder};
    use std::vec;
    use std::vec::Vec;

    const ARMS: u32 = 4;

    /// `tsp_classify` of gl_test_probe_decode.inc, transcribed.
    fn tsp_classify(r: &TspReadback) -> u32 {
        if r.timeouts != 0 {
            0xf
        } else if r.guard_changed != 0 || r.s_changed != 0 {
            1
        } else if r.k0_ok == 0 {
            2
        } else if r.t_first_bad != TSP_NOT_FOUND {
            3
        } else if r.n_clear == PROBE_DWORDS as u32 {
            4
        } else if r.n_detile == PROBE_DWORDS as u32 {
            5
        } else if r.n_dead == PROBE_DWORDS as u32 {
            6
        } else {
            7
        }
    }

    fn detile(arm: &ProbeArm, interleaved: bool) -> Vec<u32> {
        (0..PROBE_DWORDS as u32).map(|i| probe_expected_b(probe_layout(arm, interleaved), i)).collect()
    }

    #[test]
    fn the_record_is_the_c_struct() {
        assert_eq!(core::mem::size_of::<TspReadback>(), 19 * 4, "struct tsp_readback: 19 uint32_t");
        assert_eq!(core::mem::align_of::<TspReadback>(), 4);
    }

    #[test]
    fn expected_b_is_layout_rs() {
        for n in 0..ARMS {
            let arm = probe_arm(n).unwrap();
            for (il, order) in [(false, SuperOrder::Drm), (true, SuperOrder::Interleaved)] {
                let l = if arm.super_tiled { Layout::SuperTiled } else { Layout::Tiled };
                for i in 0..PROBE_DWORDS as u32 {
                    let (x, y) = (i % 64, i / 64);
                    let off = layout::offset_in(l, order, x, y, 4, 256) / 4;
                    assert_eq!(probe_expected_b(probe_layout(&arm, il), i), probe_b(off as u32), "arm {n} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn row0_without_allocation_is_k3_expected_clear() {
        let ts = probe_k3_ts();
        for l in [K3Layout::Tiled, K3Layout::SuperTiledDrm, K3Layout::SuperTiledInterleaved] {
            for mode in [TS_MODE_128B, TS_MODE_256B] {
                for low in [true, false] {
                    let clear = k3_expected_clear(l, PROBE_SIDE, PROBE_SIDE, mode, &ts, low);
                    assert_eq!(
                        k3_expected_row0(l, mode, &ts, low),
                        k3_row0_map(&clear, PROBE_SIDE),
                        "{l:?} {mode} {low}"
                    );
                }
            }
        }
        /* the note's maps: 0x55 / 0xaa in 128B, 0x33 / 0xcc in 256B */
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_128B, &ts, true), 0x55);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_128B, &ts, false), 0xaa);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_256B, &ts, true), 0x33);
        assert_eq!(k3_expected_row0(K3Layout::Tiled, TS_MODE_256B, &ts, false), 0xcc);
        let a0 = probe_arm(0).unwrap();
        assert_eq!(k3_nibble_order(0x55, &a0), K3_LOW_NIBBLE_FIRST);
        assert_eq!(k3_nibble_order(0xaa, &a0), 0);
        assert_eq!(k3_nibble_order(0x0f, &a0), K3_NO_MATCH);
    }

    #[test]
    fn crc_is_zlibs() {
        /* zlib.crc32(b"\x01\x00\x00\x00\x02\x00\x00\x00") */
        assert_eq!(probe_crc32(&[1, 2]), 0x0381_177c);
        assert_eq!(probe_crc32(&[]), 0);
    }

    #[test]
    fn readback_agrees_with_the_c_classification() {
        let arm = probe_arm(0).unwrap();
        let b = detile(&arm, false);
        let fill = vec![PROBE_FILL; PROBE_TS_DWORDS];
        let mut bad_fill = fill.clone();
        bad_fill[5] = 0;
        let c = vec![PROBE_CLEAR; PROBE_DWORDS];
        let dead = vec![PROBE_DEST_INIT; PROBE_DWORDS];
        let mut mixed = c.clone();
        mixed[7] = 0x1234_5678;
        let base = ArmReads {
            arm: 0,
            steps_done: 3,
            fence_timeouts: 0,
            guard_changed: 0,
            surface_changed: false,
            expected_b: &b,
            k0_d: &b,
            k1_t: &fill,
            k2_d: &c,
            k3_d: &[],
        };
        let cases: [(ArmReads, Outcome); 8] = [
            (base, Outcome::Yes),
            (ArmReads { k2_d: &b, ..base }, Outcome::Ignored),
            (ArmReads { k2_d: &dead, ..base }, Outcome::NoWrite),
            (ArmReads { k2_d: &mixed, ..base }, Outcome::Mixed),
            (ArmReads { k1_t: &bad_fill, ..base }, Outcome::FillMiss),
            (ArmReads { k0_d: &dead, ..base }, Outcome::NoControl),
            (ArmReads { guard_changed: 2, ..base }, Outcome::Guard),
            (ArmReads { fence_timeouts: 1, steps_done: 1, k1_t: &[], k2_d: &[], ..base }, Outcome::Hang),
        ];
        for (r, want) in cases {
            let (o, rb) = probe_readback(&r, &arm);
            assert_eq!(o, want);
            assert_eq!(tsp_classify(&rb), o as u32, "{want:?}: the C twin classifies the record alike");
        }
        let (_, rb) = probe_readback(&ArmReads { k2_d: &mixed, ..base }, &arm);
        assert_eq!((rb.first_idx, rb.first_val, rb.n_clear), (7, 0x1234_5678, 4095));
        assert_eq!(rb.crc, probe_crc32(&mixed));
        let (_, rb) = probe_readback(&ArmReads { k1_t: &bad_fill, ..base }, &arm);
        assert_eq!(rb.t_first_bad, 5);
    }

    #[test]
    fn a_step_fits_the_hooks_buffer() {
        let a = ProbeAddrs { s: 0x1000, t: 0x2000, d: 0x3000 };
        for n in 0..ARMS {
            let arm = probe_arm(n).unwrap();
            for w in [probe_k0(&arm, &a), probe_k1(&a), probe_k2(&arm, &a)] {
                assert!(2 * w.len() + 2 <= 256, "arm {n}: {} writes", w.len());
            }
        }
    }

    /// The hook on a host OSAL whose GPU is `rs_interp` with Mesa's TS
    /// semantics: the board's code path, the design's answers.
    #[cfg(feature = "test-knobs")]
    mod hook_tests {
        use super::*;
        use crate::osal::{OsalGpuMem, OsalOps};
        use crate::testutil::TestGl;
        use crate::vivante::kick::GcKick;
        use crate::vivante::regs::{load_state, GC_OP_LOAD_STATE};
        use crate::vivante::rs_interp::{Memory, RsInterp};
        use core::ffi::c_void;
        use std::boxed::Box;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Gpu {
            allocs: Vec<(u64, Box<[u8]>)>,
            next: u64,
            rs: RsInterp,
            /// Kicks submitted, and each one's decoded writes.
            streams: Vec<Vec<(u32, u32)>>,
            /// The kick (counted from 1) whose fence never signals.
            hang_at: Option<usize>,
            /// A header that is not one LOAD_STATE of one register.
            odd_words: u32,
        }

        static GPU: Mutex<Option<Gpu>> = Mutex::new(None);
        static STEPS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

        fn with<R>(f: impl FnOnce(&mut Gpu) -> R) -> R {
            f(GPU.lock().unwrap_or_else(|e| e.into_inner()).as_mut().expect("the rig's GPU"))
        }

        unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
            with(|g| {
                let mut b = vec![0x5au8; size].into_boxed_slice();
                let gpu = g.next;
                g.next += (size as u64).next_multiple_of(0x1000) + 0x1000;
                let cpu = b.as_mut_ptr() as *mut c_void;
                g.allocs.push((gpu, b));
                // SAFETY: the caller's out-parameter.
                unsafe { *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: core::ptr::null_mut() } };
                0
            })
        }

        unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
            // SAFETY: the caller's descriptor.
            let m = unsafe { &mut *mem };
            with(|g| g.allocs.retain(|(gpu, _)| *gpu != m.gpu || m.cpu.is_null()));
            *m = OsalGpuMem::ZERO;
            0
        }

        unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}

        unsafe extern "C" fn m_submit(desc: *const c_void, len: usize) -> i64 {
            assert_eq!(len, core::mem::size_of::<GcKick>());
            // SAFETY: the back end's descriptor of that size.
            let k = unsafe { *(desc as *const GcKick) };
            with(|g| {
                assert_eq!(k.stream_bytes % 8, 0, "an even number of words");
                let (base, b) = g
                    .allocs
                    .iter()
                    .find(|(gpu, b)| k.stream_gpu >= *gpu && k.stream_gpu < gpu + b.len() as u64)
                    .expect("the stream is an allocation");
                let o = (k.stream_gpu - base) as usize;
                let words: Vec<u32> = b[o..o + k.stream_bytes as usize]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|w| u32::from_le_bytes(*w))
                    .collect();
                let mut writes = Vec::new();
                for p in words.as_chunks::<2>().0 {
                    if p[0] & 0xf800_0000 == GC_OP_LOAD_STATE && (p[0] >> 16) & 0x3ff == 1 {
                        writes.push(((p[0] & 0xffff) * 4, p[1]));
                    } else {
                        g.odd_words += 1;
                    }
                }
                g.streams.push(writes.clone());
                if g.hang_at != Some(g.streams.len()) {
                    let mut mem = Memory::new();
                    for (gpu, b) in &g.allocs {
                        mem.map(*gpu as u32, b.to_vec());
                    }
                    g.rs.run(&writes, &mut mem);
                    for (gpu, b) in g.allocs.iter_mut() {
                        b.copy_from_slice(mem.region(*gpu as u32));
                    }
                }
                g.streams.len() as i64
            })
        }

        unsafe extern "C" fn m_wait(fence: i64, timeout_us: i64) -> i32 {
            assert_eq!(timeout_us, 2_000_000, "the probe's wait is bounded");
            with(|g| (g.hang_at == Some(fence as usize)) as i32)
        }

        unsafe extern "C" fn m_completed() -> i64 {
            with(|g| g.streams.len() as i64)
        }

        unsafe extern "C" fn m_step(s: u32) {
            STEPS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
        }

        const OPS: OsalOps = OsalOps {
            init: None,
            shutdown: None,
            gpu_alloc: Some(m_alloc),
            gpu_free: Some(m_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: None,
            gpu_read: None,
            cache_flush: Some(m_cache),
            cache_invd: Some(m_cache),
            submit: Some(m_submit),
            wait: Some(m_wait),
            completed: Some(m_completed),
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
        static GPU_OSAL: OsalOps = OPS;
        static NO_SUBMIT_OSAL: OsalOps = OsalOps { submit: None, ..OPS };

        /// The serialising guard and [`GPU_OSAL`] with a fresh GPU.
        struct Rig {
            prev: *const OsalOps,
            _gl: TestGl,
        }

        impl Rig {
            fn new(ts_source: bool, hang_at: Option<usize>) -> Rig {
                let gl = TestGl::new(4, 4);
                let mut rs = RsInterp::new();
                rs.ts_source = ts_source;
                *GPU.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Gpu { next: 0x1000_0800, rs, hang_at, ..Gpu::default() });
                STEPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
                // SAFETY: tests are serialised by the TestGl guard.
                let prev = unsafe {
                    let prev = crate::osal::g_osal;
                    crate::osal::osal_set_ops(&GPU_OSAL);
                    prev
                };
                Rig { prev, _gl: gl }
            }

            fn steps(&self) -> Vec<u32> {
                STEPS.lock().unwrap_or_else(|e| e.into_inner()).clone()
            }
        }

        impl Drop for Rig {
            fn drop(&mut self) {
                // SAFETY: as in `new`.
                unsafe { crate::osal::osal_set_ops(self.prev) };
                *GPU.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }

        fn probe(arm: u32) -> (i32, TspReadback) {
            let mut rb = TspReadback::default();
            // SAFETY: a TspReadback of the size passed; `m_step` takes any u32.
            let rc =
                unsafe { gc_debug_ts_probe(arm, &mut rb, core::mem::size_of::<TspReadback>() as u32, Some(m_step)) };
            (rc, rb)
        }

        #[test]
        fn every_arm_reads_yes_on_mesas_ts_semantics() {
            for n in 0..ARMS {
                let r = Rig::new(true, None);
                let arm = probe_arm(n).unwrap();
                let (rc, rb) = probe(n);
                assert_eq!(rc, Outcome::Yes as i32, "arm {n}: {rb:?}");
                assert_eq!(tsp_classify(&rb), rc as u32, "arm {n}: the C twin agrees");
                assert_eq!(
                    (rb.arm, rb.steps_done, rb.timeouts, rb.guard_changed, rb.s_changed, rb.k0_ok, rb.t_first_bad),
                    (n, 4, 0, 0, 0, 1, TSP_NOT_FOUND),
                    "arm {n}"
                );
                assert_eq!((rb.n_clear, rb.n_detile, rb.n_dead, rb.first_idx), (4096, 0, 0, TSP_NOT_FOUND), "arm {n}");
                assert_eq!(rb.crc, probe_crc32(&[PROBE_CLEAR; PROBE_DWORDS]));
                let want_map = k3_expected_row0(probe_layout(&arm, false), arm.ts_mode, &probe_k3_ts(), true);
                assert_eq!(
                    (rb.k3_present, rb.k3_map, rb.k3_nibble_order),
                    (1, want_map, K3_LOW_NIBBLE_FIRST),
                    "arm {n}"
                );
                assert_eq!(rb.k3_n_clear + rb.k3_n_detile, 4096, "arm {n}: K3 is C or B everywhere");
                assert_eq!(r.steps(), [1, 2, 3, 4, 5, 6], "arm {n}");

                /* the four kicks are K0, K1, K2 and K2 again, one LOAD_STATE a
                 * register, at the objects' addresses */
                let (streams, odd, live) = with(|g| (g.streams.clone(), g.odd_words, g.allocs.len()));
                assert_eq!((streams.len(), odd, live), (4, 0, 0), "arm {n}: four kicks, nothing left allocated");
                let reg = |s: &[(u32, u32)], r: u32| s.iter().rev().find(|w| w.0 == r).unwrap().1;
                let a = ProbeAddrs {
                    s: reg(&streams[2], TS_COLOR_SURFACE_BASE),
                    t: reg(&streams[2], TS_COLOR_STATUS_BASE),
                    d: reg(&streams[2], RS_PIPE_DEST_ADDR0),
                };
                assert!([a.s, a.t, a.d].iter().all(|x| x % 0x1000 == 0), "{a:?}");
                assert_eq!(streams[0], probe_k0(&arm, &a));
                assert_eq!(streams[1], probe_k1(&a));
                assert_eq!(streams[2], probe_k2(&arm, &a));
                assert_eq!(streams[3], probe_k3(&arm, &a));
                assert_eq!(load_state(RS_CONFIG, 1, false), 0x0801_0581, "the note's hdr()");
            }
        }

        #[test]
        fn a_ts_the_rs_ignores_reads_ignored_and_runs_no_k3() {
            let r = Rig::new(false, None);
            let (rc, rb) = probe(0);
            assert_eq!(rc, Outcome::Ignored as i32);
            assert_eq!(tsp_classify(&rb), rc as u32);
            assert_eq!((rb.steps_done, rb.n_detile, rb.n_clear, rb.k3_present), (3, 4096, 0, 0));
            assert_eq!(r.steps(), [1, 2, 3, 4, 6]);
            assert_eq!(with(|g| g.streams.len()), 3);
        }

        #[test]
        fn a_hang_stops_the_arm() {
            let r = Rig::new(true, Some(2));
            let (rc, rb) = probe(1);
            assert_eq!(rc, 0xf, "HANG");
            assert_eq!(tsp_classify(&rb), 0xf);
            assert_eq!((rb.steps_done, rb.timeouts, rb.k0_ok, rb.k3_present), (1, 1, 1, 0));
            assert_eq!(with(|g| g.streams.len()), 2, "no kick after the timeout");
            assert_eq!(r.steps(), [1, 2, 3, 6]);
        }

        #[test]
        fn the_hook_refuses_what_it_cannot_run() {
            let _r = Rig::new(true, None);
            let mut rb = TspReadback::default();
            let size = core::mem::size_of::<TspReadback>() as u32;
            // SAFETY: null and short records are refused before any write.
            unsafe {
                assert_eq!(gc_debug_ts_probe(0, core::ptr::null_mut(), size, None), PROBE_ERR_ARGS);
                assert_eq!(gc_debug_ts_probe(0, &mut rb, size - 4, None), PROBE_ERR_ARGS);
                assert_eq!(gc_debug_ts_probe(4, &mut rb, size, None), PROBE_ERR_ARM);
                crate::osal::osal_set_ops(&NO_SUBMIT_OSAL);
                assert_eq!(gc_debug_ts_probe(0, &mut rb, size, None), PROBE_ERR_OSAL);
                crate::osal::osal_set_ops(&GPU_OSAL);
            }
            assert_eq!(rb, TspReadback::default(), "a refusal writes nothing");
            assert_eq!(with(|g| g.streams.len()), 0);
        }
    }
}
