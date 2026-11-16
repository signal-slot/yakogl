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
//! What is modelled by the NEAREST entry points ([`sample_2d`],
//! [`sample_cube`], the IR evaluator's): NEAREST within a level,
//! CLAMP_TO_EDGE, the magnification filter NEAREST, the minification filter
//! NEAREST or NEAREST_MIPMAP_NEAREST, a level of detail that is given (an
//! explicit LOD, or a texture with one level, where any level of detail
//! reads level 0). Refused, each by its own [`Refusal`]: a level of detail
//! within dEQP's precision of a level boundary (dEQP accepts either level,
//! so the hardware's is not predicted), a cube direction on or near a face
//! edge (the hardware's tie rule is not dEQP's to promise), a coordinate on
//! a texel edge.
//!
//! The sampler-state entry points ([`sample_layer`],
//! [`sample_cube_filtered`], the V3D interpreter's, 2026-10-07) take a
//! [`Sampler`]: the magnification filter, the six minification filters and
//! the three GL ES 2.0 wrap modes. LINEAR is dEQP's `sampleLinear2D`
//! (`tfr_linear`, the same header: x0 = floor(u - 0.5), a = frac(u - 0.5),
//! the four taps wrapped by the sampler), a mipmap-LINEAR filter blends
//! floor(lod) and the level after it by frac(lod) (`sampleLevelArray2D`),
//! and the switch from the magnification to the minification filter is at
//! lod <= 0 (ASSUMPTION: the TMU's own switch, which no sampler word moves;
//! GL ES 2.0 3.7.8's c = 0.5 for mag LINEAR with min NEAREST_MIPMAP_* is the
//! shader's to emulate, `lodkey.rs`). The filter weights are f32, not the
//! TMU's fixed-point ones: a result is the exact bilinear value, and a
//! comparison with the hardware allows dEQP's LINEAR tolerance (gl_test case
//! 109 measured the board, docs/online-shaderc/v3d-backend.md "LINEAR on the
//! board"). A cube look-up is SEAMLESS, as the V3D 4.2 filters every cube
//! (docs/v3d-noseamless-cube.md): the wrap modes are ignored and a LINEAR
//! tap past a face edge reads the texel of the neighbouring face its centre
//! points at ([`seamless_texel`]); a tap past a corner (out of range in both
//! axes) is the mean of the other three taps, dEQP's `getCubeLinearSamples`
//! convention, which ES 3.0 3.8.9.1 recommends and the hardware's rule is
//! not known to be, so a test stays away from corners. A 2D-array look-up on
//! a cube texture (the V3D's TMUI on the cube's own record) reads one face
//! as an independent 2D texture with the sampler's wrap modes: that is ES
//! 2.0's non-seamless rule (3.7.5), and the emulation's path.
//!
//! The ES 3.0 forms (2026-11-07): a level's images are a 2D array's layers
//! or a 3D texture's slices as well as a cube's faces ([`Level::faces`]; the
//! look-up decides which, the texture does not say). [`layer_of`] is the
//! layer of a 2D-array coordinate (3.8.9: the nearest, clamped);
//! [`sample_layer_offset`] and [`sample_3d`] add a constant texel offset
//! (`textureOffset` and its forms) to the texel coordinates of each level
//! read before the wrap (3.8.10); [`sample_3d`] picks the slice NEAREST
//! along r with CLAMP_TO_EDGE and refuses a LINEAR filter (no blend across
//! slices is modelled); [`fetch`] is `texelFetch`'s texel of its level and
//! image, no filter and no wrap, refused by name outside the texture (8.8
//! leaves it undefined).

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

/// The minification filter of a texture's sampler (GL's six). The NEAREST
/// entry points take the first two; [`Sampler`] takes all six.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MinFilter {
    Nearest,
    NearestMipmapNearest,
    Linear,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

impl MinFilter {
    /// The filter within a level when this filter minifies.
    pub fn within_level(self) -> Filter {
        match self {
            MinFilter::Nearest | MinFilter::NearestMipmapNearest | MinFilter::NearestMipmapLinear => Filter::Nearest,
            MinFilter::Linear | MinFilter::LinearMipmapNearest | MinFilter::LinearMipmapLinear => Filter::Linear,
        }
    }

    /// Whether it reads a mipmap chain, and if so whether it blends two
    /// levels (`Some(true)`) or picks one (`Some(false)`).
    pub fn mipmap(self) -> Option<bool> {
        match self {
            MinFilter::Nearest | MinFilter::Linear => None,
            MinFilter::NearestMipmapNearest | MinFilter::LinearMipmapNearest => Some(false),
            MinFilter::NearestMipmapLinear | MinFilter::LinearMipmapLinear => Some(true),
        }
    }
}

/// A filter within one level: GL's TEXTURE_MAG_FILTER values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    Nearest,
    Linear,
}

/// GL ES 2.0's wrap modes (TEXTURE_WRAP_S / _T).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wrap {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

/// A sampler's state, as the TMU's sampler record holds it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sampler {
    pub mag: Filter,
    pub min: MinFilter,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}

impl Sampler {
    /// The sampler the NEAREST entry points assume: NEAREST both ways,
    /// CLAMP_TO_EDGE both ways.
    pub const NEAREST_CLAMP: Sampler =
        Sampler { mag: Filter::Nearest, min: MinFilter::Nearest, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };

    /// The sampler a texture described without one has: [`Self::NEAREST_CLAMP`]
    /// with the texture's own minification filter.
    pub fn of(t: &Texture) -> Sampler {
        Sampler { min: t.min, ..Sampler::NEAREST_CLAMP }
    }
}

/// `tfr_wrap` (tcuTexture.cpp `wrap`, the three GL ES 2.0 modes): texel
/// index `c` of a side of `n` texels, wrapped by `mode`.
pub fn wrap_index(mode: Wrap, c: i32, n: u32) -> u32 {
    let n = n as i32;
    let imod = |a: i32, b: i32| {
        let m = a % b;
        if m < 0 {
            m + b
        } else {
            m
        }
    };
    let mirror = |a: i32| if a >= 0 { a } else { -(1 + a) };
    let w = match mode {
        Wrap::ClampToEdge => c.clamp(0, n - 1),
        Wrap::Repeat => imod(c, n),
        Wrap::MirroredRepeat => (n - 1) - mirror(imod(c, 2 * n) - n),
    };
    w as u32
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
    /// A mipmap-LINEAR filter at a level of detail that blends two levels,
    /// asked of the NEAREST entry point [`level`], which answers one level.
    LevelBlend(f32),
    /// A 2D-array look-up whose layer is not finite.
    LayerNotFinite,
    /// A LINEAR filter on a 3D look-up: a blend across two slices, which
    /// [`sample_3d`] does not model (NEAREST along r only).
    VolumeLinear,
    /// A texel fetch of a level the texture has not (the level, the count).
    FetchLevel(i32, usize),
    /// A texel fetch outside its level's image: the texel, the level and
    /// the level's size.
    FetchTexel {
        i: i32,
        j: i32,
        level: i32,
        width: u32,
        height: u32,
    },
    /// A texel fetch of a layer (or a 3D texture's slice) the level has not
    /// (the image index, the count).
    FetchLayer(i32, usize),
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
            Refusal::LevelBlend(l) => {
                write!(f, "a level of detail {l} that blends two levels under a mipmap-LINEAR filter")
            }
            Refusal::LayerNotFinite => write!(f, "a 2D-array layer that is not finite"),
            Refusal::VolumeLinear => {
                write!(f, "a LINEAR filter on a 3D look-up (a blend across slices is not modelled)")
            }
            Refusal::FetchLevel(l, n) => write!(f, "a texel fetch of level {l} on a texture of {n} levels"),
            Refusal::FetchTexel { i, j, level, width, height } => {
                write!(f, "a texel fetch of ({i}, {j}) outside level {level}'s {width}x{height}")
            }
            Refusal::FetchLayer(k, n) => write!(f, "a texel fetch of layer {k} on a texture of {n} layers"),
        }
    }
}

