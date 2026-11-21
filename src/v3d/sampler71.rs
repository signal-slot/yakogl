// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 sampler state field values (pure, no `crate::` imports so a test
//! can include it with `#[path]`). From plain GL-side inputs it gives the
//! values `v3d_tex71_pack_sampler` (`driver/src/v3d/v3d_tex71.h`) takes, in
//! a struct whose field names are that header's `struct v3d_tex71_sampler`,
//! and the 24-byte record those values make (SAMPLER_STATE,
//! `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:1709-1734`,
//! one struct for every generation).
//!
//! What differs from 4.2 (`v3d_tex71.h`, Mesa `v3dx_state.c:586-767`): the
//! field layout is 4.2's, but a FOLLOWS border colour is four 32-bit words
//! given as is (the 7.x TMU converts them itself, `:749-760`), where 4.2
//! wants f16 values. Cube maps (`docs/notes/v3d71-cube-seamless.md`): the
//! packet has no cube and no seamless field, a cube look-up (TMUSCM) is
//! seamless at 7.1 by prediction (U8, not measured) and ignores the wrap
//! modes, and nothing can turn that off: a request for non-seamless
//! filtering is refused by name.
//!
//! Decisions (Mesa's sampler variant code is not vendored; these are this
//! module's, not Mesa figures):
//!  - the border colour mode is chosen here: when no wrap mode is
//!    CLAMP_TO_BORDER the mode is 0000 whatever the colour; an all-zero
//!    colour gives 0000; a float (0, 0, 0, 1) gives 0001 and a float
//!    (1, 1, 1, 1) gives 1111; every other colour, and every non-zero
//!    integer colour, is FOLLOWS with the four words as given (an integer
//!    (0, 0, 0, 1) is FOLLOWS because the 0001 shortcut is a float 1.0);
//!  - the words are given in the channel order and clamp the caller's
//!    border variant already has (Mesa's `:646-747`); this module only
//!    carries the 32 bits (f32 bits for a float colour).

/// `V3D_TEX71_WRAP_*` (`v3d_tex71.h`, Mesa's "Wrap Mode" enum).
pub const WRAP_REPEAT: u32 = 0;
pub const WRAP_CLAMP: u32 = 1;
pub const WRAP_MIRROR: u32 = 2;
pub const WRAP_BORDER: u32 = 3;
pub const WRAP_MIRROR_ONCE: u32 = 4;

/// `V3D_TEX71_BORDER_*` ("Border Color Mode" enum).
pub const BORDER_0000: u32 = 0;
pub const BORDER_0001: u32 = 1;
pub const BORDER_1111: u32 = 2;
pub const BORDER_FOLLOWS: u32 = 7;

/// `V3D_TEX71_SAMPLER_BYTES`.
pub const SAMPLER_BYTES: usize = 24;

/// SAMPLER_STATE's fields as (name, start bit, size), the XML's table
/// (`v3d_packet.xml:1709-1734`); the test compares it with the XML text.
pub const FIELDS: [(&str, u32, u32); 19] = [
    ("Border color word 3", 160, 32),
    ("Border color word 2", 128, 32),
    ("Border color word 1", 96, 32),
    ("Border color word 0", 64, 32),
    ("Maximum Anisotropy", 61, 2),
    ("Border Color Mode", 58, 3),
    ("Wrap I Border", 57, 1),
    ("Wrap R", 54, 3),
    ("Wrap T", 51, 3),
    ("Wrap S", 48, 3),
    ("Fixed Bias", 32, 16),
    ("Max Level-of-Detail", 20, 12),
    ("Min Level-of-Detail", 8, 12),
    ("sRGB Disable", 7, 1),
    ("Depth Compare Function", 4, 3),
    ("Anisotropy Enable", 3, 1),
    ("Mip filter Nearest", 2, 1),
    ("Min filter Nearest", 1, 1),
    ("Mag filter Nearest", 0, 1),
];

/// A filter, GL's NEAREST or LINEAR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

/// The mip filter: GL's NEAREST_MIPMAP_*, LINEAR_MIPMAP_*, or none (a
/// minification filter without a mipmap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipFilter {
    None,
    Nearest,
    Linear,
}

/// A wrap mode of GL ES 3.0 plus MIRROR_CLAMP_TO_EDGE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    ClampToEdge,
    MirroredRepeat,
    ClampToBorder,
    MirrorClampToEdge,
}

impl Wrap {
    pub const fn value(self) -> u32 {
        match self {
            Wrap::Repeat => WRAP_REPEAT,
            Wrap::ClampToEdge => WRAP_CLAMP,
            Wrap::MirroredRepeat => WRAP_MIRROR,
            Wrap::ClampToBorder => WRAP_BORDER,
            Wrap::MirrorClampToEdge => WRAP_MIRROR_ONCE,
        }
    }
}

