// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The texture model the V3D interpreter (`v3d::sim`) and the IR evaluator
//! (`eval`) share: which level and which cube face a look-up reads, and the
//! texel NEAREST picks there.
//!
//! One transcription, used by both sides, so a disagreement between our
//! program and the IR is about the compiled look-up (its coordinates, its
//! level of detail, its face), never about two readings of the rules.
//!
//! The rules are dEQP's, because dEQP judges the driver, and they come from
//! v3d-tex's C ports, not from a new reading of dEQP:
//! - the cube face and the face coordinates: `tfr_select_cube_face` and
//!   `tfr_project_to_face` (tools/v3d/tex_filter_ref.h, from tcuTexture.cpp
//!   :3530-3631 at VK-GL-CTS 93bca01), faces in GL's order;
//! - the level a NEAREST_MIPMAP_NEAREST look-up reads for a level of detail:
//!   the nearest-mipmap arm of `tmc_levels` (tools/v3d/tex_mip_case.h, from
//!   tcuTexLookupVerifier.cpp isLookupResultValid), which is dEQP's
//!   `ceil(lod + 0.5) - 1` together with the `floor(lod + 0.5)` it also
//!   accepts; with dEQP's LOD precision (`tmc_fixed_error(8)`) around the
//!   look-up's level of detail;
//! - magnified or minified: `lod <= lodThreshold`, and the threshold is 0
//!   when the magnification filter is NEAREST (tcuTexture.cpp:3190-3194,
//!   sampleLevelArray2DOffset).
//!
//! `tests/fixtures/v3d_texref.txt` holds the C functions' own outputs on the
//! fixtures tex_cube_case.h and tex_mip_case.h use (written by
//! tools/v3d/texref_fixture.c); `texref::tests` compares every row bit for
//! bit, and refuses a fixture written from other versions of the headers.
//!
//! What is modelled: NEAREST within a level, CLAMP_TO_EDGE, the
//! magnification filter NEAREST, the minification filter NEAREST or
//! NEAREST_MIPMAP_NEAREST, a level of detail that is given (an explicit LOD,
//! or a texture with one level, where any level of detail reads level 0).
//! Refused, each by its own [`Refusal`]: LINEAR (not modelled: the board
//! measures it, gl_test case 109), a level of detail within dEQP's precision
//! of a level boundary (dEQP accepts either level, so the hardware's is not
//! predicted), a cube direction on or near a face edge (the hardware's tie
//! rule is not dEQP's to promise), a coordinate on a texel edge.

/// The cube faces in GL's order: `GL_TEXTURE_CUBE_MAP_POSITIVE_X + n`, which
/// is also the layer order of the V3D texture record (a cube is six layers
/// of one mip tree each, `cube_map_stride` apart: driver/src/v3d/v3d_tex.c
/// `v3d_tex_layer_offset`, and `gl::texture::face_of`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    pub const ALL: [Face; 6] = [Face::PosX, Face::NegX, Face::PosY, Face::NegY, Face::PosZ, Face::NegZ];

    /// The layer index: `GL_TEXTURE_CUBE_MAP_POSITIVE_X + index` is this face.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// `tfr_select_cube_face`, transcribed (tools/v3d/tex_filter_ref.h).
pub fn select_cube_face(x: f32, y: f32, z: f32) -> Face {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let xf = if x >= 0.0 { Face::PosX } else { Face::NegX };
    let yf = if y >= 0.0 { Face::PosY } else { Face::NegY };
    let zf = if z >= 0.0 { Face::PosZ } else { Face::NegZ };
    if ay < ax && az < ax {
        return xf;
    }
    if ax < ay && az < ay {
        return yf;
    }
    if ax < az && ay < az {
        return zf;
    }
    /* some components are equal: dEQP's tie-breaking rule */
    if ax == ay {
        return if ax < az { zf } else { xf };
    }
    if ax == az {
        return if az < ay { yf } else { zf };
    }
    if ay == az {
        return if ay < ax { xf } else { yf };
    }
    xf
}

/// `tfr_project_to_face`, transcribed: the face coordinates (s, t) in
/// [0, 1]; (0, 0) when the major axis is 0.
pub fn project_to_face(face: Face, x: f32, y: f32, z: f32) -> (f32, f32) {
    let (sc, tc, ma) = match face {
        Face::NegX => (z, -y, -x),
        Face::PosX => (-z, -y, x),
        Face::NegY => (x, -z, -y),
        Face::PosY => (x, z, y),
        Face::NegZ => (-x, -y, -z),
        Face::PosZ => (x, -y, z),
    };
    if ma.abs() < 1.192_092_9e-7 {
        return (0.0, 0.0);
    }
    (((sc / ma) + 1.0) / 2.0, ((tc / ma) + 1.0) / 2.0)
}

