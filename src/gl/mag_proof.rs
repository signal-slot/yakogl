// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A per-draw magnification proof for one sampler of a program.
//!
//! The question: over a whole draw, is every implicit-LOD fetch of a
//! sampler magnified (lambda <= c), so that a mipmap-filtered fetch reads
//! level 0 only and the texture's levels 1.. are never observed? This is
//! rule M of `docs/notes/lazy-mip-chain.md` and candidate 2 of
//! `docs/notes/terrain-mip-usage.md` ("Candidate 1, the bound").
//!
//! The module depends on `std` only and on no crate module. It has its own
//! input types: a later change converts the GL core's shader reflection and
//! draw state into them (see the handoff of the change that added it).
//!
//! The proof is conservative. [`prove_magnified`] answers
//! [`MagVerdict::ProvenMagnified`] only when all of these hold:
//!
//! - the sampler is fetched in the fragment stage only, by `texture2D`
//!   without a bias and without an explicit LOD;
//! - each fetch's coordinate is an affine function of varyings, uniforms and
//!   constants, and each varying it reads is an affine function of
//!   attributes, uniforms and constants;
//! - `gl_Position` is an affine function of attributes with a constant,
//!   positive `w` (so interpolation is linear in window space);
//! - the coordinate's gradient factors through the window position, so it
//!   is the same for every non-degenerate primitive of the draw, whatever
//!   the vertex data;
//! - the bound on rho over the whole draw is below 1 texel a pixel, with a
//!   margin ([`RHO_LIMIT`]).
//!
//! Anything else is [`MagVerdict::Unknown`] with the reason. A converter
//! that cannot express a construct (a function call, a branch, a loop, a
//! fetch under non-uniform control flow, `discard` before a fetch) must
//! encode it as [`Expr::Other`], which the proof never sees through.

use std::collections::{BTreeMap, HashMap};

/// The largest rho bound (texels a pixel) that counts as magnified.
///
/// GL ES 2.0.25 3.7.7: lambda = log2(rho); the fetch is magnified when
/// lambda <= c, and c is 0 or 0.5. rho < 1 gives lambda < 0 <= c. The margin
/// of 1/256 (lambda < -0.0056) keeps a GPU's fixed-point LOD from rounding a
/// bound just under 1 up past c.
pub const RHO_LIMIT: f64 = 1.0 - 1.0 / 256.0;

/// A shader expression, as far as the proof needs it. Vectors are lists of
/// components; a scalar is a one-component vector and broadcasts in
/// [`Expr::Add`] and [`Expr::Mul`].
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A vertex attribute, read in the vertex stage, with the width it is
    /// declared with in the shader (`attribute vec3 position` is 3).
    Attribute(String, usize),
    /// A varying, read in the fragment stage.
    Varying(String),
    /// A uniform; its value comes from [`DrawState::uniforms`].
    Uniform(String),
    /// A constant vector (or scalar).
    Const(Vec<f64>),
    /// Component-wise addition.
    Add(Box<Expr>, Box<Expr>),
    /// Component-wise multiplication.
    Mul(Box<Expr>, Box<Expr>),
    /// Matrix times vector. The matrix must be a [`Expr::Uniform`] whose
    /// value is a [`UniformValue::Mat`].
    MatVec(Box<Expr>, Box<Expr>),
    /// A swizzle or component select: the indices of the components kept.
    Swizzle(Box<Expr>, Vec<usize>),
    /// A vector constructor: the components of its arguments in order
    /// (`vec4(position, 1.0)`).
    Construct(Vec<Expr>),
    /// A texture fetch.
    Texture2D {
        sampler: String,
        coord: Box<Expr>,
        /// The fragment-stage `bias` argument, if present.
        bias: Option<Box<Expr>>,
        /// The explicit LOD (`texture2DLod`), if present.
        lod: Option<Box<Expr>>,
    },
    /// Anything else (a function call, `abs`, a division by a non-constant,
    /// a branch). The label is for the reader; the arguments are searched
    /// for fetches, never seen through.
    Other(String, Vec<Expr>),
}

/// The vertex stage: each varying's expression and `gl_Position`'s.
#[derive(Clone, Debug, PartialEq)]
pub struct VertexStage {
    pub varyings: Vec<(String, Expr)>,
    pub position: Expr,
}

/// The fragment stage: its output expressions (`gl_FragColor`, or each
/// `gl_FragData[i]`). Every fetch in them, at any depth, is a fetch.
#[derive(Clone, Debug, PartialEq)]
pub struct FragmentStage {
    pub outputs: Vec<Expr>,
}

/// A linked program, as far as the proof needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub vertex: VertexStage,
    pub fragment: FragmentStage,
}

