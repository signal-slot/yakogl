// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 render-target config and `Clear Render Targets` values per GL
//! colour format (roadmap P9 item 5 and item 4; the Pi 5's V3D 7.1.10).
//!
//! Pure functions over plain integers; no `crate::` import, so a test
//! includes this file with `#[path]` (tests/v3d71_rt.rs). Nothing hooks it
//! into `src/v3d/mod.rs` yet: the 7.1 table (backend71, cases71) does that.
//!
//! What moved at 7.1 (docs/notes/v3d71-gl-table.md section 2, items 4 and 5;
//! docs/notes/v3d71-clear-frame.md section 1.2):
//!
//! * 4.2's `CLEAR_COLORS_PART1..3` and the one `COLOR` packet (bpp, type and
//!   clamp of all four targets) are gone. Each colour target has its own
//!   `Tile Rendering Mode Cfg (Render Target Part1)` (sub-id 2): the clear
//!   colour's low 32 bits, internal type and clamping (the 7.1 enum
//!   `Render Target Type Clamp`, [`TypeClamp`]), internal bpp, row stride in
//!   512-bit units minus one, base address in the tile buffer, target
//!   number (3 bits: up to 8 targets). `Part2` (sub-id 3, 40 bits) and
//!   `Part3` (sub-id 4, 56 bits) carry the rest of the clear colour at 64
//!   and 128 bpp ([`RtClear71`]).
//! * The tile list clears the tile buffer with the one-byte `Clear Render
//!   Targets` packet 25 ([`CLEAR_RT_CODE`]); the Store's clear bit is not
//!   used.
//! * 8 targets, 7680 px ([`MAX_RENDER_TARGETS`], [`MAX_SIZE`]).
//!
//! Answers come from Mesa 26.2.1: the internal type and bpp of an output
//! image format are `v3dX(get_internal_type_bpp_for_output_format)`
//! (v3dx_format_table.c:220-341, no version branch), the 7.1 clamp value is
//! `clamp_for_format_and_type` (v3dx_rcl.c, V3D_VERSION >= 71; the same rows
//! as driver/src/v3d/v3d_job71.c `rt71_internal`), the clear packing is
//! `v3d_tlb_clear` (v3dx_draw.c:1706-1769) as `src/v3d/state.rs`
//! `clear_words` packs it for 4.2: the clear colour is written in the tile
//! buffer's own format and Mesa copies `4 << bpp` bytes into a zeroed job,
//! so the words past the tile pixel are 0.
//!
//! A GL format the 7.1 table cannot render gets a [`Refusal71`] variant,
//! never a default row.

/// Render targets a 7.1 job has (`V3D_MAX_RENDER_TARGETS(71)`,
/// v3d_limits.h:44; docs/notes/v3d71-gl-table.md section 2 item 4). The same
/// number as src/v3d/limits71.rs; duplicated here to keep this file
/// self-contained.
pub const MAX_RENDER_TARGETS: u32 = 8;
/// The largest framebuffer edge in pixels (`V3D_MAX_FRAMEBUFFER_SIZE(71)`,
/// v3d_limits.h:63).
pub const MAX_SIZE: u32 = 7680;

/// `Clear Render Targets`, packet 25, one byte, no fields (v3d_packet.xml
/// :485, `min_ver="71"`).
pub const CLEAR_RT_CODE: u8 = 25;
/// `Tile Rendering Mode Cfg` (the 7.1 render-target parts), packet 121.
pub const TILE_RENDERING_MODE_CFG_CODE: u8 = 121;
/// The sub-ids of the three parts.
pub const PART1_SUB_ID: u64 = 2;
pub const PART2_SUB_ID: u64 = 3;
pub const PART3_SUB_ID: u64 = 4;

/// The fields of each part as v3d_packet.xml gives them at 7.1:
/// `(name, start bit, size in bits)`, in the packed 64-bit payload (the
/// byte after the packet code is the payload's low byte). tests/v3d71_rt.rs
/// compares them with the xml's text.
pub const PART1_FIELDS: &[(&str, u32, u32)] = &[
    ("Clear Color low bits", 32, 32),
    ("Internal Type and Clamping", 27, 5),
    ("Internal BPP", 25, 2),
    ("Stride", 18, 7),
    ("Base Address", 7, 11),
    ("Render Target number", 3, 3),
    ("sub-id", 0, 3),
];
pub const PART2_FIELDS: &[(&str, u32, u32)] =
    &[("Clear Color mid bits", 24, 40), ("Render Target number", 3, 3), ("sub-id", 0, 3)];
