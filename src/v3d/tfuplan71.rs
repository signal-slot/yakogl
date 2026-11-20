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
    /// Bytes of the level padded to whole UIF blocks (planning figure).
    pub bytes: u64,
    /// Offset of the level from the first generated level (base + 1), the
    /// levels packed in order (planning figure; the layout is the caller's).
    pub offset: u64,
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

/// Plan the jobs: `layer_count` jobs (at least 1), each generating levels
/// `base_level+1 ..= last_level` from `base_level`.
pub fn plan(a: &Args) -> Result<Vec<Job>, Refusal> {
    let uh = utile_height(a.cpp).ok_or(Refusal::BadCpp)?;
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
    // UIF block = 2x2 utiles of 64 bytes: 2*uh rows, 2*(64/(cpp*uh)) texels.
    let (bh, bw) = (2 * uh, 2 * (64 / (a.cpp * uh)));
    let mut levels = Vec::new();
    let mut off = 0u64;
    for i in 1..=nummm {
        let (w, h) = ((a.width >> i).max(1), (a.height >> i).max(1));
        let bytes = (w.div_ceil(bw) as u64) * (h.div_ceil(bh) as u64) * 256;
        levels.push(Level { level: a.base_level + i, width: w, height: h, bytes, offset: off });
        off += bytes;
    }
    let n = a.layer_count.max(1);
    Ok((0..n)
        .map(|k| Job {
            in_level: a.base_level,
            out_level: a.base_level + 1,
            layer: a.layer + k,
            nummm,
            ioc_mm: 1 | nummm << IOC_NUMMM_SHIFT,
            levels: levels.clone(),
        })
        .collect())
}
