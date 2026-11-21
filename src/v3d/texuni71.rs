// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The 7.1 texture uniform words (P10, docs/notes/v3d71-gl-table.md section 3):
//! TMU Config Parameter 0 and 1 (`QUNIFORM_TMU_CONFIG_P0` / `_P1`) and the
//! texture size words (`QUNIFORM_TEXTURE_WIDTH` .. `_ARRAY_SIZE`), the words
//! `uniform71` refuses by name.
//!
//! Pure over plain words, no `crate::` import. The compiler's `data` of a
//! TMU config uniform is `unit << 24 | bits` (`select.rs`, the `Wrtmuc`
//! pushes): for P0 the low four bits are "Return words of texture data", for
//! P1 bit 2 is "Per-pixel mask enable", bit 1 "Unnormalized coordinates" and
//! bit 0 "Output Type 32-bit" (`v3d_packet.xml`, TMU Config Parameter 0 and
//! 1). The words are
//!
//! - P0 = texture shader state address (32-byte aligned) | return words
//! - P1 = sampler state address (8-byte aligned) | flag bits
//!
//! as Mesa's `v3d_uniforms.c` writes them; the address of unit `unit` comes
//! from the caller's [`Unit`] table. A size word's `data` is the unit.
//! The sizes are the base level's width and height, a 3D texture's depth,
//! a 2D array's layers (1 for other kinds); a size not given (0) is refused.

pub const TMU_CONFIG_P0: u32 = 40;
pub const TMU_CONFIG_P1: u32 = 41;
pub const TEXTURE_WIDTH: u32 = 44;
pub const TEXTURE_HEIGHT: u32 = 45;
pub const TEXTURE_DEPTH: u32 = 46;
pub const TEXTURE_ARRAY_SIZE: u32 = 47;

/// Texture state record alignment, bytes (the address field's low 4 bits
/// hold the return words; Mesa allocates the record 32-byte aligned).
pub const TEX_STATE_ALIGN: u32 = 32;
/// Sampler state record alignment, bytes (P1's low 3 bits are flags).
pub const SAMPLER_STATE_ALIGN: u32 = 8;

/// One texture unit: its record addresses and its base level's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub unit: u32,
    pub tex_state: u32,
    pub sampler: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// no [`Unit`] for the unit the uniform names
    NoUnit { unit: u32 },
    /// the texture state address is not 32-byte aligned
    TexStateAlign { addr: u32 },
    /// the sampler state address is not 8-byte aligned
    SamplerAlign { addr: u32 },
    /// `data` has bits set outside the unit (24..32) and the config field
    DataBits { contents: u32, data: u32 },
    /// the size word's value is 0 (not given)
    SizeNotGiven { contents: u32, unit: u32 },
    /// not a texture uniform
    NotTexture { contents: u32 },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Refusal::NoUnit { unit } => write!(f, "no texture unit {unit} bound for the uniform"),
            Refusal::TexStateAlign { addr } => write!(f, "texture state address {addr:#x} is not 32-byte aligned"),
            Refusal::SamplerAlign { addr } => write!(f, "sampler state address {addr:#x} is not 8-byte aligned"),
            Refusal::DataBits { contents, data } => {
                write!(f, "uniform {contents} data {data:#x} has bits outside the unit and the config field")
            }
            Refusal::SizeNotGiven { contents, unit } => {
                write!(f, "texture size uniform {contents} of unit {unit} has no size given")
            }
            Refusal::NotTexture { contents } => write!(f, "uniform contents {contents:#x} is not a texture uniform"),
        }
    }
}

fn find(units: &[Unit], unit: u32) -> Result<&Unit, Refusal> {
    units.iter().find(|u| u.unit == unit).ok_or(Refusal::NoUnit { unit })
}

/// TMU Config Parameter 0 of the compiler's `data`.
pub fn tmu_p0(data: u32, units: &[Unit]) -> Result<u32, Refusal> {
    if data & 0x00ff_fff0 != 0 {
        return Err(Refusal::DataBits { contents: TMU_CONFIG_P0, data });
    }
    let u = find(units, data >> 24)?;
    if u.tex_state % TEX_STATE_ALIGN != 0 {
        return Err(Refusal::TexStateAlign { addr: u.tex_state });
    }
    Ok(u.tex_state | (data & 0xf))
}

/// TMU Config Parameter 1 of the compiler's `data`.
pub fn tmu_p1(data: u32, units: &[Unit]) -> Result<u32, Refusal> {
    if data & 0x00ff_fff8 != 0 {
        return Err(Refusal::DataBits { contents: TMU_CONFIG_P1, data });
    }
    let u = find(units, data >> 24)?;
    if u.sampler % SAMPLER_STATE_ALIGN != 0 {
        return Err(Refusal::SamplerAlign { addr: u.sampler });
    }
    Ok(u.sampler | (data & 0x7))
}

/// A texture size word (`data` is the unit).
pub fn size_word(contents: u32, data: u32, units: &[Unit]) -> Result<u32, Refusal> {
    let u = find(units, data)?;
    let v = match contents {
        TEXTURE_WIDTH => u.width,
        TEXTURE_HEIGHT => u.height,
        TEXTURE_DEPTH => u.depth,
        TEXTURE_ARRAY_SIZE => u.array_size,
        c => return Err(Refusal::NotTexture { contents: c }),
    };
    if v == 0 {
        return Err(Refusal::SizeNotGiven { contents, unit: data });
    }
    Ok(v)
}

/// Any of the six texture uniforms.
pub fn word(contents: u32, data: u32, units: &[Unit]) -> Result<u32, Refusal> {
    match contents {
        TMU_CONFIG_P0 => tmu_p0(data, units),
        TMU_CONFIG_P1 => tmu_p1(data, units),
        _ => size_word(contents, data, units),
    }
}
