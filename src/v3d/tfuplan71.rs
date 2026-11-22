// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D 7.1 glGenerateMipmap plan through the TFU with IOC (pure, no `crate::`
//! imports so a test can include it with `#[path]`). It mirrors
//! `driver/src/v3d/v3d_tfu71.{h,c}` (Mesa 26.2.2 `v3dx_tfu.c`, 7.1 branch):
//! ONE job makes every level after the base (`NUMMM` = last - base, `DIMTW`),
//! so there is no split: the 4-bit field (bits 4..7) holds at most 15 and a
//! 16384 px side halves 14 times. A cube face or array layer is one job.

/// `V3D_TFU71_MAX_DIM` (`v3d_tfu71.h`).
pub const MAX_DIM: u32 = 16384;
/// `V3D_TFU71_IOC_NUMMM_SHIFT` (`v3d_tfu71.h`).
pub const IOC_NUMMM_SHIFT: u32 = 4;
/// The NUMMM field is 4 bits wide (bits 4..7; FORMAT starts at bit 12).
pub const MAX_NUMMM: u32 = 15;

/// Texture data types `v3d_tfu71_supports(ttype, true)` accepts
/// (the `return true` group of v3d_tfu71.c).
pub const SUPPORTED_MIPMAP: &[u32] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 25];
/// Types accepted only for a plain copy (`return !for_mipmap`).
pub const SUPPORTED_COPY_ONLY: &[u32] = &[20, 29, 30, 31];

/// `v3d_tfu71_supports`.
pub fn supports(ttype: u32, for_mipmap: bool) -> bool {
    SUPPORTED_MIPMAP.contains(&ttype) || (!for_mipmap && SUPPORTED_COPY_ONLY.contains(&ttype))
}

/// Tiling of one side, Mesa's `enum v3d_tiling_mode` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiling {
    Raster = 0,
    LinearTile,
    UbLinear1Column,
    UbLinear2Column,
    UifNoXor,
    UifXor,
}

/// Named refusals: what the TFU cannot do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `v3d_tfu71_supports(ttype, true)` is false.
    UnsupportedType,
    /// bytes per texel is not 1, 2, 4, 8 or 16.
    BadCpp,
    /// width or height is 0 or above 16384.
    Dimension,
    /// `last_level < base_level`, or more levels than the larger side halves
    /// to 1, or more than the 4-bit NUMMM field holds.
    LevelCount,
    /// nothing to generate (`last_level == base_level`).
    NoLevels,
    /// the destination is raster ("TFU requires UIF when generating mipmaps").
    RasterDestination,
    /// the source is not the base level's layout (another level, or a
    /// source tiling that differs from the destination's).
    SourceNotBase,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::UnsupportedType => "v3d71 tfu: texture type not supported for mipmap generation",
            Refusal::BadCpp => "v3d71 tfu: bytes per texel must be 1, 2, 4, 8 or 16",
            Refusal::Dimension => "v3d71 tfu: base level side is 0 or above 16384",
            Refusal::LevelCount => "v3d71 tfu: level count not expressible by IOC NUMMM",
            Refusal::NoLevels => "v3d71 tfu: no level after the base to generate",
            Refusal::RasterDestination => "v3d71 tfu: mipmaps need a tiled (UIF) destination",
            Refusal::SourceNotBase => "v3d71 tfu: source must be the base level in the destination's layout",
        }
    }
}

/// One generated level of a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub level: u32,
    pub width: u32,
    pub height: u32,
    /// Bytes of the level's slice (Mesa's `slice->size`, one layer).
    pub bytes: u64,
    /// Offset of the level in the buffer, layer 0 (Mesa's `slice->offset`
    /// after the 4 KiB shift): smaller levels sit BELOW the base level.
    pub offset: u64,
    /// The level's tiling by Mesa's `v3d_setup_slices` rules (not always UIF:
    /// small levels are LINEARTILE or UBLINEAR).
    pub tiling: Tiling,
}

/// One TFU job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub in_level: u32,
    /// First level written (the one after `in_level`).
    pub out_level: u32,
    pub layer: u32,
    pub nummm: u32,
    /// The IOC bits this plan decides: DIMTW | NUMMM << 4.
    pub ioc_mm: u32,
    /// Offset of `in_level` in the buffer, layer 0 (the TFU's IIA/IOA point
    /// here; the hardware writes the smaller levels below it).
    pub base_offset: u64,
    pub levels: Vec<Level>,
}

/// Plan inputs.
#[derive(Clone, Copy, Debug)]
pub struct Args {
    /// The base level's size.
    pub width: u32,
    pub height: u32,
    pub base_level: u32,
    pub last_level: u32,
    /// The level the source image is (must equal `base_level`).
    pub src_level: u32,
    pub ttype: u32,
    pub cpp: u32,
    pub src_tiling: Tiling,
    pub dst_tiling: Tiling,
    /// Cube faces or array layers to generate (one job each), from `layer`.
    pub layer: u32,
    pub layer_count: u32,
}

fn utile_height(cpp: u32) -> Option<u32> {
    match cpp {
        1 => Some(8),
        2 | 4 => Some(4),
        8 | 16 => Some(2),
        _ => None,
    }
}

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