pub const PART3_FIELDS: &[(&str, u32, u32)] =
    &[("Clear Color top bits", 8, 56), ("Render Target number", 3, 3), ("sub-id", 0, 3)];

/// `Render Target Type Clamp` (v3d_packet.xml:154-175), the values this
/// table uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TypeClamp {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    Ui8 = 4,
    Ui16 = 5,
    Ui32 = 6,
    /// type 8, no clamping
    Type8 = 8,
    F16 = 9,
    F32 = 10,
    I8Clamped = 16,
    I16Clamped = 17,
    I32Clamped = 18,
    Ui8Clamped = 20,
    Ui16Clamped = 21,
    Ui32Clamped = 22,
    F16ClampNorm = 24,
}

/// `Internal BPP` (v3d_packet.xml:135-139).
pub const BPP_32: u32 = 0;
pub const BPP_64: u32 = 1;
pub const BPP_128: u32 = 2;

/// How the clear colour is written into the tile buffer's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearClass {
    /// type 8: one R8G8B8A8_UNORM word (RGBA8, RGB8, RG8, R8, RGB565, RGBA4,
    /// RGB5_A1: all through the 8-bit tile buffer).
    Unorm8,
    /// type 16F, a fixed-point surface (sRGB8_ALPHA8, RGB10_A2): halves, the
    /// colour clamped to [0, 1] first as src/v3d/state.rs does.
    Fixed16F,
    /// type 16F, a float surface (R16F, RG16F, RGBA16F, R11F_G11F_B10F).
    Float16,
    /// type 32F: the floats' bits.
    Float32,
    /// 8I / 8UI: the low byte of each raw word, four in one word.
    Int8,
    /// 16I / 16UI: the low 16 bits of each raw word, two a word.
    Int16,
    /// 32I / 32UI: the raw words.
    Int32,
}

/// One colour format's render-target answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rt71 {
    pub type_clamp: TypeClamp,
    /// `BPP_32`, `BPP_64` or `BPP_128`.
    pub bpp: u32,
    pub class: ClearClass,
    /// True for a pure integer format: its clear colour is four raw integers.
    pub integer: bool,
}

/// Why a render target or a clear is refused. Each variant names its cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal71 {
    /// a colour format Mesa's table has no render-target row for: SNORM,
    /// RGB9_E5, sRGB8 without alpha, RGB8I and the other three-channel
    /// integers, 16-bit normalised
    FormatNotRenderable,
    /// a depth or stencil format given as a colour target
    NotAColourFormat,
    /// a GL enum this table does not know
    UnknownFormat,
    /// more than [`MAX_RENDER_TARGETS`]
    TooManyTargets,
    /// a width or height over [`MAX_SIZE`]
    TargetTooLarge,
    /// a width or height of 0
    EmptyTarget,
    /// a float colour for an integer target, or an integer one for a float
    /// target (glClearBufferfv on an integer buffer is a GL error)
    ClearKindMismatch,
    /// a render target number of 8 or more
    TargetNumberOutOfRange,
}

impl Refusal71 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Refusal71::FormatNotRenderable => "v3d 7.1: this colour format has no render-target row in Mesa's table",
            Refusal71::NotAColourFormat => "v3d 7.1: a depth or stencil format is not a colour target",
            Refusal71::UnknownFormat => "v3d 7.1: an unknown colour format",
            Refusal71::TooManyTargets => "v3d 7.1: more than 8 render targets",
            Refusal71::TargetTooLarge => "v3d 7.1: a render target larger than 7680 px",
            Refusal71::EmptyTarget => "v3d 7.1: an empty render target",
            Refusal71::ClearKindMismatch => "v3d 7.1: the clear colour's kind does not match the target's",
            Refusal71::TargetNumberOutOfRange => "v3d 7.1: a render target number above 7",
        }
    }
}

/// The clear colour glClear / glClearBuffer{f,i,ui}v gives for one target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearColor {
    Float([f32; 4]),
    Int([i32; 4]),
    Uint([u32; 4]),
}

const fn row(type_clamp: TypeClamp, bpp: u32, class: ClearClass, integer: bool) -> Rt71 {
    Rt71 { type_clamp, bpp, class, integer }
}

