// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An interpreter for the front end's IR, with real control flow.
//!
//! The byte interpreter in `mod.rs` compares our program with selection's
//! program, and a defect IN selection (an if-conversion that selects the
//! wrong arm, a retarget that moves the wrong writer) is invisible to that:
//! both sides carry it. pco's program would catch it, but pco compiles an
//! `if` to predicated execution and branches this model does not follow.
//! This evaluates the `ir::Module` itself -- `if` as a branch, not as a
//! select -- so what it answers depends on nothing below the front end.
//!
//! GLSL ES 1.00 meanings, written from the specification's definitions, one
//! lane at a time; a boolean is all-ones or zero, as the IR's constants and
//! both back ends have it. The derivatives and texture fetches have no
//! single-invocation meaning and use the same model as the byte interpreter
//! (`alu`'s fdsx/fdsy, `sample_model`), which is all a comparison needs.

use std::collections::BTreeMap;
use yakogl::shaderc::online::ir::{self, AluOp, IoSlot, Node, Op};

/// Where the program's inputs come from.
pub struct IrInputs<'a> {
    /// An attribute or varying's component, by location.
    pub input: &'a dyn Fn(IoSlot, u32) -> u32,
    /// A uniform block dword.
    pub uniform: &'a dyn Fn(u16) -> u32,
    /// A sampler binding's marker for `sample_model`.
    pub marker: &'a dyn Fn(u8) -> u32,
}

type V4 = [u32; 4];
/// Per lane: is the value undefined in GLSL (derived from a local nothing
/// wrote, or from a NaN an operation had to decide about)?
type T4 = [bool; 4];
/// An output's four components as stored: the value and whether it is
/// undefined, `None` for a component nothing stored.
pub type Stored = [Option<(u32, bool)>; 4];

/// What lane `lane` past a value's width holds, for the value made by
/// instruction (or local) `id`. Every value is four lanes here and an
/// operation computes all four, and the module's width table can only
/// over-state a width (a scalar minus a splat constant reads as four), so
/// these lanes meet divisions and square roots and cannot be skipped. A
/// generic value, different per value and per lane, so that two of them
/// never cancel: zero padding made `0 / 0` and one constant made `(a - b) *
/// rcp(c - b)` a NaN in lanes nothing reads, and the undefined-input check
/// discarded every run of three programs for it.
fn pad(id: u32, lane: usize) -> u32 {
    (1.0f32 + 0.0137 * (id % 101) as f32 + 0.29 * lane as f32).to_bits()
}

struct Ev<'m, 'i> {
    m: &'m ir::Module<'m>,
    inp: &'i IrInputs<'i>,
    vals: Vec<Option<(V4, T4)>>,
    locals: BTreeMap<u32, (V4, T4)>,
    outputs: Vec<(IoSlot, Stored)>,
    steps: usize,
    taken: BTreeMap<u32, u8>,
    nan: bool,
    discarded: bool,
}

enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

fn f(x: u32) -> f32 {
    f32::from_bits(x)
}
fn fb(x: f32) -> u32 {
    x.to_bits()
}
fn b(x: bool) -> u32 {
    if x {
        u32::MAX
    } else {
        0
    }
}

impl Ev<'_, '_> {
    /// The bits of `pvr_layers_<sampler>`, the stacked texture's layer
    /// count the GL core writes, found by name as selection finds it.
    fn layers_of(&self, sampler: u8) -> Result<u32, String> {
        let name = self
            .m
            .uniforms
            .iter()
            .find(|u| u.binding == sampler && u.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK)
            .ok_or("a stacked sampler with no row")?
            .name;
        let at = self
            .m
            .uniforms
            .iter()
            .find(|u| {
                u.dword_off != yakogl::shaderc::SHC_UNIFORM_NO_BLOCK && u.name.strip_prefix("pvr_layers_") == Some(name)
            })
            .ok_or("a stacked sampler with no pvr_layers_ uniform")?
            .dword_off;
        Ok((self.inp.uniform)(at))
    }

    fn src(&self, s: ir::Src) -> Result<V4, String> {
        Ok(self.src_t(s)?.0)
    }

    fn src_t(&self, s: ir::Src) -> Result<(V4, T4), String> {
        let (v, t) =
            self.vals.get(s.val.0 as usize).copied().flatten().ok_or(format!("v{} read before defined", s.val.0))?;
        let at = |k: usize| s.swizzle[k] as usize & 3;
        Ok(([0, 1, 2, 3].map(|k| v[at(k)]), [0, 1, 2, 3].map(|k| t[at(k)])))
    }