/// The border colour as the format class gives it: four floats (float and
/// normalised formats) or four integers (integer formats).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Border {
    Float([f32; 4]),
    Int([u32; 4]),
}

/// What the module takes, all plain values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    pub min_filter: Filter,
    pub mag_filter: Filter,
    pub mip_filter: MipFilter,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
    pub wrap_r: Wrap,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    /// GL_COMPARE_REF_TO_TEXTURE on.
    pub compare_enabled: bool,
    /// 0..7 in GL order (NEVER, LESS, EQUAL, LEQUAL, GREATER, NOTEQUAL,
    /// GEQUAL, ALWAYS); only read when the compare is on.
    pub compare_func: u32,
    /// As Gallium keeps it: 0 off, otherwise up to 16 taps.
    pub max_anisotropy: u32,
    /// Float or integer by the format's class (the integer-format flag).
    pub border: Border,
    /// The texture is a cube map.
    pub cube: bool,
    /// Seamless filtering wanted (read for a cube only). 7.1 has no control
    /// for it: the TMU is seamless, so `false` on a cube is refused.
    pub seamless: bool,
}

impl Input {
    /// GL's defaults for a 2D texture: nearest, repeat, LOD range
    /// (-1000, 1000), no bias, no compare, no anisotropy, border 0.
    pub const fn default_2d() -> Input {
        Input {
            min_filter: Filter::Nearest,
            mag_filter: Filter::Nearest,
            mip_filter: MipFilter::None,
            wrap_s: Wrap::Repeat,
            wrap_t: Wrap::Repeat,
            wrap_r: Wrap::Repeat,
            min_lod: -1000.0,
            max_lod: 1000.0,
            lod_bias: 0.0,
            compare_enabled: false,
            compare_func: 0,
            max_anisotropy: 0,
            border: Border::Float([0.0; 4]),
            cube: false,
            seamless: true,
        }
    }
}

/// What 7.1 or the C packer cannot take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A cube map sampled non-seamlessly: no 7.1 sampler or texture field
    /// turns the TMU's seamless filtering off (`v3d71-cube-seamless.md`
    /// section 3); the 4.2 emulation keyed on the program is not available.
    CubeNonSeamless,
    /// A compare function above 7.
    CompareFuncOutOfRange,
    /// A NaN level-of-detail limit.
    NanLod,
    /// A NaN LOD bias.
    NanBias,
    /// A bias outside the s8.8 field (-128 <= bias < 128), as
    /// `v3d_tex71_pack_sampler` refuses it.
    BiasOutOfRange,
    /// More than 16 anisotropy taps.
    AnisotropyOutOfRange,
    /// A NaN in a float border colour.
    NanBorder,
}

impl Refusal {
    pub const fn message(self) -> &'static str {
        match self {
            Refusal::CubeNonSeamless => {
                "V3D 7.1: a cube map is always filtered seamlessly, the sampler and texture records have no field to turn it off"
            }
            Refusal::CompareFuncOutOfRange => "V3D 7.1 sampler: depth compare function above 7",
            Refusal::NanLod => "V3D 7.1 sampler: a NaN level-of-detail limit",
            Refusal::NanBias => "V3D 7.1 sampler: a NaN LOD bias",
            Refusal::BiasOutOfRange => "V3D 7.1 sampler: LOD bias outside the s8.8 field (-128 <= bias < 128)",
            Refusal::AnisotropyOutOfRange => "V3D 7.1 sampler: more than 16 anisotropy taps",
            Refusal::NanBorder => "V3D 7.1 sampler: a NaN in the border colour",
        }
    }
}

/// The values `v3d_tex71_pack_sampler` takes: `struct v3d_tex71_sampler`
/// of `v3d_tex71.h`, field for field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampler71 {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub wrap_r: u32,
    pub min_nearest: bool,
    pub mag_nearest: bool,
    pub mip_nearest: bool,
    pub mip_none: bool,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    pub compare_func: u32,
    pub max_anisotropy: u32,
    pub border_color_mode: u32,
    pub border_color: [u32; 4],
}

/// Round half away from zero (`llroundf`, as `util_bitpack_[us]fixed`).
fn round_half_away(v: f32) -> i64 {
    (if v >= 0.0 { v + 0.5 } else { v - 0.5 }) as i64
}

fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// The bias as the s8.8 field's integer, or `None` where the C packer
/// refuses it.
fn bias_fixed(bias: f32) -> Option<i64> {
    if !(-129.0..=129.0).contains(&bias) {
        return None;
    }
    let q = round_half_away(bias * 256.0);
    (-32768..=32767).contains(&q).then_some(q)
}