/// The 7.1 render-target answers for a GL sized internal format (a GLES 3.0
/// enum as `u32`).
pub const fn rt_format71(gl_internal: u32) -> Result<Rt71, Refusal71> {
    use ClearClass::*;
    use TypeClamp::*;
    Ok(match gl_internal {
        // 8-bit normalised, the tile buffer's type 8 at 32 bpp
        0x8058 /* RGBA8 */ | 0x8051 /* RGB8 */ | 0x822B /* RG8 */ | 0x8229 /* R8 */
        | 0x8D62 /* RGB565 */ | 0x8056 /* RGBA4 */ | 0x8057 /* RGB5_A1 */ => {
            row(Type8, BPP_32, Unorm8, false)
        }
        // sRGB: 16F at 64 bpp with norm clamping, the TLB converts
        0x8C43 /* SRGB8_ALPHA8 */ => row(F16ClampNorm, BPP_64, Fixed16F, false),
        0x8059 /* RGB10_A2 */ => row(F16, BPP_64, Fixed16F, false),
        0x881A /* RGBA16F */ | 0x8C3A /* R11F_G11F_B10F */ | 0x822D /* R16F */ | 0x822F /* RG16F */ => {
            row(F16, BPP_64, Float16, false)
        }
        0x822E /* R32F */ => row(F32, BPP_32, Float32, false),
        0x8230 /* RG32F */ => row(F32, BPP_64, Float32, false),
        0x8814 /* RGBA32F */ => row(F32, BPP_128, Float32, false),
        0x8231 /* R8I */ | 0x8237 /* RG8I */ | 0x8D8E /* RGBA8I */ => row(I8Clamped, BPP_32, Int8, true),
        0x8232 /* R8UI */ | 0x8238 /* RG8UI */ | 0x8D7C /* RGBA8UI */ => row(Ui8Clamped, BPP_32, Int8, true),
        0x8233 /* R16I */ | 0x8239 /* RG16I */ => row(I16Clamped, BPP_32, Int16, true),
        0x8D88 /* RGBA16I */ => row(I16Clamped, BPP_64, Int16, true),
        0x8234 /* R16UI */ | 0x823A /* RG16UI */ => row(Ui16Clamped, BPP_32, Int16, true),
        0x8D76 /* RGBA16UI */ | 0x906F /* RGB10_A2UI */ => row(Ui16Clamped, BPP_64, Int16, true),
        0x8235 /* R32I */ => row(I32Clamped, BPP_32, Int32, true),
        0x823B /* RG32I */ => row(I32Clamped, BPP_64, Int32, true),
        0x8D82 /* RGBA32I */ => row(I32Clamped, BPP_128, Int32, true),
        0x8236 /* R32UI */ => row(Ui32Clamped, BPP_32, Int32, true),
        0x823C /* RG32UI */ => row(Ui32Clamped, BPP_64, Int32, true),
        0x8D70 /* RGBA32UI */ => row(Ui32Clamped, BPP_128, Int32, true),
        // depth and stencil
        0x81A5 /* DEPTH_COMPONENT16 */ | 0x81A6 /* DEPTH_COMPONENT24 */ | 0x81A7 /* DEPTH_COMPONENT32 */
        | 0x8CAC /* DEPTH_COMPONENT32F */ | 0x88F0 /* DEPTH24_STENCIL8 */ | 0x8CAD /* DEPTH32F_STENCIL8 */
        | 0x8D48 /* STENCIL_INDEX8 */ => return Err(Refusal71::NotAColourFormat),
        // SNORM (the table's rt column is NO), RGB9_E5, SRGB8, RGB8I and
        // friends, 16-bit normalised, RGB16F / RGB32F
        0x8F97 /* RGBA8_SNORM */ | 0x8F96 /* RGB8_SNORM */ | 0x8F95 /* RG8_SNORM */ | 0x8F94 /* R8_SNORM */
        | 0x8C3D /* RGB9_E5 */ | 0x8C41 /* SRGB8 */ | 0x8D8F /* RGB8I */ | 0x8D7D /* RGB8UI */
        | 0x8D89 /* RGB16I */ | 0x8D77 /* RGB16UI */ | 0x8D83 /* RGB32I */ | 0x8D71 /* RGB32UI */
        | 0x881B /* RGB16F */ | 0x8815 /* RGB32F */ | 0x822A /* R16 */ | 0x822C /* RG16 */
        | 0x805B /* RGBA16 */ => return Err(Refusal71::FormatNotRenderable),
        _ => return Err(Refusal71::UnknownFormat),
    })
}