/// Mesa's `v3d_get_ub_pad` (v3d_resource.c), 4 KiB pages, 8 banks.
fn ub_pad(uh: u32, height: u32) -> u32 {
    const PAGE_UB_ROWS: u32 = 4096 / 1024;
    const X15: u32 = (PAGE_UB_ROWS * 3) >> 1;
    const PC_ROWS: u32 = 4096 * 8 / 1024;
    let h_ub = height / (uh * 2);
    let in_pc = h_ub % PC_ROWS;
    if in_pc == 0 {
        0
    } else if in_pc < X15 {
        if h_ub < PC_ROWS {
            0
        } else {
            X15 - in_pc
        }
    } else if in_pc > PC_ROWS - X15 {
        PC_ROWS - in_pc
    } else {
        0
    }
}

/// The slices of `v3d_setup_slices` (Mesa 26.2.1 v3d_resource.c:583-736;
/// gallium's files are not vendored) for a single-sampled, uncompressed 2D
/// resource whose level 0 is `width` x `height`, levels `0..=last` laid out
/// from `last` down to 0 (level 0 last), then everything shifted so level 0
/// starts on 4 KiB (:738-754). Returns levels 1..=last and level 0's offset.
/// The pot padding of levels >= 2 follows level 1 (:597-607), so a chain
/// planned from a base above level 0 assumes `width`/`height` are level 0's.
fn mesa_levels(width: u32, height: u32, cpp: u32, last: u32) -> (Vec<Level>, u64) {
    let (uw, uh) = (
        if cpp <= 2 {
            8
        } else if cpp <= 8 {
            4
        } else {
            2
        },
        match cpp {
            1 => 8,
            2 | 4 => 4,
            _ => 2,
        },
    );
    let (bw, bh) = (uw * 2, uh * 2);
    let pot = |d: u32| 2 * (d >> 1).max(1).next_power_of_two();
    let (pw, ph) = (pot(width), pot(height));
    let mut slices = vec![(Tiling::LinearTile, 0u64, 0u64); last as usize + 1];
    let mut off = 0u64;
    for i in (0..=last).rev() {
        let (mut lw, mut lh) =
            if i < 2 { ((width >> i).max(1), (height >> i).max(1)) } else { ((pw >> i).max(1), (ph >> i).max(1)) };
        let t = if lw <= uw || lh <= uh {
            lw = align(lw, uw);
            lh = align(lh, uh);
            Tiling::LinearTile
        } else if lw <= bw {
            lw = align(lw, bw);
            lh = align(lh, bh);
            Tiling::UbLinear1Column
        } else if lw <= 2 * bw {
            lw = align(lw, 2 * bw);
            lh = align(lh, bh);
            Tiling::UbLinear2Column
        } else {
            lw = align(lw, 4 * bw);
            lh = align(lh, bh);
            lh += ub_pad(uh, lh) * bh;
            if (lh / bh) % 32 == 0 {
                Tiling::UifXor
            } else {
                Tiling::UifNoXor
            }
        };
        let size = u64::from(lh) * u64::from(lw * cpp);
        slices[i as usize] = (t, off, size);
        let mut total = size;
        // level 1 is page aligned when it could be UIF XOR (:707-716)
        if i == 1 && lw > 4 * bw && lh > (32 - 6) * bh {
            total = total.div_ceil(4096) * 4096;
        }
        off += total;
    }
    let shift = u64::from(align(slices[0].1 as u32, 4096)) - slices[0].1;
    let levels = (1..=last)
        .map(|i| {
            let (t, o, size) = slices[i as usize];
            Level {
                level: i,
                width: (width >> i).max(1),
                height: (height >> i).max(1),
                bytes: size,
                offset: o + shift,
                tiling: t,
            }
        })
        .collect();
    (levels, slices[0].1 + shift)
}

/// Plan the jobs: `layer_count` jobs (at least 1), each generating levels
/// `base_level+1 ..= last_level` from `base_level`.
pub fn plan(a: &Args) -> Result<Vec<Job>, Refusal> {
    utile_height(a.cpp).ok_or(Refusal::BadCpp)?;
    if a.width == 0 || a.height == 0 || a.width > MAX_DIM || a.height > MAX_DIM {
        return Err(Refusal::Dimension);
    }
    if !supports(a.ttype, true) {
        return Err(Refusal::UnsupportedType);
    }
    if a.last_level < a.base_level {
        return Err(Refusal::LevelCount);
    }
    let nummm = a.last_level - a.base_level;
    if nummm == 0 {
        return Err(Refusal::NoLevels);
    }
    let mut m = a.width.max(a.height);
    let mut halvings = 0;
    while m > 1 {
        m >>= 1;
        halvings += 1;
    }
    if nummm > halvings || nummm > MAX_NUMMM {
        return Err(Refusal::LevelCount);
    }
    if a.dst_tiling == Tiling::Raster {
        return Err(Refusal::RasterDestination);
    }
    if a.src_level != a.base_level || a.src_tiling != a.dst_tiling {
        return Err(Refusal::SourceNotBase);
    }
    let (levels, base_offset) = mesa_levels(a.width, a.height, a.cpp, nummm);
    let n = a.layer_count.max(1);
    Ok((0..n)
        .map(|k| Job {
            in_level: a.base_level,
            out_level: a.base_level + 1,
            layer: a.layer + k,
            nummm,
            ioc_mm: 1 | nummm << IOC_NUMMM_SHIFT,
            base_offset,
            levels: levels.clone(),
        })
        .collect())
}