/// `tmc_fp_error` (computeFloatingPointError): the size of the value's
/// mantissa bits past `bits` accurate ones, at its exponent.
pub fn fp_error(value: f32, bits: u32) -> f32 {
    let exp = (value.to_bits() >> 23) & 0xff;
    let mask = (1u32 << (23 - bits)) - 1;
    f32::from_bits((exp << 23) | mask) - f32::from_bits(exp << 23)
}

/// `tmc_fixed_error` (computeFixedPointError).
pub fn fixed_error(bits: u32) -> f32 {
    fp_error(1.0, bits)
}

/// dEQP's LOD precision for these cases (es2fTextureMipmapTests.cpp:330-340,
/// `lodBits 8`, as tex_mip_case.h uses it).
pub const LOD_BITS: u32 = 8;

/// `tmc_floor` / `tmc_ceil`, as the C computes them (without libm).
fn floor_i(f: f32) -> i32 {
    let i = f as i32;
    if i as f32 > f {
        i - 1
    } else {
        i
    }
}
fn ceil_i(f: f32) -> i32 {
    let i = f as i32;
    if (i as f32) < f {
        i + 1
    } else {
        i
    }
}

/// The nearest-mipmap arm of `tmc_levels`, transcribed, for `levels`
/// levels: the lowest and highest level dEQP accepts for a level of detail
/// in `[lo, hi]` ("ceil(lod + 0.5) - 1 ... Khronos has made decision to allow
/// floor(lod + 0.5) as well").
pub fn nearest_mipmap_levels(lo: f32, hi: f32, levels: u32) -> (u32, u32) {
    let top = levels as i32 - 1;
    let lo_level = (ceil_i(lo + 0.5) - 1).clamp(0, top);
    let hi_level = floor_i(hi + 0.5).clamp(0, top);
    (lo_level as u32, hi_level as u32)
}

/// The minification filter of a texture's sampler. The magnification filter
/// is NEAREST (LINEAR is not modelled).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MinFilter {
    Nearest,
    NearestMipmapNearest,
}

/// One level of a texture: its size and its images, one for a 2D texture,
/// six for a cube in [`Face`] order. Row 0 is t = 0.
#[derive(Clone, Debug)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub faces: Vec<Vec<[f32; 4]>>,
}

/// A texture as its record and sampler describe it: 2D or cube, its levels
/// (level 0 first), and the minification filter.
#[derive(Clone, Debug)]
pub struct Texture {
    pub cube: bool,
    pub min: MinFilter,
    pub levels: Vec<Level>,
}

impl Texture {
    /// A 2D texture of one level, sampled NEAREST.
    pub fn flat(width: u32, height: u32, texels: Vec<[f32; 4]>) -> Texture {
        Texture { cube: false, min: MinFilter::Nearest, levels: vec![Level { width, height, faces: vec![texels] }] }
    }

    /// Level 0's size.
    pub fn size(&self) -> (u32, u32) {
        self.levels.first().map_or((0, 0), |l| (l.width, l.height))
    }
}

/// How a look-up chooses its level of detail.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lod {
    /// Given: an explicit level of detail.
    Explicit(f32),
    /// From the derivatives (and a bias, if any): not modelled, so read only
    /// where every level of detail reads the same level.
    Implicit,
}

/// Why a look-up is not predicted. One variant per cause, so a histogram
/// keyed on the message cannot merge two of them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Refusal {
    NotFinite,
    TexelEdge(f32),
    LevelBoundary(f32),
    ImplicitLod(usize),
    FaceEdge,
    ZeroDirection,
    NoLevel,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Refusal::NotFinite => write!(f, "a texture coordinate or level of detail that is not finite"),
            Refusal::TexelEdge(x) => write!(f, "coordinate {x} on a texel edge"),
            Refusal::LevelBoundary(l) => {
                write!(f, "a level of detail {l} within dEQP's LOD precision of a level boundary")
            }
            Refusal::ImplicitLod(n) => {
                write!(f, "an implicit level of detail on a texture of {n} levels (derivatives are not modelled)")
            }
            Refusal::FaceEdge => write!(f, "a cube direction within 1e-3 of a face edge"),
            Refusal::ZeroDirection => write!(f, "a cube direction of length 0"),
            Refusal::NoLevel => write!(f, "a texture with no level"),
        }
    }
}