/// At most [`MAX_RENDER_TARGETS`] colour targets.
pub const fn check_targets(n: u32) -> Result<(), Refusal71> {
    if n > MAX_RENDER_TARGETS {
        Err(Refusal71::TooManyTargets)
    } else {
        Ok(())
    }
}

/// A target of 1..=[`MAX_SIZE`] pixels each way.
pub const fn check_size(width: u32, height: u32) -> Result<(), Refusal71> {
    if width == 0 || height == 0 {
        Err(Refusal71::EmptyTarget)
    } else if width > MAX_SIZE || height > MAX_SIZE {
        Err(Refusal71::TargetTooLarge)
    } else {
        Ok(())
    }
}

/// The 32-bit words of a pixel in the tile buffer: 1, 2 or 4.
pub const fn bpp_words(bpp: u32) -> u32 {
    1 << if bpp > 2 { 2 } else { bpp }
}

/// The row stride field before its minus-one, in 512-bit units
/// (v3d_compute_rt_row_row_stride_128_bits; docs/notes/v3d71-clear-frame.md
/// section 1.2): `tile_width * words / 2`. A 64-px tile at 32 bpp is 32, a
/// 32-px one 16; 64 px at 128 bpp is 128, the field's largest.
pub const fn rt_stride(tile_width: u32, bpp: u32) -> u32 {
    tile_width * bpp_words(bpp) / 2
}

/// How far the next target's base moves: `tile_height * stride / 8`
/// (v3dx_rcl.c:871,874; tools/v3d/pi5/gl_mrt8.clif: 0, 64, 128 ... at a
/// 32 x 32 tile).
pub const fn rt_base_step(tile_height: u32, stride: u32) -> u32 {
    tile_height * stride / 8
}

/// The clear colour of one target as the tile buffer's words, and the
/// three packet payload fields they fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtClear71 {
    /// the words, 0 past the pixel's `bpp_words`
    pub words: [u32; 4],
    /// Part1's `Clear Color low bits` (32 bits): word 0
    pub low: u32,
    /// Part2's `Clear Color mid bits` (40 bits): word 1, then the low byte
    /// of word 2; only emitted from 64 bpp
    pub mid: u64,
    /// Part3's `Clear Color top bits` (56 bits): the upper 24 bits of word
    /// 2, then word 3; only emitted at 128 bpp
    pub top: u64,
}

/// `util_pack_color` for R8G8B8A8_UNORM, R in the low byte, rounding half to
/// even (`_mesa_float_to_unorm`); NaN is 0. (src/v3d/state.rs `pack_rgba8`.)
const fn pack_rgba8(c: [f32; 4]) -> u32 {
    let mut out = 0u32;
    let mut i = 0;
    while i < 4 {
        let v = c[i];
        let x = if v.is_nan() || v < 0.0 {
            0.0
        } else if v > 1.0 {
            255.0
        } else {
            v * 255.0
        };
        let fl = (x as u32) as f32;
        let frac = x - fl;
        let r = if frac > 0.5 || (frac == 0.5 && (fl as u32) & 1 == 1) { fl + 1.0 } else { fl };
        out |= (r as u32 & 0xff) << (8 * i);
        i += 1;
    }
    out
}

/// A half-float, nearest even, 65504 the largest, overflow to infinity
/// (src/v3d/state.rs `f32_to_f16`, `_mesa_float_to_half`).
const fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127;
    if e > 15 {
        return sign | 0x7c00;
    }
    let sig = if exp == 0 { man } else { man | 0x80_0000 };
    let shift = if e >= -14 { 13 } else { 13 + (-14 - e) } as u32;
    if shift > 24 {
        return sign;
    }
    let kept = sig >> shift;
    let rest = sig & ((1u32 << shift) - 1);
    let half = 1u32 << (shift - 1);
    let rounded = kept + (rest > half || (rest == half && kept & 1 == 1)) as u32;
    if e >= -14 {
        let field = (((e + 15) as u32) << 10) + (rounded - 0x400);
        sign | (if field > 0x7c00 { 0x7c00 } else { field }) as u16
    } else {
        sign | rounded as u16
    }
}

