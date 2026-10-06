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
    if !(s.is_finite() && t.is_finite()) {
        return Err(Refusal::NotFinite);
    }
    let texels = &l.faces[face];
    let (w, h) = (l.width, l.height);
    let at = |i: u32, j: u32| texels[(j * w + i) as usize];
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
    let faces = t.levels.first().map_or(0, |l| l.faces.len());
    if faces == 0 {
        return Err(Refusal::NoLevel);
    }
    let layer = layer.min(faces - 1);
    over_levels(t, sp, lod, |l, filter| sample_face(l, layer, filter, sp, s, tc))
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
    let v = over_levels(t, sp, lod, |l, filter| match filter {
        Filter::Nearest => {
            let (i, j) = (nearest_index(s, l.width)?, nearest_index(tc, l.height)?);
            Ok(l.faces[face.index()][(j * l.width + i) as usize])
        }
        Filter::Linear => {
            let n = l.width;
            let (u, v) = (s * n as f32, tc * n as f32);
            let (x0, y0) = (floor_i(u - 0.5), floor_i(v - 0.5));
            let (a, b) = (u - 0.5 - (x0 as f32), v - 0.5 - (y0 as f32));
            let taps =
                [(x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1)].map(|(i, j)| seamless_texel(l, face, i, j));
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
}