/// The level a NEAREST look-up reads (one level; a blend is refused).
pub fn level(t: &Texture, lod: Lod) -> Result<usize, Refusal> {
    match levels(t, t.min, lod)? {
        (l, None) => Ok(l),
        (_, Some(_)) => Err(Refusal::LevelBlend(match lod {
            Lod::Explicit(l) => l,
            Lod::Implicit => f32::NAN,
        })),
    }
}

/// The level(s) a look-up with minification filter `min` reads at `lod`:
/// the first level, and for a mipmap-LINEAR filter that blends, the second
/// with its weight (dEQP's `sampleLevelArray2D`: floor(lod) and the level
/// after it, weighed by frac(lod)). Magnified (lod <= 0) or without a mipmap
/// filter: level 0 alone. A one-level texture reads its level whatever the
/// level of detail; an implicit one on a longer chain is not modelled.
pub fn levels(t: &Texture, min: MinFilter, lod: Lod) -> Result<(usize, Option<(usize, f32)>), Refusal> {
    let n = t.levels.len();
    if n == 0 {
        return Err(Refusal::NoLevel);
    }
    let lod = match lod {
        Lod::Implicit if n == 1 => return Ok((0, None)),
        Lod::Implicit => return Err(Refusal::ImplicitLod(n)),
        Lod::Explicit(l) => l,
    };
    if !lod.is_finite() {
        return Err(Refusal::NotFinite);
    }
    let Some(blend) = min.mipmap() else { return Ok((0, None)) };
    if lod <= 0.0 {
        return Ok((0, None));
    }
    let top = n as i32 - 1;
    if blend {
        let l0 = floor_i(lod).clamp(0, top);
        let l1 = (l0 + 1).min(top);
        let w = lod - lod.floor();
        if l0 == l1 || w == 0.0 {
            return Ok((l0 as usize, None));
        }
        return Ok((l0 as usize, Some((l1 as usize, w))));
    }
    let e = fixed_error(LOD_BITS);
    let (lo, hi) = nearest_mipmap_levels(lod - e, lod + e, n as u32);
    if lo != hi {
        return Err(Refusal::LevelBoundary(lod));
    }
    Ok((lo as usize, None))
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

/* ---- the sampler-state entry points ------------------------------------- */

/// The direction of face coordinates (sc, tc) on `face`'s plane (the major
/// axis at 1): the inverse of [`project_to_face`]'s table, ES 2.0 Table
/// 3.21 read backwards. `sc` or `tc` outside [-1, 1] is a point past the
/// face's edge, which another face owns.
pub fn face_direction(face: Face, sc: f32, tc: f32) -> (f32, f32, f32) {
    match face {
        Face::PosX => (1.0, -tc, -sc),
        Face::NegX => (-1.0, -tc, sc),
        Face::PosY => (sc, 1.0, tc),
        Face::NegY => (sc, -1.0, -tc),
        Face::PosZ => (sc, -tc, 1.0),
        Face::NegZ => (-sc, -tc, -1.0),
    }
}

/// The texel a seamless LINEAR tap at integer face coordinates (i, j) of
/// `face` reads on level `l`: the texel itself inside the face; past one
/// edge, the texel of the neighbouring face that the tap's centre points at
/// (its direction on this face's plane, [`face_direction`], selected and
/// projected as any direction is: the index along the shared edge is kept,
/// the other is the neighbour's edge column or row); past a corner, `None`.
pub fn seamless_texel(l: &Level, face: Face, i: i32, j: i32) -> Option<[f32; 4]> {
    let n = l.width as i32;
    let (in_u, in_v) = (0 <= i && i < n, 0 <= j && j < n);
    if in_u && in_v {
        return Some(l.faces[face.index()][(j * n + i) as usize]);
    }
    if !in_u && !in_v {
        return None;
    }
    let centre = |k: i32| (k as f32 + 0.5) / n as f32 * 2.0 - 1.0;
    let (x, y, z) = face_direction(face, centre(i), centre(j));
    let g = select_cube_face(x, y, z);
    let (s, t) = project_to_face(g, x, y, z);
    let idx = |c: f32| floor_i(c * n as f32).clamp(0, n - 1);
    Some(l.faces[g.index()][(idx(t) * n + idx(s)) as usize])
}

/// dEQP's `sampleNearest2D` / `sampleLinear2D` on one face of level `l`
/// with the sampler's wrap modes: the texel NEAREST reads at normalised
/// (s, t), or the bilinear blend of the four texels around it. A NEAREST
/// coordinate within 1e-3 of a texel edge is refused as [`nearest_index`]
/// does (under REPEAT and MIRRORED_REPEAT every edge counts, the image's
/// own included: the next texel is a wrapped one).
pub fn sample_face(l: &Level, face: usize, filter: Filter, sp: &Sampler, s: f32, t: f32) -> Result<[f32; 4], Refusal> {
    sample_face_with(l, face, filter, sp, s, t, |v| v)
}

/// [`sample_face`] with `tap` applied to every texel read before the blend:
/// the one body of the plain and the compare-then-filter samplers.
fn sample_face_with(
    l: &Level,
    face: usize,
    filter: Filter,
    sp: &Sampler,
    s: f32,
    t: f32,
    tap: impl Fn([f32; 4]) -> [f32; 4],
) -> Result<[f32; 4], Refusal> {
    if !(s.is_finite() && t.is_finite()) {
        return Err(Refusal::NotFinite);
    }
    let texels = &l.faces[face];
    let (w, h) = (l.width, l.height);
    let at = |i: u32, j: u32| tap(texels[(j * w + i) as usize]);
    let (u, v) = (s * w as f32, t * h as f32);
    match filter {
        Filter::Nearest => {
            let edge = |c: f32, mode: Wrap, n: u32| {
                let inside = c > 0.0 && c < n as f32;
                (inside || mode != Wrap::ClampToEdge) && (c - c.round()).abs() < 1e-3
            };
            if edge(u, sp.wrap_s, w) {
                return Err(Refusal::TexelEdge(s));
            }
            if edge(v, sp.wrap_t, h) {
                return Err(Refusal::TexelEdge(t));
            }
            Ok(at(wrap_index(sp.wrap_s, floor_i(u), w), wrap_index(sp.wrap_t, floor_i(v), h)))
        }
        Filter::Linear => {
            let (x0, y0) = (floor_i(u - 0.5), floor_i(v - 0.5));
            let (a, b) = (u - 0.5 - (x0 as f32), v - 0.5 - (y0 as f32));
            let (i0, i1) = (wrap_index(sp.wrap_s, x0, w), wrap_index(sp.wrap_s, x0 + 1, w));
            let (j0, j1) = (wrap_index(sp.wrap_t, y0, h), wrap_index(sp.wrap_t, y0 + 1, h));
            Ok(bilinear([at(i0, j0), at(i1, j0), at(i0, j1), at(i1, j1)], a, b))
        }
    }
}

/// (1-a)(1-b) p00 + a(1-b) p10 + (1-a)b p01 + ab p11.
fn bilinear(p: [[f32; 4]; 4], a: f32, b: f32) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        *o = p[0][c] * (1.0 - a) * (1.0 - b) + p[1][c] * a * (1.0 - b) + p[2][c] * (1.0 - a) * b + p[3][c] * a * b;
    }
    out
}