    /// Which lanes of `o`'s result are undefined, given its operands'.
    /// (`Bcsel` is the caller's: it depends on the arm taken.)
    fn taint(&self, o: AluOp, a: &[ir::Src; 3]) -> Result<T4, String> {
        use AluOp::*;
        let n = o.arity();
        let mut ops: [(V4, T4); 3] = [([0; 4], [false; 4]); 3];
        for (k, s) in a.iter().enumerate().take(n) {
            ops[k] = self.src_t(*s)?;
        }
        /* A NaN reaching an operation that must decide something about it
         * -- a comparison, min/max, sign, a conversion -- gets an answer
         * each implementation chooses (our `fle` is `fge` swapped, pco's is
         * not; `max` picks either operand), so the two sides may differ
         * without either being wrong. Arithmetic only carries a NaN along,
         * which both sides agree on and `close` accepts. */
        let decides =
            matches!(o, Flt | Fle | Feq | Fne | Fmin | Fmax | Fsign | F2i | F2b | Ffloor | Fceil | Ftrunc | Ffract);
        let any = |t: &[bool]| t.iter().any(|x| *x);
        Ok(match o {
            Fdot2 | Fdot3 | Fdot4 => {
                let k = match o {
                    Fdot2 => 2,
                    Fdot3 => 3,
                    _ => 4,
                };
                [any(&ops[0].1[..k]) || any(&ops[1].1[..k]); 4]
            }
            Bany | Ball => [any(&ops[0].1); 4],
            /* GLSL leaves a zero divisor, and INT_MIN / -1, undefined: the
             * lane is not compared. */
            Idiv | Imod => [0, 1, 2, 3].map(|k| {
                let (x, y) = (ops[0].0[k] as i32, ops[1].0[k] as i32);
                ops[0].1[k] || ops[1].1[k] || y == 0 || (x == i32::MIN && y == -1)
            }),
            /* sin and cos of an argument so large that its own float spacing
             * (from 2^16 on, 2^-7 rad and more) exceeds what any comparison
             * could hold two implementations to: the hardware reduces x by
             * 2pi in f32 (fred), the IR evaluates exactly, and neither answer
             * is meaningful there. Found by Qt's particle shaders, whose
             * sin(float(instanceIndex) * 12.99) reached 1.4e10. */
            Fsin | Fcos => [0, 1, 2, 3].map(|k| ops[0].1[k] || f(ops[0].0[k]).abs() >= 65536.0),
            /* a zero divisor, a shift past the width, and a float that is
             * not a u32 (negative, NaN, too large) are undefined */
            Udiv | Umod => [0, 1, 2, 3].map(|k| ops[0].1[k] || ops[1].1[k] || ops[1].0[k] == 0),
            Ushr | Ishl | Ishr => [0, 1, 2, 3].map(|k| ops[0].1[k] || ops[1].1[k] || ops[1].0[k] >= 32),
            F2u => [0, 1, 2, 3].map(|k| {
                let v = f(ops[0].0[k]);
                ops[0].1[k] || !(0.0..4_294_967_296.0).contains(&v)
            }),
            _ => [0, 1, 2, 3]
                .map(|k| (0..n).any(|i| ops[i].1[k]) || (decides && (0..n).any(|i| f(ops[i].0[k]).is_nan()))),
        })
    }