/// The level a look-up reads.
pub fn level(t: &Texture, lod: Lod) -> Result<usize, Refusal> {
    let n = t.levels.len();
    if n == 0 {
        return Err(Refusal::NoLevel);
    }
    let lod = match lod {
        /* Every level of detail reads level 0 of a one-level texture: the
         * magnification filter is NEAREST and the minification filter reads
         * level 0 (NEAREST) or the only level there is. */
        Lod::Implicit if n == 1 => return Ok(0),
        Lod::Implicit => return Err(Refusal::ImplicitLod(n)),
        Lod::Explicit(l) => l,
    };
    if !lod.is_finite() {
        return Err(Refusal::NotFinite);
    }
    /* magnified (lod <= 0 with a NEAREST magnification filter) or a
     * minification filter without mipmaps: level 0 */
    if t.min == MinFilter::Nearest || lod <= 0.0 {
        return Ok(0);
    }
    let e = fixed_error(LOD_BITS);
    let (lo, hi) = nearest_mipmap_levels(lod - e, lod + e, n as u32);
    if lo != hi {
        return Err(Refusal::LevelBoundary(lod));
    }
    Ok(lo as usize)
}

/// The texel index NEAREST reads at a normalised coordinate on a side of
/// `n` texels, CLAMP_TO_EDGE: dEQP's `floor(u)` of `u = n s`, clamped
/// (`tfr_nearest` with TFR_CLAMP_TO_EDGE). A coordinate within 1e-3 of a
/// texel edge inside the image is refused: where u is an integer the texel
/// depends on the hardware's rounding, which no spec fixes.
pub fn nearest_index(s: f32, n: u32) -> Result<u32, Refusal> {
    if !s.is_finite() {
        return Err(Refusal::NotFinite);
    }
    let u = s * n as f32;
    if u > 0.0 && u < n as f32 && (u - u.round()).abs() < 1e-3 {
        return Err(Refusal::TexelEdge(s));
    }
    Ok((floor_i(u).max(0) as u32).min(n - 1))
}

/// The texel a 2D look-up at (s, t) reads: NEAREST on its level.
pub fn sample_2d(t: &Texture, s: f32, tc: f32, lod: Lod) -> Result<[f32; 4], Refusal> {
    let l = &t.levels[level(t, lod)?];
    let (i, j) = (nearest_index(s, l.width)?, nearest_index(tc, l.height)?);
    Ok(l.faces[0][(j * l.width + i) as usize])
}

/// The texel a cube look-up at direction (x, y, z) reads, and the face.
/// The direction is refused where its two largest components are within
/// 1e-3 (relative) of each other: there the face is a tie rule's choice.
pub fn sample_cube(t: &Texture, x: f32, y: f32, z: f32, lod: Lod) -> Result<([f32; 4], Face), Refusal> {
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return Err(Refusal::NotFinite);
    }
    let mut m = [x.abs(), y.abs(), z.abs()];
    m.sort_by(|a, b| b.total_cmp(a));
    if m[0] == 0.0 {
        return Err(Refusal::ZeroDirection);
    }
    if m[0] - m[1] < 1e-3 * m[0] {
        return Err(Refusal::FaceEdge);
    }
    let face = select_cube_face(x, y, z);
    let (s, tc) = project_to_face(face, x, y, z);
    let l = &t.levels[level(t, lod)?];
    let (i, j) = (nearest_index(s, l.width)?, nearest_index(tc, l.height)?);
    Ok((l.faces[face.index()][(j * l.width + i) as usize], face))
}

#[cfg(test)]
mod tests {
    //! Pinned against v3d-tex's C functions: tests/fixtures/v3d_texref.txt
    //! holds their outputs, written by tools/v3d/texref_fixture.c, on the
    //! directions tex_cube_case.h draws (tcc_dir, all 6 x 32 x 32 pixels),
    //! the hand cases of tex_filter_ref_check.c's part_cube, and a grid of
    //! levels of detail over tex_mip_case.h's 7 levels.
    use super::*;
    use std::string::{String, ToString};
    use std::vec::Vec;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/v3d_texref.txt");
    const FILTER_REF: &[u8] = include_bytes!("../../../tools/v3d/tex_filter_ref.h");
    const MIP_CASE: &[u8] = include_bytes!("../../../tools/v3d/tex_mip_case.h");

    fn hex(b: &[u8; 32]) -> String {
        b.iter().map(|x| std::format!("{x:02x}")).collect()
    }