/// The filter within a level for `lod` under `sp`: the magnification filter
/// at lod <= 0 (the TMU's switch, module doc), else the minification
/// filter's.
fn filter_at(sp: &Sampler, lod: Lod) -> Filter {
    match lod {
        Lod::Explicit(l) if l > 0.0 => sp.min.within_level(),
        _ => sp.mag,
    }
}

/// `sample` applied on the level(s) `lod` selects under `sp`, the two
/// levels of a mipmap-LINEAR blend weighed (dEQP's `sampleLevelArray2D`).
fn over_levels(
    t: &Texture,
    sp: &Sampler,
    lod: Lod,
    sample: impl Fn(&Level, Filter) -> Result<[f32; 4], Refusal>,
) -> Result<[f32; 4], Refusal> {
    let (l0, second) = levels(t, sp.min, lod)?;
    let filter = filter_at(sp, lod);
    let p0 = sample(&t.levels[l0], filter)?;
    let Some((l1, w)) = second else { return Ok(p0) };
    let p1 = sample(&t.levels[l1], filter)?;
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        *o = p0[c] * (1.0 - w) + p1[c] * w;
    }
    Ok(out)
}

/// A 2D (or 2D-array) look-up at (s, t) of layer `layer` under sampler
/// `sp`: each face of a cube texture is a layer, in [`Face`] order; a 2D
/// texture has layer 0. The layer is clamped to the texture's, as GL does.
/// Non-seamless by construction: the wrap modes apply within the layer.
pub fn sample_layer(t: &Texture, sp: &Sampler, layer: usize, s: f32, tc: f32, lod: Lod) -> Result<[f32; 4], Refusal> {
    sample_layer_offset(t, sp, layer, s, tc, lod, [0, 0])
}

/// [`sample_layer`] with a constant texel offset (`textureOffset` and its
/// forms; the V3D's TMU P2 Offset S / T): GL ES 3.0 3.8.10 adds the offset
/// to the texel coordinates (u, v) = (s w, t h) of the level sampled,
/// before the wrap, so each level read is sampled at s + offset_s / w of
/// that level, under the sampler's filter and wrap modes ([`over_levels`]).
/// A zero offset is [`sample_layer`] as it always was.
pub fn sample_layer_offset(
    t: &Texture,
    sp: &Sampler,
    layer: usize,
    s: f32,
    tc: f32,
    lod: Lod,
    offset: [i32; 2],
) -> Result<[f32; 4], Refusal> {
    let faces = t.levels.first().map_or(0, |l| l.faces.len());
    if faces == 0 {
        return Err(Refusal::NoLevel);
    }
    let layer = layer.min(faces - 1);
    over_levels(t, sp, lod, |l, filter| {
        if offset == [0, 0] {
            return sample_face(l, layer, filter, sp, s, tc);
        }
        let (s, tc) = (s + offset[0] as f32 / l.width as f32, tc + offset[1] as f32 / l.height as f32);
        sample_face(l, layer, filter, sp, s, tc)
    })
}

/// The layer a 2D-array look-up at coordinate `z` reads on `layers` images:
/// GL ES 3.0 3.8.9's clamp(floor(z + 1/2), 0, d - 1), the nearest layer (the
/// V3D's TMUI is ASSUMED to take the float coordinate and round it so,
/// `v3d::sim`'s module doc). Refused when `z` is not finite.
pub fn layer_of(z: f32, layers: usize) -> Result<usize, Refusal> {
    if !z.is_finite() {
        return Err(Refusal::LayerNotFinite);
    }
    if layers == 0 {
        return Err(Refusal::NoLevel);
    }
    Ok(((z + 0.5).floor().max(0.0) as usize).min(layers - 1))
}

/// A 3D look-up at (s, t, r) under sampler `sp`: on each level read
/// ([`over_levels`]; a level's images are its slices, so its depth is
/// `faces.len()`, minified with the level), the slice NEAREST picks along r
/// with CLAMP_TO_EDGE (w = r d plus `offset[2]`, floor, clamped: GL ES 3.0
/// 3.8.10's equation for the third coordinate, the offset added before
/// the wrap), then [`sample_face`] on that slice with the sampler's filter
/// and wrap modes for s and t (and the offset's S and T, as
/// [`sample_layer_offset`] adds them). Only NEAREST is modelled along r: a
/// LINEAR filter (a blend of two slices) is refused by name, and so is a w
/// within 1e-3 of a slice boundary inside the volume, as a texel edge is.
/// The wrap mode along r is CLAMP_TO_EDGE always (the only one the callers
/// bind).
pub fn sample_3d(
    t: &Texture,
    sp: &Sampler,
    s: f32,
    tc: f32,
    r: f32,
    lod: Lod,
    offset: [i32; 3],
) -> Result<[f32; 4], Refusal> {
    if !r.is_finite() {
        return Err(Refusal::NotFinite);
    }
    over_levels(t, sp, lod, |l, filter| {
        if filter == Filter::Linear {
            return Err(Refusal::VolumeLinear);
        }
        let depth = l.faces.len();
        if depth == 0 {
            return Err(Refusal::NoLevel);
        }
        let w = r * depth as f32 + offset[2] as f32;
        if w > 0.0 && w < depth as f32 && (w - w.round()).abs() < 1e-3 {
            return Err(Refusal::TexelEdge(r));
        }
        let slice = (floor_i(w).max(0) as usize).min(depth - 1);
        let (s, tc) = (s + offset[0] as f32 / l.width as f32, tc + offset[1] as f32 / l.height as f32);
        sample_face(l, slice, filter, sp, s, tc)
    })
}