/// A uniform's current value.
#[derive(Clone, Debug, PartialEq)]
pub enum UniformValue {
    /// A float, vec2, vec3 or vec4.
    Vec(Vec<f64>),
    /// A square matrix of size `n`, column-major (as `glUniformMatrix*fv`
    /// takes it with `transpose` false).
    Mat { n: usize, data: Vec<f64> },
}

/// Where an attribute's values come from in this draw.
#[derive(Clone, Debug, PartialEq)]
pub enum AttribSource {
    /// The component-wise min and max over the draw's vertices.
    Range { min: Vec<f64>, max: Vec<f64> },
    /// The draw's vertices (each a list of components).
    Vertices(Vec<Vec<f64>>),
}

/// The viewport in pixels (`glViewport`'s width and height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

/// The bound texture's level-0 size in texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureSize {
    pub width: u32,
    pub height: u32,
}

/// The draw's state the proof reads.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawState {
    pub viewport: Viewport,
    /// Each attribute's source. An attribute whose range is one point (the
    /// same value at every vertex, or a disabled array's current value) is a
    /// constant. Components the source leaves out take GL's defaults
    /// (0, 0, 0, 1).
    pub attributes: HashMap<String, AttribSource>,
    pub uniforms: HashMap<String, UniformValue>,
    /// The level-0 size of the texture bound to the sampler.
    pub texture: TextureSize,
}

/// Why the proof could not show the sampler magnified.
#[derive(Clone, Debug, PartialEq)]
pub enum Unknown {
    /// The program never fetches the sampler.
    NoFetch,
    /// A fetch in the vertex stage.
    VertexStageFetch,
    /// A fetch with a `bias` argument.
    Bias,
    /// A fetch with an explicit LOD.
    Lod,
    /// A fetch whose coordinate reads another fetch.
    DependentRead,
    /// A fetch whose coordinate is not affine (a product of two non-constant
    /// terms, or an [`Expr::Other`]).
    CoordNotAffine,
    /// `gl_Position` is not affine in the attributes.
    PositionNotAffine,
    /// `gl_Position.w` is not a positive constant over the draw.
    WNotConstant,
    /// The coordinate's gradient depends on the primitive (it reads an
    /// attribute direction the window position does not see).
    DependsOnPrimitive,
    /// The window position does not span two dimensions (no fragments, or a
    /// degenerate transform).
    Degenerate,
    /// An input is missing or malformed (a uniform, an attribute, a varying,
    /// a mismatched width).
    BadInput(String),
    /// The coordinate is affine, but the bound on rho is not below
    /// [`RHO_LIMIT`].
    NotMagnified { rho_bound: f64 },
}

/// The proof's answer for one sampler.
#[derive(Clone, Debug, PartialEq)]
pub enum MagVerdict {
    /// Every fetch of the sampler is magnified over the whole draw, with
    /// the largest rho bound found.
    ProvenMagnified {
        rho_bound: f64,
    },
    Unknown(Unknown),
}

impl MagVerdict {
    pub fn is_proven(&self) -> bool {
        matches!(self, MagVerdict::ProvenMagnified { .. })
    }
}

/// Proves (or not) that every fetch of `sampler` in `program` is magnified
/// over the whole draw described by `draw`.
pub fn prove_magnified(program: &Program, sampler: &str, draw: &DrawState) -> MagVerdict {
    match prove(program, sampler, draw) {
        Ok(rho_bound) => MagVerdict::ProvenMagnified { rho_bound },
        Err(why) => MagVerdict::Unknown(why),
    }
}

/// The input variable of the affine forms: one attribute component.
type Var = (String, usize);

/// An affine scalar: sum of coefficient * attribute component, plus a
/// constant.
#[derive(Clone, Debug, Default, PartialEq)]
struct Lin {
    coeffs: BTreeMap<Var, f64>,
    c: f64,
}

impl Lin {
    fn constant(c: f64) -> Lin {
        Lin { coeffs: BTreeMap::new(), c }
    }

    fn is_const(&self) -> bool {
        self.coeffs.values().all(|&k| k == 0.0)
    }

    fn add(&self, o: &Lin) -> Lin {
        let mut r = self.clone();
        for (v, k) in &o.coeffs {
            *r.coeffs.entry(v.clone()).or_insert(0.0) += k;
        }
        r.c += o.c;
        r
    }

    fn scale(&self, s: f64) -> Lin {
        Lin { coeffs: self.coeffs.iter().map(|(v, k)| (v.clone(), k * s)).collect(), c: self.c * s }
    }