    fn header(key: &str) -> &'static str {
        FIXTURE
            .lines()
            .find_map(|l| l.strip_prefix("# ").and_then(|r| r.strip_prefix(key)).and_then(|r| r.strip_prefix(": ")))
            .unwrap_or_else(|| panic!("the fixture has no '{key}' line"))
    }

    fn f(word: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(word, 16).expect("a hex float"))
    }

    /// The fixture was written from the headers in the tree: a header that
    /// changed without the fixture being written again fails here, with the
    /// command that writes it.
    #[test]
    fn the_fixture_is_the_c_headers_of_this_tree() {
        let sha = crate::shaderc::sha256::sha256;
        assert_eq!(
            (header("tex_filter_ref.h"), header("tex_mip_case.h")),
            (hex(&sha(FILTER_REF)).as_str(), hex(&sha(MIP_CASE)).as_str()),
            "tools/v3d/texref_fixture.c must be run again (see its header comment)"
        );
    }

    /// Every cube row: the face and the (s, t) bits the C gives.
    #[test]
    fn the_cube_face_and_projection_are_the_c_functions() {
        let mut rows = 0usize;
        for l in FIXTURE.lines().filter_map(|l| l.strip_prefix("cube ")) {
            let w: Vec<&str> = l.split_whitespace().collect();
            let (x, y, z) = (f(w[0]), f(w[1]), f(w[2]));
            let face = select_cube_face(x, y, z);
            let (s, t) = project_to_face(face, x, y, z);
            assert_eq!(
                (face.index(), s.to_bits(), t.to_bits()),
                (w[3].parse().unwrap(), f(w[4]).to_bits(), f(w[5]).to_bits()),
                "direction ({x}, {y}, {z})"
            );
            rows += 1;
        }
        assert_eq!(rows.to_string(), header("cube rows"), "every cube row was read");
        assert!(rows >= 6 * 32 * 32, "tex_cube_case.h's whole draw set: {rows}");
    }

    /// Every level row: the lowest and highest level `tmc_levels` gives for
    /// a nearest-mipmap filter.
    #[test]
    fn the_level_choice_is_the_c_function() {
        let mut rows = 0usize;
        for l in FIXTURE.lines().filter_map(|l| l.strip_prefix("level ")) {
            let w: Vec<&str> = l.split_whitespace().collect();
            let (lo, hi) = (f(w[0]), f(w[1]));
            let got = nearest_mipmap_levels(lo, hi, 7);
            assert_eq!(got, (w[2].parse().unwrap(), w[3].parse().unwrap()), "LOD bounds [{lo}, {hi}]");
            rows += 1;
        }
        assert_eq!(rows.to_string(), header("level rows"), "every level row was read");
        assert_eq!(fixed_error(LOD_BITS).to_bits(), f(header("fixed_error(8)")).to_bits());
    }

    /// The model around the functions: a face edge, a level boundary and an
    /// implicit level of detail on a mipmapped texture are refused by name;
    /// an explicit one picks the level dEQP picks.
    #[test]
    fn the_model_refuses_what_it_cannot_predict() {
        let lv = |w: u32, v: f32| Level { width: w, height: w, faces: vec![vec![[v; 4]; (w * w) as usize]; 6] };
        let t = Texture {
            cube: true,
            min: MinFilter::NearestMipmapNearest,
            levels: vec![lv(4, 0.0), lv(2, 1.0), lv(1, 2.0)],
        };
        assert_eq!(level(&t, Lod::Explicit(0.2)), Ok(0));
        assert_eq!(level(&t, Lod::Explicit(0.9)), Ok(1), "ceil(0.9 + 0.5) - 1");
        assert_eq!(level(&t, Lod::Explicit(7.0)), Ok(2), "clamped to the last level");
        assert_eq!(level(&t, Lod::Explicit(-3.0)), Ok(0), "magnified");
        assert_eq!(level(&t, Lod::Explicit(1.5)), Err(Refusal::LevelBoundary(1.5)));
        assert_eq!(level(&t, Lod::Implicit), Err(Refusal::ImplicitLod(3)));
        assert_eq!(sample_cube(&t, 1.0, 1.0, 0.2, Lod::Explicit(0.0)), Err(Refusal::FaceEdge));
        assert_eq!(sample_cube(&t, 0.0, 0.0, 0.0, Lod::Explicit(0.0)), Err(Refusal::ZeroDirection));
        let (v, face) = sample_cube(&t, 0.3, -1.0, 0.1, Lod::Explicit(0.9)).unwrap();
        assert_eq!((v[0], face), (1.0, Face::NegY), "level 1 of -Y");
        let one = Texture::flat(2, 2, vec![[0.5; 4]; 4]);
        assert_eq!(level(&one, Lod::Implicit), Ok(0), "one level: any level of detail reads it");
    }
}