/// The texel `texelFetch` reads: texel (i, j) of image `k` of `level` (a 2D
/// array's layer, a 3D texture's slice; 0 for a 2D texture or a cube's +X
/// face), no filter, no wrap, whatever the sampler. GL ES 3.0 8.8 leaves a
/// fetch outside the level, its images or the chain undefined, and what a
/// TMU returns there is not measured: each is refused by name.
pub fn fetch(t: &Texture, level: i32, i: i32, j: i32, k: i32) -> Result<[f32; 4], Refusal> {
    let n = t.levels.len();
    if level < 0 || level as usize >= n {
        return Err(Refusal::FetchLevel(level, n));
    }
    let l = &t.levels[level as usize];
    if i < 0 || j < 0 || i >= l.width as i32 || j >= l.height as i32 {
        return Err(Refusal::FetchTexel { i, j, level, width: l.width, height: l.height });
    }
    let images = l.faces.len();
    if k < 0 || k as usize >= images {
        return Err(Refusal::FetchLayer(k, images));
    }
    Ok(l.faces[k as usize][(j as u32 * l.width + i as u32) as usize])
}

/// A cube look-up at direction (x, y, z) under sampler `sp`, SEAMLESS (the
/// V3D's only cube filtering, module doc): the face and (s, t) of the
/// direction, then NEAREST on that face or a LINEAR blend whose taps past
/// the face's edges read the neighbouring faces ([`seamless_texel`]); the
/// wrap modes are ignored. Returns the face selected. Refused as
/// [`sample_cube`] refuses: a direction near a face edge (the face is a tie
/// rule's), a NEAREST coordinate on a texel edge.
pub fn sample_cube_filtered(
    t: &Texture,
    sp: &Sampler,
    x: f32,
    y: f32,
    z: f32,
    lod: Lod,
) -> Result<([f32; 4], Face), Refusal> {
    sample_cube_with(t, sp, x, y, z, lod, |v| v)
}

/// The face a cube look-up at direction (x, y, z) selects and its (s, t) on
/// that face, in [0, 1]: what [`sample_cube_filtered`] and
/// [`sample_cube_compare`] project to before they filter. Refused as
/// [`sample_cube`] refuses.
pub fn cube_face_coords(x: f32, y: f32, z: f32) -> Result<(Face, f32, f32), Refusal> {
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
    Ok((face, s, tc))
}

/// [`sample_cube_filtered`] with `tap` applied to every texel read before
/// the blend (and before a corner tap's mean).
fn sample_cube_with(
    t: &Texture,
    sp: &Sampler,
    x: f32,
    y: f32,
    z: f32,
    lod: Lod,
    tap: impl Fn([f32; 4]) -> [f32; 4],
) -> Result<([f32; 4], Face), Refusal> {
    let (face, s, tc) = cube_face_coords(x, y, z)?;
    let v = over_levels(t, sp, lod, |l, filter| match filter {
        Filter::Nearest => {
            let (i, j) = (nearest_index(s, l.width)?, nearest_index(tc, l.height)?);
            Ok(tap(l.faces[face.index()][(j * l.width + i) as usize]))
        }
        Filter::Linear => {
            let n = l.width;
            let (u, v) = (s * n as f32, tc * n as f32);
            let (x0, y0) = (floor_i(u - 0.5), floor_i(v - 0.5));
            let (a, b) = (u - 0.5 - (x0 as f32), v - 0.5 - (y0 as f32));
            let taps = [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)]
                .map(|(i, j)| seamless_texel(l, face, i, j).map(&tap));
            /* a corner tap: the mean of the other three (module doc) */
            let mut p = [[0.0f32; 4]; 4];
            for k in 0..4 {
                p[k] = match taps[k] {
                    Some(v) => v,
                    None => {
                        let mut mean = [0.0f32; 4];
                        for q in taps.iter().flatten() {
                            for c in 0..4 {
                                mean[c] += q[c] / 3.0;
                            }
                        }
                        mean
                    }
                };
            }
            Ok(bilinear(p, a, b))
        }
    })?;
    Ok((v, face))
}

/// A GL depth compare function (`GL_TEXTURE_COMPARE_FUNC`), in the order and
/// with the encoding of `GlesTextureBind::compare_op` bits 0-2 (`func -
/// GL_NEVER`; docs/notes/v3d-shadow.md (a)), which is also the V3D sampler
/// record's "Depth Compare Function" field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareFunc {
    Never,
    Less,
    Equal,
    Lequal,
    Greater,
    NotEqual,
    Gequal,
    Always,
}

impl CompareFunc {
    /// The function `compare_op` asks for, or `None` when bit 4 (no compare:
    /// the depth itself, ES 3.0.6 3.8.15.1) is set. Bit 3 (a fixed-point
    /// depth) is not part of the function: see [`compare_op_clamps_ref`].
    pub fn from_compare_op(compare_op: u8) -> Option<CompareFunc> {
        if compare_op & 16 != 0 {
            return None;
        }
        Some(match compare_op & 7 {
            0 => CompareFunc::Never,
            1 => CompareFunc::Less,
            2 => CompareFunc::Equal,
            3 => CompareFunc::Lequal,
            4 => CompareFunc::Greater,
            5 => CompareFunc::NotEqual,
            6 => CompareFunc::Gequal,
            _ => CompareFunc::Always,
        })
    }

    /// `reference OP depth`: GL_LESS passes when the reference is less than
    /// the texel's depth (ES 3.0.6 3.8.15.1).
    pub fn passes(self, reference: f32, depth: f32) -> bool {
        match self {
            CompareFunc::Never => false,
            CompareFunc::Less => reference < depth,
            CompareFunc::Equal => reference == depth,
            CompareFunc::Lequal => reference <= depth,
            CompareFunc::Greater => reference > depth,
            CompareFunc::NotEqual => reference != depth,
            CompareFunc::Gequal => reference >= depth,
            CompareFunc::Always => true,
        }
    }
}

/// `compare_op` bit 3: a fixed-point depth, whose reference is clamped to
/// [0, 1] (ES 3.0.6 3.8.15.1); the `clamp_ref` of [`Compare`].
pub fn compare_op_clamps_ref(compare_op: u8) -> bool {
    compare_op & 8 != 0
}

/// One shadow look-up's compare: the function, the reference value
/// (`Dref`, the coordinate's last lane) and whether the reference is clamped
/// to [0, 1] first (a fixed-point depth format; the caller knows the format,
/// this model has no format table). The texel is taken as it is: a
/// fixed-point depth texel is in range already.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Compare {
    pub func: CompareFunc,
    pub dref: f32,
    pub clamp_ref: bool,
}

impl Compare {
    /// 1.0 when the reference passes against the texel's R channel (the
    /// depth: the GL core keeps depth as R32F), else 0.0.
    pub fn texel(&self, texel: [f32; 4]) -> f32 {
        let r = if self.clamp_ref { self.dref.clamp(0.0, 1.0) } else { self.dref };
        if self.func.passes(r, texel[0]) {
            1.0
        } else {
            0.0
        }
    }