    fn mul(&self, o: &Lin) -> Result<Lin, Unknown> {
        if o.is_const() {
            Ok(self.scale(o.c))
        } else if self.is_const() {
            Ok(o.scale(self.c))
        } else {
            Err(Unknown::CoordNotAffine)
        }
    }
}

type Aff = Vec<Lin>;

/// What a stage's names resolve to while building affine forms.
enum Stage<'a> {
    /// Attributes resolve through the draw's sources.
    Vertex,
    /// Varyings resolve to their vertex-stage forms.
    Fragment(&'a HashMap<String, Result<Aff, Unknown>>),
}

fn prove(program: &Program, sampler: &str, draw: &DrawState) -> Result<f64, Unknown> {
    // 1. Where the sampler is fetched.
    let mut vertex_fetches = Vec::new();
    for (_, e) in &program.vertex.varyings {
        collect_fetches(e, sampler, &mut vertex_fetches);
    }
    collect_fetches(&program.vertex.position, sampler, &mut vertex_fetches);
    if !vertex_fetches.is_empty() {
        return Err(Unknown::VertexStageFetch);
    }
    let mut fetches = Vec::new();
    for e in &program.fragment.outputs {
        collect_fetches(e, sampler, &mut fetches);
    }
    if fetches.is_empty() {
        return Err(Unknown::NoFetch);
    }
    for f in &fetches {
        if let Expr::Texture2D { bias, lod, .. } = f {
            if lod.is_some() {
                return Err(Unknown::Lod);
            }
            if bias.is_some() {
                return Err(Unknown::Bias);
            }
        }
    }

    if draw.viewport.width == 0 || draw.viewport.height == 0 {
        return Err(Unknown::Degenerate);
    }
    if draw.texture.width == 0 || draw.texture.height == 0 {
        return Err(Unknown::BadInput("texture size 0".into()));
    }

    // 2. gl_Position: affine, constant positive w.
    let pos = affine(&program.vertex.position, &Stage::Vertex, draw).map_err(|e| match e {
        Unknown::CoordNotAffine | Unknown::DependentRead => Unknown::PositionNotAffine,
        other => other,
    })?;
    if pos.len() != 4 {
        return Err(Unknown::BadInput("gl_Position is not a vec4".into()));
    }
    let w = &pos[3];
    if !w.is_const() || !w.c.is_finite() || w.c <= 0.0 {
        return Err(Unknown::WNotConstant);
    }
    // Window x, y per unit of each attribute component:
    // x_win = (x_clip / w + 1) * width / 2 + x0.
    let sx = draw.viewport.width as f64 / 2.0 / w.c;
    let sy = draw.viewport.height as f64 / 2.0 / w.c;
    let px = pos[0].scale(sx);
    let py = pos[1].scale(sy);

    // 3. The varyings' vertex-stage forms (an error is kept and reported
    //    only if a coordinate reads that varying).
    let mut varyings = HashMap::new();
    for (name, e) in &program.vertex.varyings {
        varyings.insert(name.clone(), affine(e, &Stage::Vertex, draw));
    }

    // 4. Each fetch: the coordinate's gradient in texels a window pixel.
    let mut worst = 0.0f64;
    for f in &fetches {
        let Expr::Texture2D { coord, .. } = f else { unreachable!() };
        if contains_fetch(coord) {
            return Err(Unknown::DependentRead);
        }
        let uv = affine(coord, &Stage::Fragment(&varyings), draw)?;
        if uv.len() != 2 {
            return Err(Unknown::BadInput("texture2D coordinate is not a vec2".into()));
        }
        let du = window_gradient(&uv[0], &px, &py)?;
        let dv = window_gradient(&uv[1], &px, &py)?;
        let tw = draw.texture.width as f64;
        let th = draw.texture.height as f64;
        // (du/dx, du/dy) and (dv/dx, dv/dy) in texels a pixel.
        let (ux, uy) = (du.0 * tw, du.1 * tw);
        let (vx, vy) = (dv.0 * th, dv.1 * th);
        // ES 2.0.25 3.7.7 lets an implementation approximate rho by a
        // value between max(|du|, |dv|) and |du| + |dv| per axis; take the
        // upper end, which also bounds the exact sqrt form.
        let rho = (ux.abs() + vx.abs()).max(uy.abs() + vy.abs());
        if !rho.is_finite() {
            return Err(Unknown::Degenerate);
        }
        worst = worst.max(rho);
    }
    if worst < RHO_LIMIT {
        Ok(worst)
    } else {
        Err(Unknown::NotMagnified { rho_bound: worst })
    }
}