/// The field values for `i`, or the named refusal. LOD limits and the bias
/// are passed on as given: the clamp to 0..15, the mip-none rule and the
/// rounding are the C packer's ([`pack`] repeats them).
pub fn sampler71(i: &Input) -> Result<Sampler71, Refusal> {
    if i.cube && !i.seamless {
        return Err(Refusal::CubeNonSeamless);
    }
    if i.compare_enabled && i.compare_func > 7 {
        return Err(Refusal::CompareFuncOutOfRange);
    }
    if i.min_lod.is_nan() || i.max_lod.is_nan() {
        return Err(Refusal::NanLod);
    }
    if i.lod_bias.is_nan() {
        return Err(Refusal::NanBias);
    }
    if bias_fixed(i.lod_bias).is_none() {
        return Err(Refusal::BiasOutOfRange);
    }
    if i.max_anisotropy > 16 {
        return Err(Refusal::AnisotropyOutOfRange);
    }

    let uses_border = [i.wrap_s, i.wrap_t, i.wrap_r].contains(&Wrap::ClampToBorder);
    let (mode, words) = match i.border {
        Border::Float(f) => {
            if f.iter().any(|v| v.is_nan()) {
                return Err(Refusal::NanBorder);
            }
            let zero = f.map(|v| v == 0.0);
            if !uses_border || zero == [true; 4] {
                (BORDER_0000, [0; 4])
            } else if zero[..3] == [true; 3] && f[3] == 1.0 {
                (BORDER_0001, [0; 4])
            } else if f == [1.0; 4] {
                (BORDER_1111, [0; 4])
            } else {
                (BORDER_FOLLOWS, f.map(f32::to_bits))
            }
        }
        Border::Int(w) => {
            if !uses_border || w == [0; 4] {
                (BORDER_0000, [0; 4])
            } else {
                (BORDER_FOLLOWS, w)
            }
        }
    };

    Ok(Sampler71 {
        wrap_s: i.wrap_s.value(),
        wrap_t: i.wrap_t.value(),
        wrap_r: i.wrap_r.value(),
        min_nearest: i.min_filter == Filter::Nearest,
        mag_nearest: i.mag_filter == Filter::Nearest,
        mip_nearest: i.mip_filter != MipFilter::Linear,
        mip_none: i.mip_filter == MipFilter::None,
        min_lod: i.min_lod,
        max_lod: i.max_lod,
        lod_bias: i.lod_bias,
        compare_func: if i.compare_enabled { i.compare_func } else { 0 },
        max_anisotropy: i.max_anisotropy,
        border_color_mode: mode,
        border_color: words,
    })
}

/// The 24-byte record `v3d_tex71_pack_sampler` makes from `s`, byte for byte
/// (the same LOD clamps, the same rounding onto the u4.8 and s8.8 grids,
/// the same anisotropy steps); `None` where that function returns false.
pub fn pack(s: &Sampler71) -> Option<[u8; SAMPLER_BYTES]> {
    if s.wrap_s > WRAP_MIRROR_ONCE
        || s.wrap_t > WRAP_MIRROR_ONCE
        || s.wrap_r > WRAP_MIRROR_ONCE
        || ![BORDER_0000, BORDER_0001, BORDER_1111, BORDER_FOLLOWS].contains(&s.border_color_mode)
        || s.compare_func > 7
        || s.min_lod.is_nan()
        || s.max_lod.is_nan()
        || s.lod_bias.is_nan()
    {
        return None;
    }
    if s.border_color_mode != BORDER_FOLLOWS && s.border_color != [0; 4] {
        return None;
    }
    let bias = bias_fixed(s.lod_bias)?;
    let mut min_lod = clampf(s.min_lod, 0.0, 15.0);
    let mut max_lod = clampf(if s.max_lod > s.min_lod { s.max_lod } else { s.min_lod }, 0.0, 15.0);
    if s.mip_none {
        let e = 1.0 / 256.0;
        min_lod = min_lod.min(e);
        max_lod = max_lod.min(e);
    }
    let min_q = round_half_away(min_lod * 256.0) as u64;
    let max_q = round_half_away(max_lod * 256.0) as u64;
    let (aniso_en, aniso) = match s.max_anisotropy {
        0 => (0u64, 0u64),
        1..=2 => (1, 0),
        3..=4 => (1, 1),
        5..=8 => (1, 2),
        _ => (1, 3),
    };
    let lo: u64 = u64::from(s.mag_nearest)
        | u64::from(s.min_nearest) << 1
        | u64::from(s.mip_nearest) << 2
        | aniso_en << 3
        | u64::from(s.compare_func) << 4
        | min_q << 8
        | max_q << 20
        | ((bias as u64) & 0xffff) << 32
        | u64::from(s.wrap_s) << 48
        | u64::from(s.wrap_t) << 51
        | u64::from(s.wrap_r) << 54
        | u64::from(s.border_color_mode) << 58
        | aniso << 61;
    let mut out = [0u8; SAMPLER_BYTES];
    out[..8].copy_from_slice(&lo.to_le_bytes());
    for (k, w) in s.border_color.iter().enumerate() {
        out[8 + 4 * k..12 + 4 * k].copy_from_slice(&w.to_le_bytes());
    }
    Some(out)
}