    fn alu(&self, o: AluOp, a: &[ir::Src; 3]) -> Result<V4, String> {
        use AluOp::*;
        let n = o.arity();
        let x = self.src(a[0])?;
        let y = if n > 1 { self.src(a[1])? } else { [0; 4] };
        let z = if n > 2 { self.src(a[2])? } else { [0; 4] };
        let lane = |k: usize| -> Result<u32, String> {
            let (p, q, r) = (x[k], y[k], z[k]);
            Ok(match o {
                Fadd => fb(f(p) + f(q)),
                Fmul => fb(f(p) * f(q)),
                Ffma => fb(f(p) * f(q) + f(r)),
                Fmin => {
                    if f(p) < f(q) {
                        p
                    } else {
                        q
                    }
                }
                Fmax => {
                    if f(p) > f(q) {
                        p
                    } else {
                        q
                    }
                }
                Fneg => p ^ 0x8000_0000,
                Fabs => p & 0x7fff_ffff,
                /* The sign bit decides, as both compilers' sign does. */
                Fsign => {
                    if (p as i32) < 0 {
                        fb(-1.0)
                    } else if f(p) > 0.0 {
                        fb(1.0)
                    } else {
                        0
                    }
                }
                Frcp => fb(1.0 / f(p)),
                Frsq => fb(1.0 / f(p).sqrt()),
                Fsqrt => fb(f(p).sqrt()),
                Fexp2 => fb(f(p).exp2()),
                Flog2 => fb(f(p).log2()),
                Fsin => fb(f(p).sin()),
                Fcos => fb(f(p).cos()),
                Ffloor => fb(f(p).floor()),
                Fceil => fb(f(p).ceil()),
                Ftrunc => fb(f(p).trunc()),
                Ffract => fb(f(p) - f(p).floor()),
                Fddx => super::alu("fdsx", &[p]).ok_or("fdsx")?,
                Fddy => super::alu("fdsy", &[p]).ok_or("fdsy")?,
                Flt => b(f(p) < f(q)),
                Fle => b(f(p) <= f(q)),
                Feq => b(f(p) == f(q)),
                Fne => b(f(p) != f(q)),
                Iadd => p.wrapping_add(q),
                Imul => p.wrapping_mul(q),
                Ineg => p.wrapping_neg(),
                Ishl => p.wrapping_shl(q),
                Ishr => ((p as i32).wrapping_shr(q)) as u32,
                Iand => p & q,
                Ior => p | q,
                Ixor => p ^ q,
                Ilt => b((p as i32) < (q as i32)),
                Ile => b((p as i32) <= (q as i32)),
                /* GLSL ES 3.00's unsigned and min/max operations (section
                 * 5.9, 8.3); the undefined lanes are `taint`'s */
                Ult => b(p < q),
                Ule => b(p <= q),
                Ushr => p.checked_shr(q).unwrap_or(0),
                Imin => (p as i32).min(q as i32) as u32,
                Imax => (p as i32).max(q as i32) as u32,
                Umin => p.min(q),
                Umax => p.max(q),
                Inot => !p,
                U2f => fb(p as f32),
                F2u => f(p) as u32,
                Ieq => b(p == q),
                Ine => b(p != q),
                B2f => {
                    if p != 0 {
                        fb(1.0)
                    } else {
                        0
                    }
                }
                B2i => u32::from(p != 0),
                F2i => f(p) as i32 as u32,
                I2f => fb(p as i32 as f32),
                F2b => b(f(p) != 0.0),
                I2b => b(p != 0),
                Bnot => b(p == 0),
                Bcsel => {
                    if p != 0 {
                        q
                    } else {
                        r
                    }
                }
                other => return Err(format!("{other:?} per lane")),
            })
        };
        Ok(match o {
            Fdot2 | Fdot3 | Fdot4 => {
                let k = match o {
                    Fdot2 => 2,
                    Fdot3 => 3,
                    _ => 4,
                };
                /* Left to right, as the expansion multiplies then adds. */
                let mut acc = f(x[0]) * f(y[0]);
                for i in 1..k {
                    acc += f(x[i]) * f(y[i]);
                }
                [fb(acc); 4]
            }
            /* All four lanes, by the IR's contract (lower.rs
             * `compare_vector`): the front end repeats the last real lane
             * into the spare ones. */
            Bany => [b(x.iter().any(|v| *v != 0)); 4],
            Ball => [b(x.iter().all(|v| *v != 0)); 4],
            /* Truncating division and the remainder with the dividend's
             * sign (`eval.rs`); the undefined lanes are `taint`'s. */
            Idiv => [0, 1, 2, 3].map(|k| (x[k] as i32).checked_div(y[k] as i32).unwrap_or(0) as u32),
            Imod => [0, 1, 2, 3].map(|k| (x[k] as i32).checked_rem(y[k] as i32).unwrap_or(0) as u32),
            Udiv => [0, 1, 2, 3].map(|k| x[k].checked_div(y[k]).unwrap_or(0)),
            Umod => [0, 1, 2, 3].map(|k| x[k].checked_rem(y[k]).unwrap_or(0)),
            _ => [lane(0)?, lane(1)?, lane(2)?, lane(3)?],
        })
    }