fn collect_fetches<'e>(e: &'e Expr, sampler: &str, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::Attribute(..) | Expr::Varying(_) | Expr::Uniform(_) | Expr::Const(_) => {}
        Expr::Add(a, b) | Expr::Mul(a, b) | Expr::MatVec(a, b) => {
            collect_fetches(a, sampler, out);
            collect_fetches(b, sampler, out);
        }
        Expr::Swizzle(a, _) => collect_fetches(a, sampler, out),
        Expr::Construct(v) | Expr::Other(_, v) => {
            for a in v {
                collect_fetches(a, sampler, out);
            }
        }
        Expr::Texture2D { sampler: s, coord, bias, lod } => {
            if s == sampler {
                out.push(e);
            }
            collect_fetches(coord, sampler, out);
            if let Some(b) = bias {
                collect_fetches(b, sampler, out);
            }
            if let Some(l) = lod {
                collect_fetches(l, sampler, out);
            }
        }
    }
}

fn contains_fetch(e: &Expr) -> bool {
    match e {
        Expr::Attribute(..) | Expr::Varying(_) | Expr::Uniform(_) | Expr::Const(_) => false,
        Expr::Add(a, b) | Expr::Mul(a, b) | Expr::MatVec(a, b) => contains_fetch(a) || contains_fetch(b),
        Expr::Swizzle(a, _) => contains_fetch(a),
        Expr::Construct(v) | Expr::Other(_, v) => v.iter().any(contains_fetch),
        Expr::Texture2D { .. } => true,
    }
}

fn broadcast(a: Aff, b: Aff) -> Result<(Aff, Aff), Unknown> {
    match (a.len(), b.len()) {
        (x, y) if x == y => Ok((a, b)),
        (1, n) => Ok((vec![a[0].clone(); n], b)),
        (n, 1) => {
            let bb = vec![b[0].clone(); n];
            Ok((a, bb))
        }
        _ => Err(Unknown::BadInput("operand widths differ".into())),
    }
}