const fn clamp01(x: f32) -> f32 {
    if x > 0.0 {
        if x < 1.0 {
            x
        } else {
            1.0
        }
    } else {
        0.0
    }
}

/// The clear-colour words and Part1..3 fields of one target (`v3d_tlb_clear`
/// as src/v3d/state.rs `clear_words` and clear_mrt.rs `clear_color_words`
/// pack them). A float colour for an integer target, or an integer one for a
/// float target, is [`Refusal71::ClearKindMismatch`]; a signed and an
/// unsigned integer colour are both raw bits to the tile buffer.
pub fn clear_words71(rt: &Rt71, color: ClearColor) -> Result<RtClear71, Refusal71> {
    let mut w = match (rt.integer, color) {
        (false, ClearColor::Float(c)) => match rt.class {
            ClearClass::Unorm8 => [pack_rgba8(c), 0, 0, 0],
            ClearClass::Fixed16F | ClearClass::Float16 => {
                let c = if rt.class == ClearClass::Fixed16F { c.map(clamp01) } else { c };
                let h = c.map(|x| f32_to_f16(x) as u32);
                [h[0] | h[1] << 16, h[2] | h[3] << 16, 0, 0]
            }
            ClearClass::Float32 => c.map(f32::to_bits),
            _ => return Err(Refusal71::ClearKindMismatch),
        },
        (true, ClearColor::Int(c)) => int_words(rt.class, c.map(|x| x as u32)),
        (true, ClearColor::Uint(c)) => int_words(rt.class, c),
        _ => return Err(Refusal71::ClearKindMismatch),
    };
    let words = bpp_words(rt.bpp) as usize;
    let mut i = words;
    while i < 4 {
        w[i] = 0;
        i += 1;
    }
    Ok(RtClear71 {
        words: w,
        low: w[0],
        mid: w[1] as u64 | ((w[2] & 0xff) as u64) << 32,
        top: (w[2] >> 8) as u64 | (w[3] as u64) << 24,
    })
}

fn int_words(class: ClearClass, u: [u32; 4]) -> [u32; 4] {
    match class {
        ClearClass::Int8 => [(u[0] & 0xff) | (u[1] & 0xff) << 8 | (u[2] & 0xff) << 16 | (u[3] & 0xff) << 24, 0, 0, 0],
        ClearClass::Int16 => [(u[0] & 0xffff) | u[1] << 16, (u[2] & 0xffff) | u[3] << 16, 0, 0],
        _ => u,
    }
}

/// Part1's 8-byte payload as a `u64` (little-endian on the wire).
/// `stride` is the value before the minus-one ([`rt_stride`]); `base` is in
/// 512-bit units. `Err` for a target number of 8 or more, a stride of 0 or
/// over 128, or a base over 11 bits.
pub const fn part1_payload(target: u32, rt: &Rt71, clear_low: u32, stride: u32, base: u32) -> Result<u64, Refusal71> {
    if target >= MAX_RENDER_TARGETS || stride == 0 || stride > 128 || base >= 1 << 11 {
        return Err(Refusal71::TargetNumberOutOfRange);
    }
    Ok((clear_low as u64) << 32
        | (rt.type_clamp as u64) << 27
        | (rt.bpp as u64) << 25
        | ((stride - 1) as u64) << 18
        | (base as u64) << 7
        | (target as u64) << 3
        | PART1_SUB_ID)
}

/// Part2's payload: 40 bits of clear colour at bit 24.
pub const fn part2_payload(target: u32, mid: u64) -> Result<u64, Refusal71> {
    if target >= MAX_RENDER_TARGETS || mid >> 40 != 0 {
        return Err(Refusal71::TargetNumberOutOfRange);
    }
    Ok(mid << 24 | (target as u64) << 3 | PART2_SUB_ID)
}

/// Part3's payload: 56 bits of clear colour at bit 8.
pub const fn part3_payload(target: u32, top: u64) -> Result<u64, Refusal71> {
    if target >= MAX_RENDER_TARGETS || top >> 56 != 0 {
        return Err(Refusal71::TargetNumberOutOfRange);
    }
    Ok(top << 8 | (target as u64) << 3 | PART3_SUB_ID)
}

/// A packet's nine bytes: the code 121 and the payload, low byte first.
pub const fn packet_bytes(payload: u64) -> [u8; 9] {
    let p = payload.to_le_bytes();
    [TILE_RENDERING_MODE_CFG_CODE, p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]
}