    /// The texel as the filters blend it: the compare result in R, (0, 0, 1)
    /// beside it, so the blend of the taps is the blend of the compares.
    fn tap(&self, texel: [f32; 4]) -> [f32; 4] {
        [self.texel(texel), 0.0, 0.0, 1.0]
    }
}

/// [`sample_face`] for a shadow look-up: compare every texel the filter
/// reads (NEAREST: the one texel; LINEAR: each of the four taps, wrapped as
/// [`sample_face`] wraps them), then blend the results with the bilinear
/// weights (percentage-closer, ES 3.0.6 3.8.15.1; docs/notes/v3d-shadow.md
/// (c)). Returns the compare result, 0.0..=1.0, one f32 as `TexLookup` in
/// the evaluator holds a shadow value. Refuses as [`sample_face`].
pub fn sample_face_compare(
    l: &Level,
    face: usize,
    filter: Filter,
    sp: &Sampler,
    cmp: &Compare,
    s: f32,
    t: f32,
) -> Result<f32, Refusal> {
    sample_face_with(l, face, filter, sp, s, t, |v| cmp.tap(v)).map(|v| v[0])
}

/// [`sample_layer`] for a shadow look-up: the level(s) [`levels`] selects,
/// [`sample_face_compare`] on each, a mipmap-LINEAR pair blended by its
/// weight.
pub fn sample_layer_compare(
    t: &Texture,
    sp: &Sampler,
    cmp: &Compare,
    layer: usize,
    s: f32,
    tc: f32,
    lod: Lod,
) -> Result<f32, Refusal> {
    let faces = t.levels.first().map_or(0, |l| l.faces.len());
    if faces == 0 {
        return Err(Refusal::NoLevel);
    }
    let layer = layer.min(faces - 1);
    over_levels(t, sp, lod, |l, filter| sample_face_with(l, layer, filter, sp, s, tc, |v| cmp.tap(v))).map(|v| v[0])
}

/// [`sample_layer_compare`] with a constant texel offset
/// (`textureOffset` on a shadow sampler, the TMU's P2 Offset S / T), added
/// to the texel coordinates of each level before the wrap as
/// [`sample_layer_offset`] adds it; a zero offset is [`sample_layer_compare`].
#[allow(clippy::too_many_arguments)]
pub fn sample_layer_offset_compare(
    t: &Texture,
    sp: &Sampler,
    cmp: &Compare,
    layer: usize,
    s: f32,
    tc: f32,
    lod: Lod,
    offset: [i32; 2],
) -> Result<f32, Refusal> {
    let faces = t.levels.first().map_or(0, |l| l.faces.len());
    if faces == 0 {
        return Err(Refusal::NoLevel);
    }
    let layer = layer.min(faces - 1);
    over_levels(t, sp, lod, |l, filter| {
        let (s, tc) = (s + offset[0] as f32 / l.width as f32, tc + offset[1] as f32 / l.height as f32);
        sample_face_with(l, layer, filter, sp, s, tc, |v| cmp.tap(v))
    })
    .map(|v| v[0])
}

/// [`sample_layer_compare`] on layer 0: a `sampler2DShadow` look-up.
pub fn sample_2d_compare(t: &Texture, sp: &Sampler, cmp: &Compare, s: f32, tc: f32, lod: Lod) -> Result<f32, Refusal> {
    sample_layer_compare(t, sp, cmp, 0, s, tc, lod)
}