fn attribute(name: &str, width: usize, draw: &DrawState) -> Result<Aff, Unknown> {
    let src = draw.attributes.get(name).ok_or_else(|| Unknown::BadInput(format!("attribute {name} has no source")))?;
    let (min, max) = match src {
        AttribSource::Range { min, max } => {
            if min.len() != max.len() {
                return Err(Unknown::BadInput(format!("attribute {name} range widths")));
            }
            (min.clone(), max.clone())
        }
        AttribSource::Vertices(vs) => {
            let n = vs.first().map_or(0, |v| v.len());
            if n == 0 || vs.iter().any(|v| v.len() != n) {
                return Err(Unknown::BadInput(format!("attribute {name} vertex widths")));
            }
            let mut min = vec![f64::INFINITY; n];
            let mut max = vec![f64::NEG_INFINITY; n];
            for v in vs {
                for i in 0..n {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
            (min, max)
        }
    };
    if !(1..=4).contains(&width) || min.len() > 4 || min.iter().chain(&max).any(|x| !x.is_finite()) {
        return Err(Unknown::BadInput(format!("attribute {name} values")));
    }
    // The declared width; GL fills the components the array leaves out
    // with (0, 0, 0, 1).
    let defaults = [0.0, 0.0, 0.0, 1.0];
    Ok((0..width)
        .map(|i| {
            if i >= min.len() {
                Lin::constant(defaults[i])
            } else if min[i] == max[i] {
                Lin::constant(min[i])
            } else {
                let mut l = Lin::default();
                l.coeffs.insert((name.to_string(), i), 1.0);
                l
            }
        })
        .collect())
}

fn affine(e: &Expr, stage: &Stage, draw: &DrawState) -> Result<Aff, Unknown> {
    match e {
        Expr::Attribute(name, width) => match stage {
            Stage::Vertex => attribute(name, *width, draw),
            Stage::Fragment(_) => Err(Unknown::BadInput(format!("attribute {name} read in the fragment stage"))),
        },
        Expr::Varying(name) => match stage {
            Stage::Vertex => Err(Unknown::BadInput(format!("varying {name} read in the vertex stage"))),
            Stage::Fragment(vs) => match vs.get(name) {
                Some(r) => r.clone(),
                None => Err(Unknown::BadInput(format!("varying {name} is not written"))),
            },
        },
        Expr::Uniform(name) => match draw.uniforms.get(name) {
            Some(UniformValue::Vec(v)) if !v.is_empty() && v.len() <= 4 => {
                Ok(v.iter().map(|&c| Lin::constant(c)).collect())
            }
            Some(_) => Err(Unknown::BadInput(format!("uniform {name} is not a vector"))),
            None => Err(Unknown::BadInput(format!("uniform {name} has no value"))),
        },
        Expr::Const(v) => {
            if v.is_empty() || v.len() > 4 {
                return Err(Unknown::BadInput("constant width".into()));
            }
            Ok(v.iter().map(|&c| Lin::constant(c)).collect())
        }
        Expr::Add(a, b) => {
            let (a, b) = broadcast(affine(a, stage, draw)?, affine(b, stage, draw)?)?;
            Ok(a.iter().zip(&b).map(|(x, y)| x.add(y)).collect())
        }
        Expr::Mul(a, b) => {
            let (a, b) = broadcast(affine(a, stage, draw)?, affine(b, stage, draw)?)?;
            a.iter().zip(&b).map(|(x, y)| x.mul(y)).collect()
        }
        Expr::MatVec(m, v) => {
            let Expr::Uniform(name) = m.as_ref() else {
                return Err(Unknown::CoordNotAffine);
            };
            let Some(UniformValue::Mat { n, data }) = draw.uniforms.get(name) else {
                return Err(Unknown::BadInput(format!("uniform {name} is not a matrix")));
            };
            let n = *n;
            if !(2..=4).contains(&n) || data.len() != n * n {
                return Err(Unknown::BadInput(format!("matrix {name} size")));
            }
            let v = affine(v, stage, draw)?;
            if v.len() != n {
                return Err(Unknown::BadInput(format!("matrix {name} times a vec{}", v.len())));
            }
            Ok((0..n)
                .map(|row| (0..n).fold(Lin::constant(0.0), |acc, col| acc.add(&v[col].scale(data[col * n + row]))))
                .collect())
        }
        Expr::Swizzle(a, idx) => {
            let a = affine(a, stage, draw)?;
            if idx.is_empty() || idx.len() > 4 {
                return Err(Unknown::BadInput("swizzle width".into()));
            }
            idx.iter()
                .map(|&i| a.get(i).cloned().ok_or_else(|| Unknown::BadInput("swizzle out of range".into())))
                .collect()
        }
        Expr::Construct(parts) => {
            let mut out = Vec::new();
            for p in parts {
                out.extend(affine(p, stage, draw)?);
            }
            if out.is_empty() || out.len() > 4 {
                return Err(Unknown::BadInput("constructor width".into()));
            }
            Ok(out)
        }
        Expr::Texture2D { .. } => Err(Unknown::DependentRead),
        Expr::Other(..) => Err(Unknown::CoordNotAffine),
    }
}

/// The gradient (d/dx_win, d/dy_win) of `f`, if `f`'s attribute part is a
/// linear combination of the window position's (`px`, `py`). Then it is the
/// same for every non-degenerate primitive. Otherwise the gradient depends
/// on the primitive's plane in attribute space and the answer is Unknown.
fn window_gradient(f: &Lin, px: &Lin, py: &Lin) -> Result<(f64, f64), Unknown> {
    let mut vars: Vec<&Var> = px.coeffs.keys().chain(py.coeffs.keys()).collect();
    vars.extend(f.coeffs.keys());
    vars.sort();
    vars.dedup();
    let get = |l: &Lin, v: &Var| l.coeffs.get(v).copied().unwrap_or(0.0);
    let x: Vec<f64> = vars.iter().map(|v| get(px, v)).collect();
    let y: Vec<f64> = vars.iter().map(|v| get(py, v)).collect();
    let g: Vec<f64> = vars.iter().map(|v| get(f, v)).collect();
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(p, q)| p * q).sum::<f64>();
    // Solve g = a x + b y in the least-squares sense, then check the rest.
    let (xx, xy, yy) = (dot(&x, &x), dot(&x, &y), dot(&y, &y));
    let det = xx * yy - xy * xy;
    // A NaN anywhere makes det NaN, which is_finite refuses.
    if !det.is_finite() || det <= 1e-12 * xx.max(1e-300) * yy.max(1e-300) {
        return Err(Unknown::Degenerate);
    }
    let (gx, gy) = (dot(&g, &x), dot(&g, &y));
    let a = (gx * yy - gy * xy) / det;
    let b = (gy * xx - gx * xy) / det;
    let rest: f64 = g.iter().zip(x.iter().zip(&y)).map(|(gi, (xi, yi))| (gi - a * xi - b * yi).abs()).sum();
    let scale: f64 = g.iter().map(|v| v.abs()).sum::<f64>().max(1e-300);
    if rest > 1e-9 * scale {
        return Err(Unknown::DependsOnPrimitive);
    }
    // f = a * x_win + b * y_win + const, with x_win and y_win independent
    // window coordinates: df/dx_win = a, df/dy_win = b.
    Ok((a, b))
}