    fn instr(&mut self, id: ir::InstrId) -> Result<(), String> {
        let op = *self.m.instr(id).ok_or("an instruction that does not exist")?;
        let v: Option<(V4, T4)> = match op {
            Op::ConstF32(c) => Some((c, [false; 4])),
            Op::ConstI32(c) => Some((c.map(|x| x as u32), [false; 4])),
            Op::ConstBool(c) => Some((c.map(b), [false; 4])),
            Op::Alu(o, a) => {
                let r = self.alu(o, &a)?;
                /* Bcsel's taint needs to know which arm it took: compare
                 * against the condition directly. */
                let t = if o == AluOp::Bcsel {
                    let (c, ct) = self.src_t(a[0])?;
                    let (_, yt) = self.src_t(a[1])?;
                    let (_, zt) = self.src_t(a[2])?;
                    [0, 1, 2, 3].map(|k| ct[k] || if c[k] != 0 { yt[k] } else { zt[k] })
                } else {
                    self.taint(o, &a)?
                };
                Some((r, t))
            }
            Op::LoadUniform { dword_off, comps } => {
                let mut r = [0, 1, 2, 3].map(|k| pad(id.0, k));
                for (k, x) in r.iter_mut().enumerate().take(comps as usize) {
                    *x = (self.inp.uniform)(dword_off + k as u16);
                }
                Some((r, [false; 4]))
            }
            /* Element `clamp(index, 0, len - 1)`: the IR's rule for an
             * index outside the array. An undefined index is undefined. */
            Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                let (iv, it) = self.src_t(index)?;
                let i = (iv[0] as i32).clamp(0, i32::from(len) - 1) as u16;
                let mut r = [0, 1, 2, 3].map(|k| pad(id.0, k));
                for (k, x) in r.iter_mut().enumerate().take(comps as usize) {
                    *x = (self.inp.uniform)(base_dw + i * stride_dw + k as u16);
                }
                Some((r, [it[0]; 4]))
            }
            Op::LoadInput { slot, comps } => {
                let mut r = [0, 1, 2, 3].map(|k| pad(id.0, k));
                for (k, x) in r.iter_mut().enumerate().take(comps as usize) {
                    *x = (self.inp.input)(slot, k as u32);
                }
                Some((r, [false; 4]))
            }
            /* A local lane nothing wrote (a VS reading back gl_Position on a
             * path that never set it) is undefined in GLSL. */
            Op::LoadLocal { local, index: None, .. } => Some(
                self.locals.get(&local.0).copied().unwrap_or(([0, 1, 2, 3].map(|k| pad(local.0 + 1000, k)), [true; 4])),
            ),
            Op::StoreLocal { local, index: None, src, write_mask } => {
                let (s, t) = self.src_t(src)?;
                let e = self.locals.entry(local.0).or_insert(([0, 1, 2, 3].map(|k| pad(local.0 + 1000, k)), [true; 4]));
                for k in 0..4 {
                    if write_mask & (1 << k) != 0 {
                        e.0[k] = s[k];
                        e.1[k] = t[k];
                    }
                }
                None
            }
            Op::StoreOutput { slot, src } => {
                let (s, t) = self.src_t(src)?;
                let mut c = [None; 4];
                for (k, x) in c.iter_mut().enumerate() {
                    *x = Some((s[k], t[k]));
                }
                match self.outputs.iter_mut().find(|(sl, _)| *sl == slot) {
                    Some((_, o)) => *o = c,
                    None => self.outputs.push((slot, c)),
                }
                None
            }
            op @ Op::Tex { sampler, coord, .. }
                if matches!(op.es100_tex(), Some((ir::Es100Tex::Tex2D | ir::Es100Tex::Tex2DProj, None))) =>
            {
                let (c, t) = self.src_t(coord)?;
                let marker = (self.inp.marker)(sampler);
                /* texture2DProj: (x / q, y / q), q the coordinate's lane 3
                 * (IR v1; it was lane 2 until then). */
                let (u, v, tq) = if matches!(op.es100_tex(), Some((ir::Es100Tex::Tex2DProj, _))) {
                    let q = f(c[3]);
                    ((f(c[0]) / q).to_bits(), (f(c[1]) / q).to_bits(), t[3])
                } else {
                    (c[0], c[1], false)
                };
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [t[0] || t[1] || tq; 4]))
            }
            /* textureSize on a 2D sampler: pco's usclib_tex_state_size over
             * the model's image state (`tex_state`), as the byte model's
             * programs read it from the descriptor. */
            Op::TexSize { sampler, dim: ir::TexDim::D2, lod } => {
                let (l, t) = self.src_t(lod)?;
                let [w, h] = super::tex_size_2d(super::tex_state((self.inp.marker)(sampler)), l[0]);
                Some(([w, h, 0, 0], [t[0]; 4]))
            }
            /* texelFetch on a 2D sampler: texel p exactly, at level `lod`
             * (the model's texel p, as `texel_coord` maps a centre). */
            Op::Tex {
                dim: ir::TexDim::D2,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(l),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let (lv, lt) = self.src_t(l)?;
                let centre = |p: u32| super::texel_coord(p as i32 as f32 + 0.5);
                let (u, v) = (centre(c[0])?, centre(c[1])?);
                let marker = super::marker_at_level((self.inp.marker)(sampler), (lv[0] as i32 as f32).to_bits());
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [t[0] || t[1] || lt[0]; 4]))
            }
            /* textureSize of a stacked texture: glsl2usc's pvr_arrSize
             * (w, h / layers, layers) and pvr_cubeSize (w, h / 6). */
            Op::TexSize { sampler, dim: dim @ (ir::TexDim::D2Array | ir::TexDim::Cube), lod } => {
                let (l, t) = self.src_t(lod)?;
                let [w, h] = super::tex_size_2d(super::tex_state((self.inp.marker)(sampler)), l[0]);
                let count = if dim == ir::TexDim::Cube { 6 } else { f(self.layers_of(sampler)?) as i32 };
                let per = (h as i32).checked_div(count).unwrap_or(0) as u32;
                Some(([w, per, count as u32, 0], [t[0]; 4]))
            }
            /* texelFetch on a 2D array: glsl2usc's pvr_arrFetch, texel
             * (x, y + z * (h / layers)) of the stacked image. */
            Op::Tex {
                dim: ir::TexDim::D2Array,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(l),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let (lv, lt) = self.src_t(l)?;
                let base = (self.inp.marker)(sampler);
                let [_, h] = super::tex_size_2d(super::tex_state(base), lv[0]);
                let count = f(self.layers_of(sampler)?) as i32;
                let per = (h as i32).checked_div(count).unwrap_or(0);
                let row = (c[1] as i32).wrapping_add((c[2] as i32).wrapping_mul(per));
                let centre = |p: i32| super::texel_coord(p as f32 + 0.5);
                let (u, v) = (centre(c[0] as i32)?, centre(row)?);
                let marker = super::marker_at_level(base, (lv[0] as i32 as f32).to_bits());
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [t[0] || t[1] || t[2] || lt[0]; 4]))
            }
            /* textureSize of a 3D texture: pvr_volSize (level 0). */
            Op::TexSize { sampler, dim: ir::TexDim::D3, lod } => {
                let (_, t) = self.src_t(lod)?;
                let (_, [sw, sh], n) =
                    super::vol_grid(super::tex_state((self.inp.marker)(sampler)), self.layers_of(sampler)?);
                Some(([sw as u32, sh as u32, n as u32, 0], [t[0]; 4]))
            }
            /* texelFetch on a 3D texture: pvr_volFetch (level 0). */
            Op::Tex {
                dim: ir::TexDim::D3,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(_),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let base = (self.inp.marker)(sampler);
                let (cols, slice, _) = super::vol_grid(super::tex_state(base), self.layers_of(sampler)?);
                let z = c[2] as i32;
                let cell = [z.checked_rem(cols).unwrap_or(0), z.checked_div(cols).unwrap_or(0)];
                let at = |k: usize| (c[k] as i32).wrapping_add(cell[k].wrapping_mul(slice[k]));
                let centre = |p: i32| super::texel_coord(p as f32 + 0.5);
                let (u, v) = (centre(at(0))?, centre(at(1))?);
                let marker = super::marker_at_level(base, 0.0f32.to_bits());
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [t[0] || t[1] || t[2]; 4]))
            }
            /* A 3D texture: glsl2usc's pvr_vol over the grid of slices
             * (`vol_sample`); pvr_volLod samples level 0 whatever it is given. */
            Op::Tex {
                dim: ir::TexDim::D3,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_)),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let base = (self.inp.marker)(sampler);
                let marker = match lod {
                    ir::TexLod::Lod(_) => super::marker_at_level(base, 0.0f32.to_bits()),
                    _ => base,
                };
                let layers = self.layers_of(sampler)?;
                let out = super::vol_sample(super::tex_state(base), layers, [c[0], c[1], c[2]], marker);
                Some((out, [t[0] || t[1] || t[2]; 4]))
            }
            /* A 2D array: the stacked 2D image's coordinate (`array_uv`,
             * glsl2usc's pvr_arrUV), the layer count read from the uniform
             * `pvr_layers_<sampler>` by name. */
            Op::Tex {
                dim: ir::TexDim::D2Array,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let name = self
                    .m
                    .uniforms
                    .iter()
                    .find(|u| u.binding == sampler && u.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK)
                    .ok_or("an array sampler with no row")?
                    .name;
                let at = self
                    .m
                    .uniforms
                    .iter()
                    .find(|u| {
                        u.dword_off != yakogl::shaderc::SHC_UNIFORM_NO_BLOCK
                            && u.name.strip_prefix("pvr_layers_") == Some(name)
                    })
                    .ok_or("an array sampler with no pvr_layers_ uniform")?
                    .dword_off;
                let base = (self.inp.marker)(sampler);
                let (u, v) = super::array_uv(super::tex_state(base), (self.inp.uniform)(at), [c[0], c[1], c[2]]);
                let (marker, lt) = match lod {
                    /* a bias on this model's implicit level 0 reads the
                     * level it names, as a replaced level does */
                    ir::TexLod::Lod(l) | ir::TexLod::Bias(l) => {
                        let (lv, lt) = self.src_t(l)?;
                        (super::marker_at_level(base, lv[0]), lt[0])
                    }
                    _ => (base, false),
                };
                let undefined = t[0] || t[1] || t[2] || lt;
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [undefined; 4]))
            }
            /* A cube: the stacked 2D image's coordinate (`cube_uv`, glsl2usc's
             * pvr_cubeUV), sampled with the implicit level or a replacing one. */
            Op::Tex {
                dim: ir::TexDim::Cube,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset: None,
            } => {
                let (c, t) = self.src_t(coord)?;
                let base = (self.inp.marker)(sampler);
                let level = match lod {
                    ir::TexLod::Lod(l) => Some(self.src_t(l)?.0[0]),
                    _ => None,
                };
                let (u, v) = super::cube_uv(super::tex_state(base), [c[0], c[1], c[2]], level);
                let (marker, lt) = match lod {
                    /* a bias on this model's implicit level 0 reads the
                     * level it names, as a replaced level does */
                    ir::TexLod::Lod(l) | ir::TexLod::Bias(l) => {
                        let (lv, lt) = self.src_t(l)?;
                        (super::marker_at_level(base, lv[0]), lt[0])
                    }
                    _ => (base, false),
                };
                let undefined = t[0] || t[1] || t[2] || lt;
                Some(([0, 1, 2, 3].map(|k| super::sample_model(marker, u, v, k)), [undefined; 4]))
            }
            /* textureLod on a 2D sampler: the level replaces the implicit one
             * (the byte model's `lodm` replace, the same texture per level),
             * and a bias adds to this model's implicit level 0;
             * with a constant offset, that many texels away. */
            Op::Tex {
                dim: ir::TexDim::D2,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset,
            } => {
                let (c, t) = self.src_t(coord)?;
                let base = (self.inp.marker)(sampler);
                let (marker, lt) = match lod {
                    /* a bias on this model's implicit level 0 reads the
                     * level it names, as a replaced level does */
                    ir::TexLod::Lod(l) | ir::TexLod::Bias(l) => {
                        let (lv, lt) = self.src_t(l)?;
                        (super::marker_at_level(base, lv[0]), lt[0])
                    }
                    _ => (base, false),
                };
                let off = offset.map_or([0, 0], |[du, dv, _]| [i32::from(du), i32::from(dv)]);
                Some((
                    [0, 1, 2, 3].map(|k| super::sample_model_at(marker, c[0], c[1], off, k)),
                    [t[0] || t[1] || lt; 4],
                ))
            }
            /* `discard` ends the invocation (the caller stops at it). */
            Op::Discard => {
                self.discarded = true;
                None
            }
            other => return Err(format!("{other:?} is not in this interpreter")),
        };
        /* A 2D array or 3D look-up reads its layer count from
         * pvr_layers_<sampler>, which the GL core only ever writes as a whole
         * number of layers (1 when nothing is bound). Any other value -- the
         * random bits a test puts in the uniform block -- makes the
         * conversion and the divisions undefined, and the program's F2i and
         * idiv need not agree with this model's saturating arithmetic. */
        let stacked = match op {
            Op::Tex { dim, sampler, .. } | Op::TexSize { dim, sampler, .. } => {
                matches!(dim, ir::TexDim::D2Array | ir::TexDim::D3).then_some(sampler)
            }
            _ => None,
        };
        let v = match (v, stacked) {
            (Some((val, mut t)), Some(sampler)) => {
                let l = f(self.layers_of(sampler)?);
                if !(1.0..=2048.0).contains(&l) || l.fract() != 0.0 {
                    t = [true; 4];
                }
                Some((val, t))
            }
            (v, _) => v,
        };
        if let Some(v) = v {
            self.vals[id.0 as usize] = Some(v);
        }
        Ok(())
    }

    fn nodes(&mut self, r: ir::Range) -> Result<Flow, String> {
        for (i, n) in self.m.nodes_of(r).iter().enumerate() {
            self.steps += 1;
            if self.steps > 1_000_000 {
                return Err("more than a million steps".into());
            }
            match *n {
                Node::Instr(id) => {
                    self.instr(id)?;
                    if self.discarded {
                        return Ok(Flow::Return);
                    }
                }
                Node::If { cond, then_, else_ } => {
                    let (cv, ct) = self.src_t(cond)?;
                    let c = cv[0] != 0;
                    /* A branch on an undefined value leaves every later
                     * store undefined: the whole run is. */
                    if ct[0] {
                        self.nan = true;
                    } else {
                        *self.taken.entry(r.first + i as u32).or_default() |= if c { 1 } else { 2 };
                    }
                    match self.nodes(if c { then_ } else { else_ })? {
                        Flow::Next => {}
                        other => return Ok(other),
                    }
                }
                /* At most UNIFORM_LOOP_CAP entries, as the compiler's loops
                 * counted at run time run: past it GLSL would go on and the
                 * compiled program stops (a deliberate deviation, so the
                 * comparison models it rather than reporting it). Every
                 * loop with a constant bound stops well before. */
                Node::Loop { body } => {
                    let cap = yakogl::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;
                    let mut entries = 0u32;
                    loop {
                        if entries >= cap {
                            break;
                        }
                        entries += 1;
                        match self.nodes(body)? {
                            Flow::Break => break,
                            Flow::Return => return Ok(Flow::Return),
                            Flow::Next | Flow::Continue => {}
                        }
                    }
                }
                Node::Break => return Ok(Flow::Break),
                Node::Continue => return Ok(Flow::Continue),
                Node::Return => return Ok(Flow::Return),
                #[allow(unreachable_patterns)]
                _ => return Err("a node this interpreter does not know".into()),
            }
        }
        Ok(Flow::Next)
    }
}