/// [`sample_cube_filtered`] for a shadow look-up (`samplerCubeShadow`): the
/// seamless taps are compared first, a corner tap being the mean of the
/// other three compares. Returns the compare result and the face selected;
/// [`cube_face_coords`] gives the face (s, t) the taps are taken around.
pub fn sample_cube_compare(
    t: &Texture,
    sp: &Sampler,
    cmp: &Compare,
    x: f32,
    y: f32,
    z: f32,
    lod: Lod,
) -> Result<(f32, Face), Refusal> {
    sample_cube_with(t, sp, x, y, z, lod, |v| cmp.tap(v)).map(|(v, f)| (v[0], f))
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

    /// `wrap_index` is tcuTexture.cpp's `wrap` (tfr_wrap): the three modes
    /// on a side of 4, from -5 to 8.
    #[test]
    fn the_wrap_modes_are_deqps() {
        let row = |m: Wrap| -> Vec<u32> { (-5..9).map(|c| wrap_index(m, c, 4)).collect() };
        assert_eq!(row(Wrap::ClampToEdge), [0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 3, 3, 3]);
        assert_eq!(row(Wrap::Repeat), [3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0]);
        assert_eq!(row(Wrap::MirroredRepeat), [3, 3, 2, 1, 0, 0, 1, 2, 3, 3, 2, 1, 0, 0]);
    }

    /// A 4x4 cube: face +X's texel (i, j) is (1, i/4, j/4, 1), -Z (across
    /// +X's s = 1 edge) is flat (0, 0, 1, 1), the rest flat (f/8).
    fn edge_cube(min: MinFilter) -> Texture {
        let n = 4usize;
        let faces: Vec<Vec<[f32; 4]>> = (0..6)
            .map(|f| match f {
                0 => (0..n * n).map(|k| [1.0, (k % n) as f32 / 4.0, (k / n) as f32 / 4.0, 1.0]).collect(),
                5 => vec![[0.0, 0.0, 1.0, 1.0]; n * n],
                _ => vec![[f as f32 / 8.0; 4]; n * n],
            })
            .collect();
        Texture { cube: true, min, levels: vec![Level { width: 4, height: 4, faces }] }
    }

    /// A seamless tap past +X's s = 1 edge reads -Z's s = 0 column at the
    /// same row; past its s = 0 edge, +Z's s = 1 column; past t = 0, +Y;
    /// past a corner, nothing. `face_direction` inverts `project_to_face`.
    #[test]
    fn a_seamless_tap_past_an_edge_reads_the_neighbouring_face() {
        let t = edge_cube(MinFilter::Linear);
        let l = &t.levels[0];
        for j in 0..4 {
            assert_eq!(seamless_texel(l, Face::PosX, 4, j), Some([0.0, 0.0, 1.0, 1.0]), "row {j} past s = 1: -Z");
            assert_eq!(seamless_texel(l, Face::PosX, -1, j), Some([0.5; 4]), "row {j} past s = 0: +Z (4/8)");
            assert_eq!(seamless_texel(l, Face::PosX, j, -1), Some([0.25; 4]), "column {j} past t = 0: +Y (2/8)");
            assert_eq!(seamless_texel(l, Face::PosX, j, 4), Some([0.375; 4]), "column {j} past t = 1: -Y (3/8)");
        }
        assert_eq!(seamless_texel(l, Face::PosX, 4, 4), None, "a corner");
        assert_eq!(seamless_texel(l, Face::PosX, 2, 1), Some([1.0, 0.5, 0.25, 1.0]), "inside");
        /* -Z's tap past ITS s = 0 edge is +X's column 3, row kept */
        assert_eq!(seamless_texel(l, Face::NegZ, -1, 2), Some([1.0, 0.75, 0.5, 1.0]));
        for face in Face::ALL {
            for (s, tc) in [(0.1f32, 0.7f32), (0.9, 0.2), (0.5, 0.5)] {
                let (x, y, z) = face_direction(face, 2.0 * s - 1.0, 2.0 * tc - 1.0);
                assert_eq!(select_cube_face(x, y, z), face);
                let (s2, t2) = project_to_face(face, x, y, z);
                assert!((s2 - s).abs() < 1e-6 && (t2 - tc).abs() < 1e-6, "{face:?} ({s}, {tc}) -> ({s2}, {t2})");
            }
        }
    }

    /// The direction (1, 0, -0.875) on the 4x4 cube: +X, u = 3.75, v = 2.
    /// A seamless LINEAR look-up blends +X's column 3 with -Z's edge column
    /// 3:1 (and rows 1, 2 equally); the same face coordinates as a layer
    /// look-up wrap within the face: CLAMP and MIRROR repeat column 3,
    /// REPEAT takes column 0. NEAREST is the texel (3, 2) either way.
    #[test]
    fn a_cube_lookup_is_seamless_and_a_layer_lookup_wraps_within_the_face() {
        let sp = |mag: Filter, min: MinFilter, wrap: Wrap| Sampler { mag, min, wrap_s: wrap, wrap_t: wrap };
        let t = edge_cube(MinFilter::Linear);
        let (x, y, z) = (1.0f32, 0.0f32, -0.875f32);
        let (s, tc) = project_to_face(Face::PosX, x, y, z);
        assert_eq!((s, tc), (0.9375, 0.5));
        for wrap in [Wrap::ClampToEdge, Wrap::Repeat, Wrap::MirroredRepeat] {
            let lin = sp(Filter::Linear, MinFilter::Linear, wrap);
            let (v, face) = sample_cube_filtered(&t, &lin, x, y, z, Lod::Explicit(0.0)).unwrap();
            assert_eq!((v, face), ([0.75, 0.5625, 0.53125, 1.0], Face::PosX), "{wrap:?}");
            let want = match wrap {
                Wrap::Repeat => [1.0, 0.5625, 0.375, 1.0],
                _ => [1.0, 0.75, 0.375, 1.0],
            };
            assert_eq!(sample_layer(&t, &lin, 0, s, tc, Lod::Explicit(0.0)), Ok(want), "{wrap:?}");
            /* NEAREST at t = 0.5 would sit on the edge of rows 1 and 2
             * (refused); y = -0.25 puts it at t = 0.625, inside row 2 */
            let near = sp(Filter::Nearest, MinFilter::Nearest, wrap);
            assert_eq!(sample_cube_filtered(&t, &near, x, y, z, Lod::Explicit(0.0)), Err(Refusal::TexelEdge(0.5)));
            let (v, _) = sample_cube_filtered(&t, &near, x, -0.25, z, Lod::Explicit(0.0)).unwrap();
            assert_eq!(v, [1.0, 0.75, 0.5, 1.0], "{wrap:?}");
            assert_eq!(sample_layer(&t, &near, 0, s, 0.625, Lod::Explicit(0.0)), Ok([1.0, 0.75, 0.5, 1.0]));
        }
        /* a REPEAT NEAREST coordinate on the image's own edge is a texel
         * edge (the next texel is column 0); under CLAMP it is not */
        let rep = sp(Filter::Nearest, MinFilter::Nearest, Wrap::Repeat);
        assert_eq!(sample_layer(&t, &rep, 0, 1.0, 0.625, Lod::Explicit(0.0)), Err(Refusal::TexelEdge(1.0)));
        let clamp = sp(Filter::Nearest, MinFilter::Nearest, Wrap::ClampToEdge);
        assert_eq!(sample_layer(&t, &clamp, 0, 1.0, 0.625, Lod::Explicit(0.0)), Ok([1.0, 0.75, 0.5, 1.0]));
        /* the layer is clamped to the faces there are; a 2D texture has one */
        assert_eq!(sample_layer(&t, &clamp, 9, 0.3, 0.3, Lod::Explicit(0.0)), Ok([0.0, 0.0, 1.0, 1.0]), "-Z");
        let flat = Texture::flat(2, 2, vec![[0.5; 4]; 4]);
        assert_eq!(sample_layer(&flat, &clamp, 3, 0.3, 0.3, Lod::Implicit), Ok([0.5; 4]));
    }

    /// The level rule under a sampler: a filter without mipmaps reads
    /// level 0; a mipmap-NEAREST one picks dEQP's level; a mipmap-LINEAR
    /// one blends floor(lod) and the next by frac(lod), and reads one
    /// level at an integer lod or past the chain's end; magnified (lod <=
    /// 0) is level 0 with the magnification filter. The NEAREST entry point
    /// refuses a blend by name.
    #[test]
    fn a_mipmap_linear_filter_blends_two_levels() {
        let lv = |w: u32, v: f32| Level { width: w, height: w, faces: vec![vec![[v; 4]; (w * w) as usize]] };
        let t = Texture {
            cube: false,
            min: MinFilter::LinearMipmapLinear,
            levels: vec![lv(4, 0.0), lv(2, 1.0), lv(1, 2.0)],
        };
        let sp =
            |min: MinFilter| Sampler { mag: Filter::Linear, min, wrap_s: Wrap::ClampToEdge, wrap_t: Wrap::ClampToEdge };
        /* (0.3, 0.3) is inside a texel on every level (0.5 would be a texel
         * edge on the 2x2 level under NEAREST) */
        let at = |min: MinFilter, lod: f32| sample_layer(&t, &sp(min), 0, 0.3, 0.3, Lod::Explicit(lod)).unwrap()[0];
        assert_eq!(at(MinFilter::LinearMipmapLinear, 0.25), 0.25);
        assert_eq!(at(MinFilter::NearestMipmapLinear, 1.5), 1.5);
        assert_eq!(at(MinFilter::LinearMipmapLinear, 1.0), 1.0, "an integer lod: one level");
        assert_eq!(at(MinFilter::LinearMipmapLinear, 5.0), 2.0, "past the chain: the last level");
        assert_eq!(at(MinFilter::LinearMipmapLinear, -1.0), 0.0, "magnified");
        assert_eq!(at(MinFilter::Linear, 1.5), 0.0, "no mipmaps: level 0");
        assert_eq!(at(MinFilter::LinearMipmapNearest, 0.9), 1.0, "ceil(0.9 + 0.5) - 1");
        assert_eq!(levels(&t, MinFilter::LinearMipmapLinear, Lod::Explicit(0.25)), Ok((0, Some((1, 0.25)))));
        assert_eq!(level(&t, Lod::Explicit(0.25)), Err(Refusal::LevelBlend(0.25)));
        assert_eq!(levels(&t, MinFilter::NearestMipmapNearest, Lod::Explicit(1.5)), Err(Refusal::LevelBoundary(1.5)));
        assert_eq!(MinFilter::NearestMipmapLinear.within_level(), Filter::Nearest);
        assert_eq!(MinFilter::LinearMipmapNearest.within_level(), Filter::Linear);
    }

    fn depth_2x2(d: [f32; 4]) -> Texture {
        Texture::flat(2, 2, d.iter().map(|&v| [v, 0.0, 0.0, 1.0]).collect())
    }

    fn cmp(func: CompareFunc, dref: f32) -> Compare {
        Compare { func, dref, clamp_ref: false }
    }

    fn lin(wrap: Wrap) -> Sampler {
        Sampler { mag: Filter::Linear, min: MinFilter::Linear, wrap_s: wrap, wrap_t: wrap }
    }

    #[test]
    fn compare_op_gives_the_function_or_none() {
        use CompareFunc::*;
        let all = [Never, Less, Equal, Lequal, Greater, NotEqual, Gequal, Always];
        for (op, f) in all.iter().enumerate() {
            assert_eq!(CompareFunc::from_compare_op(op as u8), Some(*f));
            assert_eq!(CompareFunc::from_compare_op(op as u8 | 8), Some(*f), "bit 3 is the clamp, not the function");
            assert_eq!(CompareFunc::from_compare_op(op as u8 | 16), None, "bit 4: no compare");
        }
        assert_eq!(CompareFunc::from_compare_op(16), None);
        assert!(compare_op_clamps_ref(8 | 1) && !compare_op_clamps_ref(1));
    }

    /// Every function on a texel of depth 0.5: the reference below, equal
    /// and above.
    #[test]
    fn every_function_compares_the_reference_with_the_texel() {
        use CompareFunc::*;
        let texel = [0.5, 0.9, 0.9, 0.9];
        let at = |f: CompareFunc, r: f32| cmp(f, r).texel(texel);
        let expect = [
            (Never, [0.0, 0.0, 0.0]),
            (Less, [1.0, 0.0, 0.0]),
            (Equal, [0.0, 1.0, 0.0]),
            (Lequal, [1.0, 1.0, 0.0]),
            (Greater, [0.0, 0.0, 1.0]),
            (NotEqual, [1.0, 0.0, 1.0]),
            (Gequal, [0.0, 1.0, 1.0]),
            (Always, [1.0, 1.0, 1.0]),
        ];
        for (f, want) in expect {
            assert_eq!([at(f, 0.25), at(f, 0.5), at(f, 0.75)], want, "{f:?}");
        }
    }

    #[test]
    fn the_reference_is_clamped_only_when_asked() {
        let hi = |clamp_ref| Compare { func: CompareFunc::Equal, dref: 3.0, clamp_ref };
        assert_eq!(hi(true).texel([1.0, 0.0, 0.0, 1.0]), 1.0, "3.0 clamps to the texel's 1.0");
        assert_eq!(hi(false).texel([1.0, 0.0, 0.0, 1.0]), 0.0);
        let lo = |clamp_ref| Compare { func: CompareFunc::Equal, dref: -2.0, clamp_ref };
        assert_eq!(lo(true).texel([0.0; 4]), 1.0);
        assert_eq!(lo(false).texel([0.0; 4]), 0.0);
    }

    #[test]
    fn nearest_returns_the_compare_of_the_one_texel() {
        let t = depth_2x2([0.2, 0.4, 0.6, 0.8]);
        let sp = Sampler::NEAREST_CLAMP;
        let at = |r: f32, s: f32, tc: f32| {
            sample_2d_compare(&t, &sp, &cmp(CompareFunc::Less, r), s, tc, Lod::Explicit(0.0)).unwrap()
        };
        /* texel (1, 0) holds 0.4; (0, 1) holds 0.6 */
        assert_eq!(at(0.3, 0.75, 0.25), 1.0);
        assert_eq!(at(0.5, 0.75, 0.25), 0.0);
        assert_eq!(at(0.5, 0.25, 0.75), 1.0);
        assert_eq!(at(0.7, 0.25, 0.75), 0.0);
    }

    /// Taps 0.0, 1.0, 1.0, 0.0 at weights (a, b) = (.25, .25), reference 0.6
    /// under LESS: the two 1.0 taps pass, so the compare-then-filter result is
    /// the weighted mean of the compares (0.375); the compare of the blended
    /// depth (0.375, which fails) is 0.
    #[test]
    fn linear_compares_each_tap_then_filters() {
        let t = depth_2x2([0.0, 1.0, 1.0, 0.0]);
        let sp = lin(Wrap::ClampToEdge);
        let c = cmp(CompareFunc::Less, 0.6);
        let (s, tc) = (0.375, 0.375);
        let got = sample_2d_compare(&t, &sp, &c, s, tc, Lod::Explicit(0.0)).unwrap();
        let (a, b) = (0.25f32, 0.25f32);
        let want = a * (1.0 - b) + (1.0 - a) * b;
        assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        let filtered = sample_layer(&t, &sp, 0, s, tc, Lod::Explicit(0.0)).unwrap()[0];
        let after = c.texel([filtered, 0.0, 0.0, 1.0]);
        assert_eq!(after, 0.0, "the blended depth {filtered} fails");
        assert_ne!(got, after, "percentage-closer differs from compare-after-filter");
    }

    #[test]
    fn a_texel_centre_has_no_blend() {
        let t = depth_2x2([0.1, 0.9, 0.9, 0.9]);
        let sp = lin(Wrap::ClampToEdge);
        let c = cmp(CompareFunc::Lequal, 0.5);
        let at = |s: f32, tc: f32| sample_2d_compare(&t, &sp, &c, s, tc, Lod::Explicit(0.0)).unwrap();
        assert_eq!(at(0.25, 0.25), 0.0, "0.5 <= 0.1 fails");
        assert_eq!(at(0.75, 0.25), 1.0, "0.5 <= 0.9 passes");
    }

    #[test]
    fn the_wrap_modes_apply_to_the_compared_taps() {
        /* column 0 holds 0.9 (LESS 0.5 passes), column 1 holds 0.1 (fails) */
        let t = depth_2x2([0.9, 0.1, 0.9, 0.1]);
        let c = cmp(CompareFunc::Less, 0.5);
        /* s = 0 is the left edge: the taps are columns -1 and 0, equal weight */
        let at = |wrap: Wrap| sample_2d_compare(&t, &lin(wrap), &c, 0.0, 0.25, Lod::Explicit(0.0)).unwrap();
        assert_eq!(at(Wrap::ClampToEdge), 1.0, "both taps read column 0");
        assert_eq!(at(Wrap::Repeat), 0.5, "column -1 wraps to column 1");
    }

    #[test]
    fn a_mipmapped_texture_compares_on_the_level_lod_picks() {
        let lv =
            |w: u32, v: f32| Level { width: w, height: w, faces: vec![vec![[v, 0.0, 0.0, 1.0]; (w * w) as usize]] };
        let t = Texture {
            cube: false,
            min: MinFilter::LinearMipmapLinear,
            levels: vec![lv(4, 0.2), lv(2, 0.8), lv(1, 0.8)],
        };
        let sp = Sampler { mag: Filter::Linear, min: MinFilter::LinearMipmapLinear, ..lin(Wrap::ClampToEdge) };
        let c = cmp(CompareFunc::Less, 0.5);
        let at = |lod: f32| sample_2d_compare(&t, &sp, &c, 0.3, 0.3, Lod::Explicit(lod)).unwrap();
        assert_eq!(at(0.0), 0.0, "level 0 holds 0.2");
        assert_eq!(at(1.0), 1.0, "level 1 holds 0.8");
        assert_eq!(at(0.25), 0.25, "the compares of the two levels blend");
    }

    #[test]
    fn a_layer_compare_reads_its_layer() {
        let l = Level { width: 1, height: 1, faces: vec![vec![[0.2; 4]], vec![[0.8; 4]]] };
        let t = Texture { cube: false, min: MinFilter::Nearest, levels: vec![l] };
        let sp = Sampler::NEAREST_CLAMP;
        let c = cmp(CompareFunc::Less, 0.5);
        let at = |layer| sample_layer_compare(&t, &sp, &c, layer, 0.3, 0.3, Lod::Explicit(0.0)).unwrap();
        assert_eq!((at(0), at(1), at(7)), (0.0, 1.0, 1.0), "a layer past the last is clamped");
    }

    /// The direction of the seamless test above ((1, 0, -0.875): +X column 3
    /// with R = 1, blended 3:1 with -Z's edge column with R = 0): LESS 0.5
    /// compares each tap, so the answer is 0.75, where the blended depth 0.75
    /// would pass outright.
    #[test]
    fn a_cube_compare_filters_the_compares_across_the_seams() {
        let t = edge_cube(MinFilter::Linear);
        let sp = lin(Wrap::ClampToEdge);
        let dir = (1.0, 0.0, -0.875);
        let at = |c: Compare| sample_cube_compare(&t, &sp, &c, dir.0, dir.1, dir.2, Lod::Explicit(0.0)).unwrap();
        let (v, face) = at(cmp(CompareFunc::Less, 0.5));
        assert_eq!(face, Face::PosX);
        assert!((v - 0.75).abs() < 1e-6, "{v}");
        let (plain, _) = sample_cube_filtered(&t, &sp, dir.0, dir.1, dir.2, Lod::Explicit(0.0)).unwrap();
        assert!((plain[0] - 0.75).abs() < 1e-6 && cmp(CompareFunc::Less, 0.5).texel(plain) == 1.0);
        assert_eq!(at(cmp(CompareFunc::Always, 0.0)).0, 1.0);
        assert_eq!(at(cmp(CompareFunc::Never, 0.0)).0, 0.0);
    }

    /// The compare of a cube look-up against a per-corner computation that
    /// names each of the four taps by hand (4x4 faces, every texel its own
    /// depth): face interior, across +X's s = 1 edge (-Z column 0, same
    /// row), and at +X's (s, t) = (1, 1) corner, whose missing tap is the
    /// mean of the other three compares (-Z (0, 3), -Y (3, 3), +X (3, 3)).
    #[test]
    fn a_cube_compare_matches_the_per_corner_computation() {
        let depth = |f: usize, i: usize, j: usize| (f * 16 + j * 4 + i) as f32 / 96.0 + 0.003;
        let faces: Vec<Vec<[f32; 4]>> =
            (0..6).map(|f| (0..16).map(|k| [depth(f, k % 4, k / 4), 0.0, 0.0, 1.0]).collect()).collect();
        let t = Texture { cube: true, min: MinFilter::Linear, levels: vec![Level { width: 4, height: 4, faces }] };
        let sp = lin(Wrap::ClampToEdge);
        let (px, nz, ny) = (Face::PosX.index(), Face::NegZ.index(), Face::NegY.index());
        type Tap = Option<(usize, usize, usize)>;
        type Case = ((f32, f32, f32), f32, f32, [Tap; 4], f32, f32);
        type Pass = fn(f32, f32) -> bool;
        let cases: [Case; 3] = [
            (
                (1.0, -0.125, 0.125),
                0.4375,
                0.5625,
                [Some((px, 1, 1)), Some((px, 2, 1)), Some((px, 1, 2)), Some((px, 2, 2))],
                0.25,
                0.75,
            ),
            (
                (1.0, 0.0, -0.875),
                0.9375,
                0.5,
                [Some((px, 3, 1)), Some((nz, 0, 1)), Some((px, 3, 2)), Some((nz, 0, 2))],
                0.25,
                0.5,
            ),
            (
                (1.0, -0.875, -0.875),
                0.9375,
                0.9375,
                [Some((px, 3, 3)), Some((nz, 0, 3)), Some((ny, 3, 3)), None],
                0.25,
                0.25,
            ),
        ];
        let funcs: [(CompareFunc, Pass); 4] = [
            (CompareFunc::Lequal, |r, d| r <= d),
            (CompareFunc::Greater, |r, d| r > d),
            (CompareFunc::Less, |r, d| r < d),
            (CompareFunc::Gequal, |r, d| r >= d),
        ];
        for (dir, s, tc, taps, a, b) in cases {
            let (face, s2, t2) = cube_face_coords(dir.0, dir.1, dir.2).unwrap();
            assert_eq!(face, Face::PosX);
            assert!((s2 - s).abs() < 1e-6 && (t2 - tc).abs() < 1e-6, "{dir:?}: ({s2}, {t2})");
            for (func, pass) in funcs {
                for dref in [0.15f32, 0.3, 0.45] {
                    let one = |f: usize, i: usize, j: usize| if pass(dref, depth(f, i, j)) { 1.0f32 } else { 0.0 };
                    let known: Vec<f32> = taps.iter().flatten().map(|&(f, i, j)| one(f, i, j)).collect();
                    let mean = known.iter().sum::<f32>() / known.len() as f32;
                    let w = [(1.0 - a) * (1.0 - b), a * (1.0 - b), (1.0 - a) * b, a * b];
                    let want: f32 = (0..4).map(|k| w[k] * taps[k].map_or(mean, |(f, i, j)| one(f, i, j))).sum();
                    let c = cmp(func, dref);
                    let (got, gface) =
                        sample_cube_compare(&t, &sp, &c, dir.0, dir.1, dir.2, Lod::Explicit(0.0)).unwrap();
                    assert_eq!(gface, Face::PosX);
                    assert!((got - want).abs() < 1e-6, "{dir:?} {func:?} {dref}: {got} vs {want}");
                }
            }
        }
    }
}