/// What one run stored, and which way each `if` it reached went.
pub struct IrRun {
    /// Every output component stored, last store winning, with whether it
    /// is undefined: derived from a local nothing wrote or from a NaN an
    /// operation had to decide about (`normalize` of a zero vector). Each
    /// implementation answers those its own way, so an undefined lane is
    /// not compared.
    pub outputs: Vec<(IoSlot, Stored)>,
    /// By the `if`'s node index: bit 0 the then arm ran, bit 1 the else arm.
    /// An `if` a run never reached, or reached with an undefined condition,
    /// is absent.
    pub taken: BTreeMap<u32, u8>,
    /// A branch was taken on an undefined condition: nothing after it is
    /// defined, and the run is not compared at all.
    pub undefined: bool,
    /// The invocation executed `discard`: no output of it counts.
    pub discarded: bool,
}

/// Run `m` on `inp`.
pub fn run_ir(m: &ir::Module<'_>, inp: &IrInputs<'_>) -> Result<IrRun, String> {
    let mut e = Ev {
        m,
        inp,
        vals: vec![None; m.instrs.len()],
        locals: BTreeMap::new(),
        outputs: Vec::new(),
        steps: 0,
        taken: BTreeMap::new(),
        nan: false,
        discarded: false,
    };
    e.nodes(m.body)?;
    Ok(IrRun { outputs: e.outputs, taken: e.taken, undefined: e.nan, discarded: e.discarded })
}

/// How many `if` nodes `m` holds, at any depth.
pub fn if_count(m: &ir::Module<'_>) -> usize {
    m.nodes.iter().filter(|n| matches!(n, Node::If { .. })).count()
}
