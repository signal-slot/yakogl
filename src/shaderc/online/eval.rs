// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A reference evaluator for [`ir::Module`]: the tests' oracle, and the
//! transform-feedback capture's vertex shader (`docs/es3-transform-feedback.md`
//! section 4.2), which runs it in the product.
//!
//! The lowering's control-flow rewrites -- early returns turned into a slot
//! and an `else`, a `return` inside a loop turned into a flag and a `break`
//! -- all type-check, all compile, and all produce a module whose SHAPE a
//! test can inspect. None of that says the module computes what the GLSL
//! did: a rewrite that lets the statement after a `return` run anyway looks
//! exactly like one that does not. Running the module answers that, on the
//! host, before any back end or board is involved.
//!
//! It is deliberately small and strict: every lane is a `u32` bit pattern
//! (a float's bits, an int, or a bool as all-ones / zero, as `ir.md`
//! decision 4 has it), and anything it does not implement -- textures,
//! derivatives, an input nobody supplied, a loop that does not end -- stops
//! the run and is returned as an `Err` naming it, never a value a caller
//! could mistake for an answer. (An `Err`, not a panic: the workspace builds
//! with `panic = "abort"`, and `yakogl-tools online-eval` has to report
//! "unmeasured" and carry on.) A slot that is read before any store holds
//! [`UNSET`], which no computation here produces by accident.
//!
//! Two callers, one machine:
//!
//! - the host oracles ([`run`], [`run_with_textures`], [`run_with_trig_error`]:
//!   `host-test` only) collect every store into an [`Outcome`], use the
//!   host's `std` float functions, sample [`Texture`] models and stop at
//!   section 5.9's undefined results, so that an invented value never
//!   becomes a reference;
//! - the capture ([`run_in`]: every build but `report-es2`) lends the
//!   working memory (a [`Frame`] in the compile scratch), takes the stores
//!   through a [`Sink`], computes with [`super::fmath`] (no libm in the
//!   product) and, being a GL draw rather than an oracle, gives an undefined
//!   result a value as the hardware would (GLSL ES 3.00 section 5.9:
//!   undefined, not an error).

use super::ir::{self, AluOp, IoSlot, Module, Node, Op, Range, Src};
#[cfg(any(test, feature = "host-test"))]
use std::{string::String, vec::Vec};

/// What a slot holds before its first store: a quiet NaN with a payload no
/// arithmetic produces, so a read of an unwritten slot shows up in the
/// output instead of passing as a zero.
pub const UNSET: u32 = 0x7fc0_dead;

/// How many iterations one loop may run before the evaluator calls it
/// non-terminating. ES 1.00 Appendix A loops are bounded by constants, and
/// every test loop is far below this.
const MAX_ITERATIONS: u32 = 10_000;

/// The same bound for the capture: ES 3.00 lifts Appendix A, so a `while`
/// loop may run longer than any test's; still finite, so a shader that does
/// not terminate refuses its draw instead of hanging the GL call.
const MAX_ITERATIONS_CAPTURE: u32 = 1 << 20;

/// Bytes a [`Fault`] keeps of its text.
const FAULT_BYTES: usize = 112;

/// Why a run stopped, as text, without a heap: the first [`FAULT_BYTES`]
/// bytes of the message (cut at a character boundary).
#[derive(Clone, Copy)]
pub struct Fault {
    buf: [u8; FAULT_BYTES],
    len: u8,
}

impl Fault {
    /// The first [`FAULT_BYTES`] bytes of the message.
    pub fn new(args: core::fmt::Arguments) -> Fault {
        let mut f = Fault { buf: [0; FAULT_BYTES], len: 0 };
        let _ = core::fmt::write(&mut f, args); /* a full buffer only cuts the text */
        f
    }

    /// The message.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("?")
    }
}

impl core::fmt::Write for Fault {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            let mut b = [0u8; 4];
            let e = ch.encode_utf8(&mut b).as_bytes();
            let at = usize::from(self.len);
            if at + e.len() > FAULT_BYTES {
                return Err(core::fmt::Error);
            }
            self.buf[at..at + e.len()].copy_from_slice(e);
            self.len += e.len() as u8;
        }
        Ok(())
    }
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::fmt::Debug for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

/// Where a run's output stores go, in store order.
pub trait Sink {
    /// `slot` was stored `value`; `defined` says which lanes some
    /// instruction defined (see [`Outcome::defined`]).
    fn store(&mut self, slot: IoSlot, value: [u32; 4], defined: [bool; 4]);
}

/// The working memory of one run, lent by the caller: a value and its
/// defined lanes per instruction (`m.instrs.len()` entries) and per local
/// (`m.locals.len()`). [`run_in`] resets it.
pub struct Frame<'f> {
    pub values: &'f mut [[u32; 4]],
    pub def: &'f mut [[bool; 4]],
    pub locals: &'f mut [[u32; 4]],
    pub ldef: &'f mut [[bool; 4]],
}

/// What running a module produced.
#[cfg(any(test, feature = "host-test"))]
#[derive(Debug, Default)]
pub struct Outcome {
    /// Every output stored, in store order; [`Outcome::output`] reads the
    /// last one for a slot.
    pub stores: Vec<(IoSlot, [u32; 4])>,
    /// For each entry of `stores`, which of its lanes some instruction
    /// defined. A lane no instruction defined -- lane 1..3 of a dot product
    /// or of `any`/`all`, a component past a value's width, a local never
    /// stored -- is what a GPU whose registers are scalar (the USC) fills
    /// with whatever was there; the value here is [`UNSET`]. Until
    /// 2026-09-24 a reduction's result was broadcast to all four lanes,
    /// which hid `faceforward`'s unsplatted condition from the value
    /// oracles (13 dEQP cases wrong on the AM62P).
    pub defined: Vec<[bool; 4]>,
    pub discarded: bool,
}

#[cfg(any(test, feature = "host-test"))]
impl Sink for Outcome {
    fn store(&mut self, slot: IoSlot, value: [u32; 4], defined: [bool; 4]) {
        self.stores.push((slot, value));
        self.defined.push(defined);
    }
}

#[cfg(any(test, feature = "host-test"))]
impl Outcome {
    /// The last value stored to `slot`, or `None` if nothing was.
    pub fn output(&self, slot: IoSlot) -> Option<[u32; 4]> {
        self.stores.iter().rev().find(|(s, _)| *s == slot).map(|(_, v)| *v)
    }

    /// The last value stored to `slot`, read as floats.
    pub fn output_f32(&self, slot: IoSlot) -> Option<[f32; 4]> {
        self.output(slot).map(|v| v.map(f32::from_bits))
    }

    /// Which lanes of the last store to `slot` an instruction defined.
    pub fn output_defined(&self, slot: IoSlot) -> Option<[bool; 4]> {
        self.stores.iter().enumerate().rev().find(|(_, (s, _))| *s == slot).map(|(i, _)| self.defined[i])
    }
}

/// The inputs a run reads: a value per input slot, and the uniform block as
/// dwords.
pub struct Inputs<'x> {
    pub io: &'x [(IoSlot, [u32; 4])],
    pub uniforms: &'x [u32],
}

/// A texture for [`run_with_textures`]: 2D or cube, its levels and its
/// minification filter, as the V3D interpreter reads it (one model,
/// [`super::texref`]). Every value must be exact in f16 (an RGBA16F
/// texture, so a GPU returns it unrounded) and finite.
#[cfg(any(test, feature = "host-test"))]
pub use super::texref::Texture;

/// Four float lanes as bit patterns.
pub fn f32x4(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flow {
    Normal,
    Break,
    Continue,
    /// `return` from `main`, or a `discard`: the program is over.
    Exit,
}

struct Machine<'m, 'x, 'f> {
    m: &'m Module<'m>,
    input: &'x Inputs<'x>,
    values: &'f mut [[u32; 4]],
    locals: &'f mut [[u32; 4]],
    /// Beside `values` / `locals`: which lanes an instruction defined.
    def: &'f mut [[bool; 4]],
    ldef: &'f mut [[bool; 4]],
    out: &'f mut dyn Sink,
    discarded: bool,
    /// What stopped the run, when the evaluator could not go on.
    fault: Option<Fault>,
    /// By sampler binding (`Op::Tex::sampler`); empty for [`run`].
    #[cfg(any(test, feature = "host-test"))]
    textures: &'x [Texture],
    /// [`run_with_trig_error`]'s shift of every sin/cos result, relative
    /// to max(1, |x|); 0 for the exact functions.
    trig: f32,
    /// The oracle's rules (the host's float functions, section 5.9's
    /// undefined results stop the run), or the capture's ([`run_in`]).
    oracle: bool,
}

/// Run `m` once with `input`. `Err` names what the evaluator does not
/// implement, or what the inputs did not supply. A texture look-up stops
/// the run; [`run_with_textures`] supplies textures.
#[cfg(any(test, feature = "host-test"))]
pub fn run(m: &Module<'_>, input: &Inputs<'_>) -> Result<Outcome, String> {
    run_with_textures(m, input, &[])
}

/// [`run`] with `textures[b]` bound to sampler binding `b`, sampled as GL ES
/// 2.0 does with TEXTURE_MAG_FILTER NEAREST, the texture's
/// TEXTURE_MIN_FILTER (NEAREST or NEAREST_MIPMAP_NEAREST) and both wrap
/// modes CLAMP_TO_EDGE (the sampler state is the caller's, not the
/// module's, so these are this evaluator's). Normalized 2D and cube
/// look-ups of a float sampler are evaluated; the level is dEQP's for an
/// explicit level of detail ([`super::texref`]), and an implicit one (the
/// derivatives) is read only on a texture of one level.
#[cfg(any(test, feature = "host-test"))]
pub fn run_with_textures(m: &Module<'_>, input: &Inputs<'_>, textures: &[Texture]) -> Result<Outcome, String> {
    run_with_trig_error(m, input, textures, 0.0)
}

/// [`run_with_textures`] with every `sin(x)` and `cos(x)` result moved by
/// `trig * max(1, |x|)`. GLSL ES 3.00 section 4.5.1 leaves the precision of
/// the trigonometric functions undefined, so an output that moves past a
/// comparison's tolerance under a shift this small is not a value the
/// language fixes, and a harness cannot hold a back end to it (Qt Quick
/// 3D's `fract(sin(dot(co, k)) * 43758.5)` hash: an f32 argument reduction's
/// 5e-6 moves it by up to 0.77).
#[cfg(any(test, feature = "host-test"))]
pub fn run_with_trig_error(
    m: &Module<'_>,
    input: &Inputs<'_>,
    textures: &[Texture],
    trig: f32,
) -> Result<Outcome, String> {
    for t in textures {
        let faces = if t.cube { 6 } else { 1 };
        if t.levels.is_empty() {
            return Err("a texture with no level".into());
        }
        for (k, l) in t.levels.iter().enumerate() {
            let n = (l.width * l.height) as usize;
            if l.width == 0 || l.height == 0 || l.faces.len() != faces || l.faces.iter().any(|f| f.len() != n) {
                return Err(std::format!(
                    "level {k}: a {}x{} level with {} images of {:?} texels",
                    l.width,
                    l.height,
                    l.faces.len(),
                    l.faces.iter().map(Vec::len).collect::<Vec<_>>()
                ));
            }
            if !l.faces.iter().flatten().flatten().all(|&x| f16_exact(x)) {
                return Err("a texel that is not exact in f16".into());
            }
        }
    }
    let mut values = std::vec![[UNSET; 4]; m.instrs.len()];
    let mut locals = std::vec![[UNSET; 4]; m.locals.len()];
    let mut def = std::vec![[false; 4]; m.instrs.len()];
    let mut ldef = std::vec![[false; 4]; m.locals.len()];
    let mut out = Outcome::default();
    let mut mc = Machine {
        m,
        input,
        textures,
        values: &mut values,
        locals: &mut locals,
        def: &mut def,
        ldef: &mut ldef,
        out: &mut out,
        discarded: false,
        fault: None,
        trig,
        oracle: true,
    };
    mc.range(m.body);
    let (fault, discarded) = (mc.fault, mc.discarded);
    match fault {
        Some(why) => Err(why.as_str().into()),
        None => {
            out.discarded = discarded;
            Ok(out)
        }
    }
}

/// Run `m` once for the transform-feedback capture: `frame` is the working
/// memory (reset here; at least `m.instrs.len()` and `m.locals.len()`
/// entries), the stores go to `out`. The capture's rules (see the module
/// note): `fmath`'s functions, an undefined result is a value, no textures.
/// `Ok(discarded)`, or why the run stopped.
pub fn run_in(m: &Module<'_>, input: &Inputs<'_>, frame: Frame<'_>, out: &mut dyn Sink) -> Result<bool, Fault> {
    let (ni, nl) = (m.instrs.len(), m.locals.len());
    if frame.values.len() < ni || frame.def.len() < ni || frame.locals.len() < nl || frame.ldef.len() < nl {
        return Err(Fault::new(format_args!("the frame is smaller than the module")));
    }
    frame.values[..ni].fill([UNSET; 4]);
    frame.def[..ni].fill([false; 4]);
    frame.locals[..nl].fill([UNSET; 4]);
    frame.ldef[..nl].fill([false; 4]);
    let mut mc = Machine {
        m,
        input,
        #[cfg(any(test, feature = "host-test"))]
        textures: &[],
        values: frame.values,
        locals: frame.locals,
        def: frame.def,
        ldef: frame.ldef,
        out,
        discarded: false,
        fault: None,
        trig: 0.0,
        oracle: false,
    };
    mc.range(m.body);
    match mc.fault {
        Some(why) => Err(why),
        None => Ok(mc.discarded),
    }
}

impl Machine<'_, '_, '_> {
    fn range(&mut self, r: Range) -> Flow {
        for &n in self.m.nodes_of(r) {
            let f = match n {
                Node::Instr(id) => self.instr(id),
                Node::If { cond, then_, else_ } => {
                    if !self.read_def(cond)[0] {
                        return self.stop(format_args!("unset lane: a branch condition no instruction defined"));
                    }
                    if self.read(cond)[0] != 0 {
                        self.range(then_)
                    } else {
                        self.range(else_)
                    }
                }
                Node::Loop { body } => {
                    let mut k = 0;
                    let bound = if self.oracle { MAX_ITERATIONS } else { MAX_ITERATIONS_CAPTURE };
                    loop {
                        k += 1;
                        if k > bound {
                            break self.stop(format_args!("a loop ran {bound} iterations: it does not terminate"));
                        }
                        match self.range(body) {
                            Flow::Break => break Flow::Normal,
                            Flow::Exit => break Flow::Exit,
                            Flow::Normal | Flow::Continue => {}
                        }
                    }
                }
                Node::Break => Flow::Break,
                Node::Continue => Flow::Continue,
                Node::Return => Flow::Exit,
            };
            if f != Flow::Normal {
                return f;
            }
        }
        Flow::Normal
    }

    fn uniform_at(&self, at: usize, comps: u8) -> Option<[u32; 4]> {
        let mut v = [0; 4];
        for (j, l) in v.iter_mut().enumerate().take(comps as usize) {
            *l = *self.input.uniforms.get(at + j)?;
        }
        Some(v)
    }

    /// Record why the run cannot go on, and end it.
    fn stop(&mut self, why: core::fmt::Arguments) -> Flow {
        if self.fault.is_none() {
            self.fault = Some(Fault::new(why));
        }
        Flow::Exit
    }

    fn read(&self, s: Src) -> [u32; 4] {
        let v = self.values[s.val.0 as usize];
        s.swizzle.map(|l| v[l as usize])
    }

    /// Which lanes of a read are defined, through its swizzle: a splat of
    /// lane 0 (`xxxx`) of a one-lane value is defined in all four.
    fn read_def(&self, s: Src) -> [bool; 4] {
        let d = self.def[s.val.0 as usize];
        s.swizzle.map(|l| d[l as usize])
    }

    /// The first `comps` lanes defined, the rest not.
    fn first(comps: u8) -> [bool; 4] {
        core::array::from_fn(|k| k < usize::from(comps))
    }

    fn instr(&mut self, id: ir::InstrId) -> Flow {
        let op = self.m.instrs[id.0 as usize];
        let all = [true; 4];
        let (v, d): ([u32; 4], [bool; 4]) = match op {
            Op::ConstF32(v) => (v, all),
            Op::ConstI32(v) => (v.map(|x| x as u32), all),
            Op::ConstBool(v) => (v.map(bits_of_bool), all),
            Op::Alu(op, srcs) => match self.alu(op, srcs) {
                Ok(vd) => vd,
                Err(why) => return self.stop(format_args!("{why}")),
            },
            Op::LoadUniform { dword_off, comps } => match self.uniform_at(dword_off as usize, comps) {
                Some(v) => (v, Self::first(comps)),
                None => return self.stop(format_args!("uniform dwords {dword_off}+{comps} are past the block")),
            },
            Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                if !self.read_def(index)[0] {
                    return self.stop(format_args!("unset lane: the index of an indexed uniform load"));
                }
                /* Clamped, as ir.md states. */
                let i = (self.read(index)[0] as i32).clamp(0, i32::from(len) - 1) as usize;
                let at = base_dw as usize + i * stride_dw as usize;
                match self.uniform_at(at, comps) {
                    Some(v) => (v, Self::first(comps)),
                    None => return self.stop(format_args!("uniform dwords {at}+{comps} are past the block")),
                }
            }
            Op::LoadInput { slot, .. } => match self.input.io.iter().find(|(s, _)| *s == slot) {
                Some((_, v)) => (*v, all),
                None => return self.stop(format_args!("the module reads input {slot:?}, which was not supplied")),
            },
            Op::StoreOutput { slot, src } => {
                let (v, d) = (self.read(src), self.read_def(src));
                self.out.store(slot, v, d);
                (v, d)
            }
            Op::LoadLocal { local, index: None, comps } => {
                let d = self.ldef[local.0 as usize];
                (self.locals[local.0 as usize], core::array::from_fn(|k| d[k] && k < usize::from(comps)))
            }
            Op::StoreLocal { local, index: None, src, write_mask } => {
                let (v, d) = (self.read(src), self.read_def(src));
                let slot = &mut self.locals[local.0 as usize];
                let sdef = &mut self.ldef[local.0 as usize];
                for i in 0..4 {
                    if write_mask & (1 << i) != 0 {
                        slot[i] = v[i];
                        sdef[i] = d[i];
                    }
                }
                (v, d)
            }
            Op::LoadLocal { index: Some(_), .. } | Op::StoreLocal { index: Some(_), .. } => {
                return self.stop(format_args!("the evaluator has no indexed local"))
            }
            #[cfg(any(test, feature = "host-test"))]
            Op::Tex { dim, shadow, proj, ret, sampler, coord, lod, offset } if !self.textures.is_empty() => {
                let cube = dim == ir::TexDim::Cube;
                if !(dim == ir::TexDim::D2 || cube) || shadow || ret != ir::ScalarType::F32 || offset.is_some() {
                    return self
                        .stop(format_args!("the evaluator samples only a float 2D or cube texture, without offsets"));
                }
                if cube && proj {
                    return self.stop(format_args!("a projected cube look-up"));
                }
                let cd = self.read_def(coord);
                if !(cd[0] && cd[1] && (!cube || cd[2]) && (!proj || cd[3])) {
                    return self.stop(format_args!("unset lane: a texture coordinate no instruction defined"));
                }
                /* An implicit level of detail, with or without a bias, is
                 * the derivatives': texref reads it only on one level. */
                let lod = match lod {
                    ir::TexLod::Implicit | ir::TexLod::Bias(_) => super::texref::Lod::Implicit,
                    ir::TexLod::Lod(l) => {
                        if !self.read_def(l)[0] {
                            return self.stop(format_args!("unset lane: a level of detail no instruction defined"));
                        }
                        super::texref::Lod::Explicit(f32::from_bits(self.read(l)[0]))
                    }
                    ir::TexLod::Grad { .. } => return self.stop(format_args!("a look-up with explicit gradients")),
                    /* integer coordinates and level, no filtering: not a
                     * normalised look-up, so never read as one */
                    ir::TexLod::Fetch(_) => return self.stop(format_args!("a texelFetch (integer coordinates)")),
                };
                match self.tex(sampler, self.read(coord), proj, cube, lod) {
                    Ok(v) => (v, all),
                    Err(why) => return self.stop(format_args!("{why}")),
                }
            }
            /* textureSize of a level (GLSL ES 3.00 8.8: the dimensions of
             * level lod); a cube's is one face's. */
            #[cfg(any(test, feature = "host-test"))]
            Op::TexSize { sampler, dim: ir::TexDim::D2 | ir::TexDim::Cube, lod } if !self.textures.is_empty() => {
                if !self.read_def(lod)[0] {
                    return self.stop(format_args!("unset lane: a textureSize level no instruction defined"));
                }
                let l = self.read(lod)[0];
                match self.textures.get(usize::from(sampler)).map(|t| t.levels.get(l as usize)) {
                    Some(Some(lv)) => ([lv.width, lv.height, 0, 0], Self::first(2)),
                    None => return self.stop(format_args!("no texture at sampler binding {sampler}")),
                    Some(None) => return self.stop(format_args!("textureSize of level {l}: the texture has fewer")),
                }
            }
            Op::Tex { .. } | Op::TexSize { .. } => return self.stop(format_args!("the evaluator has no textures")),
            Op::Discard => {
                self.discarded = true;
                return Flow::Exit;
            }
        };
        self.values[id.0 as usize] = v;
        self.def[id.0 as usize] = d;
        Flow::Normal
    }

    #[cfg(any(test, feature = "host-test"))]
    /// One NEAREST look-up with CLAMP_TO_EDGE, GL ES 2.0.25 sections 3.7.6
    /// and 3.7.7. Both filters NEAREST always read the level zero array
    /// (3.7.7 for minification; 3.7.8: magnification "behaves exactly as
    /// NEAREST for TEXTURE_MIN_FILTER"), so the level or bias does not matter.
    fn tex(
        &self,
        sampler: u8,
        c: [u32; 4],
        proj: bool,
        cube: bool,
        lod: super::texref::Lod,
    ) -> Result<[u32; 4], String> {
        use super::texref::{sample_2d, sample_cube, Refusal};
        let t = self
            .textures
            .get(usize::from(sampler))
            .ok_or_else(|| std::format!("no texture at sampler binding {sampler}"))?;
        if t.cube != cube {
            return Err(std::format!(
                "a {} look-up on a {} texture at sampler binding {sampler}",
                if cube { "cube" } else { "2D" },
                if t.cube { "cube" } else { "2D" }
            ));
        }
        /* The texel NEAREST reads (GL ES 2.0 3.7.6-3.7.8), through the model
         * the V3D interpreter shares; the two refusals this evaluator has
         * always named keep their words. */
        let why = |e: Refusal| match e {
            Refusal::NotFinite => "a texture coordinate that is not finite".into(),
            Refusal::TexelEdge(_) => "a texture coordinate within 1e-3 of a texel edge".into(),
            e => std::format!("{e}"),
        };
        let f = f32::from_bits;
        let v = if cube {
            sample_cube(t, f(c[0]), f(c[1]), f(c[2]), lod).map(|(v, _)| v)
        } else {
            let q = if proj { f(c[3]) } else { 1.0 };
            sample_2d(t, f(c[0]) / q, f(c[1]) / q, lod)
        };
        v.map(|v| v.map(f32::to_bits)).map_err(why)
    }

    /// A float function of one operand: the host's `std` for the oracle
    /// (its pins were taken with it), [`super::fmath`] for the capture.
    fn f1(&self, op: AluOp, x: f32) -> f32 {
        use super::fmath;
        #[cfg(any(test, feature = "host-test"))]
        if self.oracle {
            return match op {
                AluOp::Fsqrt => x.sqrt(),
                AluOp::Fexp2 => x.exp2(),
                AluOp::Flog2 => x.log2(),
                AluOp::Fsin => x.sin(),
                AluOp::Fcos => x.cos(),
                AluOp::Ffloor => x.floor(),
                AluOp::Fceil => x.ceil(),
                _ => x.trunc(),
            };
        }
        match op {
            AluOp::Fsqrt => fmath::sqrt(x),
            AluOp::Fexp2 => fmath::exp2(x),
            AluOp::Flog2 => fmath::log2(x),
            AluOp::Fsin => fmath::sin(x),
            AluOp::Fcos => fmath::cos(x),
            AluOp::Ffloor => fmath::floor(x),
            AluOp::Fceil => fmath::ceil(x),
            _ => fmath::trunc(x),
        }
    }

    /// The value, and which of its lanes are defined: a lane-wise operation
    /// defines a lane when every operand's lane is; a reduction (a dot
    /// product, `any`, `all`) defines lane 0 alone, from the lanes it read.
    /// A consumer that wants a reduction's value in every lane reads it
    /// through a splat swizzle, which is what the width table says it must
    /// do; one that does not, reads lanes 1..3 nothing defined.
    fn alu(&self, op: AluOp, srcs: [Src; 3]) -> Result<([u32; 4], [bool; 4]), &'static str> {
        use AluOp::*;
        let a = self.read(srcs[0]);
        let b = if op.arity() >= 2 { self.read(srcs[1]) } else { [0; 4] };
        let c = if op.arity() >= 3 { self.read(srcs[2]) } else { [0; 4] };
        let da = self.read_def(srcs[0]);
        let db = if op.arity() >= 2 { self.read_def(srcs[1]) } else { [true; 4] };
        let dc = if op.arity() >= 3 { self.read_def(srcs[2]) } else { [true; 4] };
        let f = |x: u32| f32::from_bits(x);
        let i = |x: u32| x as i32;
        let lanes = |g: &dyn Fn(u32, u32, u32) -> u32| -> [u32; 4] { core::array::from_fn(|k| g(a[k], b[k], c[k])) };
        let fl = |g: &dyn Fn(f32, f32, f32) -> f32| lanes(&|x, y, z| g(f(x), f(y), f(z)).to_bits());
        let il = |g: &dyn Fn(i32, i32) -> i32| lanes(&|x, y, _| g(i(x), i(y)) as u32);
        let fc = |g: &dyn Fn(f32, f32) -> bool| lanes(&|x, y, _| bits_of_bool(g(f(x), f(y))));
        let ic = |g: &dyn Fn(i32, i32) -> bool| lanes(&|x, y, _| bits_of_bool(g(i(x), i(y))));
        let dot = |n: usize| {
            let s: f32 = (0..n).map(|k| f(a[k]) * f(b[k])).sum();
            [s.to_bits(), UNSET, UNSET, UNSET]
        };
        let d: [bool; 4] = match op {
            Fdot2 | Fdot3 | Fdot4 => {
                let n = match op {
                    Fdot2 => 2,
                    Fdot3 => 3,
                    _ => 4,
                };
                [(0..n).all(|k| da[k] && db[k]), false, false, false]
            }
            Bany | Ball => [da.iter().all(|&x| x), false, false, false],
            Bcsel => core::array::from_fn(|k| da[k] && if a[k] != 0 { db[k] } else { dc[k] }),
            _ => core::array::from_fn(|k| da[k] && db[k] && dc[k]),
        };
        /* Section 5.9's undefined results are judged on the lanes the
         * result defines, never on spare lanes: a scalar `u / n` whose
         * divisor was read from a uniform block carries the block's next
         * dwords (often padding, 0) in lanes 1..3, and refusing the run for
         * them made 8 Qt Quick 3D particle pairs unmeasured (2026-09-26).
         * The spare lanes are then computed without the undefined operation
         * (UNSET), so nothing panics on them. */
        /* the capture gives them the value computed below (section 5.9:
         * undefined, not an error), as the hardware would */
        let on_defined = |v: [u32; 4], bad: &dyn Fn(u32) -> bool| self.oracle && (0..4).any(|k| d[k] && bad(v[k]));
        let v = match op {
            Fadd => fl(&|x, y, _| x + y),
            Fmul => fl(&|x, y, _| x * y),
            Ffma => fl(&|x, y, z| x * y + z),
            Fmin => fl(&|x, y, _| x.min(y)),
            Fmax => fl(&|x, y, _| x.max(y)),
            Fneg => fl(&|x, _, _| -x),
            Fabs => fl(&|x, _, _| x.abs()),
            Fsign => fl(&|x, _, _| {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }),
            Frcp => fl(&|x, _, _| 1.0 / x),
            Frsq => fl(&|x, _, _| 1.0 / self.f1(Fsqrt, x)),
            Fsqrt | Fexp2 | Flog2 => fl(&|x, _, _| self.f1(op, x)),
            /* the exact functions unless a harness asks for the shift
             * (sin(-0.0) + 0.0 would be +0.0: the default path must stay
             * bit for bit what it was) */
            Fsin | Fcos if self.trig == 0.0 => fl(&|x, _, _| self.f1(op, x)),
            Fsin | Fcos => fl(&|x, _, _| self.f1(op, x) + self.trig * x.abs().max(1.0)),
            Ffloor | Fceil | Ftrunc => fl(&|x, _, _| self.f1(op, x)),
            Ffract => fl(&|x, _, _| x - self.f1(Ffloor, x)),
            Fddx | Fddy => return Err("the evaluator has no derivatives"),
            Fdot2 => dot(2),
            Fdot3 => dot(3),
            Fdot4 => dot(4),
            Flt => fc(&|x, y| x < y),
            Fle => fc(&|x, y| x <= y),
            Feq => fc(&|x, y| x == y),
            Fne => fc(&|x, y| x != y),
            Iadd => il(&|x, y| x.wrapping_add(y)),
            Imul => il(&|x, y| x.wrapping_mul(y)),
            /* Section 5.9 leaves these undefined in both languages -- a
             * division by zero, and in ES 3.00 a shift by a negative amount
             * or by 32 or more -- so the run stops unmeasured, as the
             * unsigned forms below do. They produced 0 and a wrapped shift
             * until 2026-09-23, which ir.md already said they did not. */
            Idiv | Imod if on_defined(b, &|y| y == 0) => return Err("undefined: an integer division by zero"),
            Ishl | Ishr if on_defined(b, &|y| y >= 32) => {
                return Err("undefined: a shift by 32 or more, or by a negative amount")
            }
            Idiv => lanes(&|x, y, _| if y == 0 { UNSET } else { i(x).wrapping_div(i(y)) as u32 }),
            Imod => lanes(&|x, y, _| if y == 0 { UNSET } else { i(x).wrapping_rem(i(y)) as u32 }),
            Ineg => il(&|x, _| x.wrapping_neg()),
            Ishl => il(&|x, y| x.wrapping_shl(y as u32)),
            Ishr => il(&|x, y| x.wrapping_shr(y as u32)),
            Iand => il(&|x, y| x & y),
            Ior => il(&|x, y| x | y),
            Ixor => il(&|x, y| x ^ y),
            Ilt => ic(&|x, y| x < y),
            Ile => ic(&|x, y| x <= y),
            Ieq => ic(&|x, y| x == y),
            Ine => ic(&|x, y| x != y),
            B2f => lanes(&|x, _, _| if x != 0 { 1.0f32.to_bits() } else { 0 }),
            B2i => lanes(&|x, _, _| u32::from(x != 0)),
            F2i => lanes(&|x, _, _| f(x) as i32 as u32),
            I2f => lanes(&|x, _, _| (i(x) as f32).to_bits()),
            F2b => lanes(&|x, _, _| bits_of_bool(f(x) != 0.0)),
            I2b => lanes(&|x, _, _| bits_of_bool(x != 0)),
            Bnot => lanes(&|x, _, _| bits_of_bool(x == 0)),
            /* All four lanes of the swizzled operand, by contract
             * (vivante/builtins.rs): the caller repeats a real component
             * into the spare lanes. Folding only the recorded width would
             * make this evaluator laxer than the hardware, and it was, until
             * a bvec2 `all()` passed here that the GPU would have got
             * wrong. */
            Bany => [bits_of_bool(a.iter().any(|&x| x != 0)), UNSET, UNSET, UNSET],
            Ball => [bits_of_bool(a.iter().all(|&x| x != 0)), UNSET, UNSET, UNSET],
            Bcsel => lanes(&|x, y, z| if x != 0 { y } else { z }),

            /* GLSL ES 3.00. Where section 5.9 leaves a result UNDEFINED --
             * a shift by 32 or more, a division by zero, a float below 0
             * converted to uint -- the run stops as unmeasured instead of
             * inventing a value: a made-up value here would become a
             * reference the oracle then enforces on a back end that is free
             * to do anything. */
            Udiv | Umod if on_defined(b, &|y| y == 0) => return Err("undefined: an unsigned division by zero"),
            Udiv => lanes(&|x, y, _| x.checked_div(y).unwrap_or(UNSET)),
            Umod => lanes(&|x, y, _| x.checked_rem(y).unwrap_or(UNSET)),
            Ushr if on_defined(b, &|y| y >= 32) => return Err("undefined: a shift by 32 or more"),
            Ushr => lanes(&|x, y, _| x.checked_shr(y).unwrap_or(UNSET)),
            Ult => lanes(&|x, y, _| bits_of_bool(x < y)),
            Ule => lanes(&|x, y, _| bits_of_bool(x <= y)),
            Imin => il(&|x, y| x.min(y)),
            Imax => il(&|x, y| x.max(y)),
            Umin => lanes(&|x, y, _| x.min(y)),
            Umax => lanes(&|x, y, _| x.max(y)),
            Inot => lanes(&|x, _, _| !x),
            U2f => lanes(&|x, _, _| (x as f32).to_bits()),
            F2u if on_defined(a, &|x| f(x).is_nan() || f(x) < 0.0) => {
                return Err("undefined: a negative float converted to uint")
            }
            F2u => lanes(&|x, _, _| f(x) as u32),
        };
        Ok((v, d))
    }
}

/// `x` is finite and an f16 holds it exactly.
#[cfg(any(test, feature = "host-test"))]
fn f16_exact(x: f32) -> bool {
    let a = x.abs();
    if x == 0.0 {
        return true;
    }
    if a.is_nan() || a > 65504.0 {
        return false;
    }
    /* Normal f16: 11 significant bits; subnormal: a multiple of 2^-24. */
    if a >= 2f32.powi(-14) {
        a.to_bits() & 0x1fff == 0
    } else {
        (a * 2f32.powi(24)).fract() == 0.0
    }
}

fn bits_of_bool(b: bool) -> u32 {
    if b {
        u32::MAX
    } else {
        0
    }
}

#[cfg(test)]
mod es3_op_tests {
    use super::*;
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use ir::{InstrId, ValueId};

    /// `op` over two integer constants, run; the colour output's bits, or
    /// the reason the run stopped.
    fn run_op(op: AluOp, a: [i32; 4], b: [i32; 4]) -> Result<[u32; 4], String> {
        let instrs = [
            Op::ConstI32(a),
            Op::ConstI32(b),
            Op::Alu(op, [Src::id(ValueId(0)), Src::id(ValueId(1)), Src::id(ValueId(1))]),
            Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(2)) },
        ];
        let nodes: std::vec::Vec<Node> = (0..4).map(|i| Node::Instr(InstrId(i))).collect();
        let m = Module {
            instrs: &instrs,
            nodes: &nodes,
            body: Range { first: 0, len: 4 },
            ..Module::empty(SHC_STAGE_FRAGMENT)
        };
        run(&m, &Inputs { io: &[], uniforms: &[] }).map(|o| o.output(IoSlot::FragColor(0)).expect("stored"))
    }

    /// The GLSL ES 3.00 integer operations of IR v1, lane by lane, with
    /// operands where signed and unsigned disagree (0xffff_fffe is -2 as an
    /// int and 4294967294 as a uint).
    #[test]
    fn unsigned_operations_read_their_operands_as_unsigned() {
        let big = -2; /* 0xffff_fffe */
        let t = u32::MAX;
        assert_eq!(run_op(AluOp::Udiv, [big, 7, 9, 1], [2, 2, 3, 1]).unwrap(), [0x7fff_ffff, 3, 3, 1]);
        assert_eq!(run_op(AluOp::Umod, [big, 7, 9, 1], [5, 2, 3, 1]).unwrap(), [4294967294 % 5, 1, 0, 0]);
        assert_eq!(run_op(AluOp::Ushr, [big, 8, 1, 0], [1, 2, 0, 31]).unwrap(), [0x7fff_ffff, 2, 1, 0]);
        assert_eq!(run_op(AluOp::Ult, [big, 1, 2, 3], [1, big, 2, 4]).unwrap(), [0, t, 0, t]);
        assert_eq!(run_op(AluOp::Ule, [big, 1, 2, 3], [1, big, 2, 4]).unwrap(), [0, t, t, t]);
        assert_eq!(run_op(AluOp::Imin, [big, 1, 5, 0], [1, 2, 3, 0]).unwrap(), [big as u32, 1, 3, 0]);
        assert_eq!(run_op(AluOp::Umin, [big, 1, 5, 0], [1, 2, 3, 0]).unwrap(), [1, 1, 3, 0]);
        assert_eq!(run_op(AluOp::Imax, [big, 1, 5, 0], [1, 2, 3, 0]).unwrap(), [1, 2, 5, 0]);
        assert_eq!(run_op(AluOp::Umax, [big, 1, 5, 0], [1, 2, 3, 0]).unwrap(), [big as u32, 2, 5, 0]);
        assert_eq!(run_op(AluOp::Inot, [0, -1, 5, 0], [0; 4]).unwrap(), [t, 0, !5, t]);
        let fb = |x: f32| x.to_bits();
        assert_eq!(run_op(AluOp::U2f, [big, 3, 0, 1], [0; 4]).unwrap(), [fb(4294967294.0), fb(3.0), 0, fb(1.0)]);
    }

    /// Section 5.9's undefined results stop the run -- they are unmeasured,
    /// never a value the oracle would then hold a back end to.
    #[test]
    fn undefined_results_are_unmeasured_not_invented() {
        assert!(run_op(AluOp::Udiv, [1; 4], [1, 0, 1, 1]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::Umod, [1; 4], [0; 4]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::Ushr, [1; 4], [32, 0, 0, 0]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::F2u, [(-1.0f32).to_bits() as i32; 4], [0; 4]).unwrap_err().contains("undefined"));
        /* The signed forms, which the ES 3.00 operators now reach: an int
         * shift by 32 or by a negative amount, and an int division by 0.
         * Revert check: these produced a wrapped shift and 0 until the arms
         * above them were added. */
        assert!(run_op(AluOp::Ishl, [1; 4], [32, 0, 0, 0]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::Ishr, [1; 4], [-1, 0, 0, 0]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::Idiv, [1; 4], [1, 0, 1, 1]).unwrap_err().contains("undefined"));
        assert!(run_op(AluOp::Imod, [1; 4], [0; 4]).unwrap_err().contains("undefined"));
        assert_eq!(run_op(AluOp::Ishr, [-8, 8, 1, 0], [1, 2, 0, 31]).unwrap(), [(-4i32) as u32, 2, 1, 0]);
    }
}

/// A reduction defines lane 0 alone; reading it wider without a splat
/// reaches lanes nothing defined, and the outcome says so. This is what a
/// scalar GPU does with `faceforward`'s unsplatted `Bcsel` condition (13
/// dEQP cases wrong on the AM62P, 2026-09-24) and what the evaluator hid by
/// broadcasting the dot product into every lane.
///
/// Revert check: broadcast `dot` again (`[s.to_bits(); 4]` with all four
/// lanes defined) and the first assertion's lanes 1..3 read as defined.
#[cfg(test)]
mod defined_lane_tests {
    use super::*;
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use ir::{InstrId, ValueId};

    /// `dot(a, b)` on two vec3 constants, then the colour from it read
    /// through `swz`, and a branch on it when `branch` is set.
    fn run_dot(swz: [u8; 4], branch: bool) -> Result<Outcome, String> {
        let a = f32x4([1.0, 2.0, 3.0, 0.0]);
        let b = f32x4([1.0, 1.0, 1.0, 0.0]);
        let instrs = [
            Op::ConstF32(a),
            Op::ConstF32(b),
            Op::Alu(AluOp::Fdot3, [Src::id(ValueId(0)), Src::id(ValueId(1)), Src::id(ValueId(1))]),
            Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src { val: ValueId(2), swizzle: swz } },
        ];
        let cond = Src { val: ValueId(2), swizzle: swz };
        let nodes: std::vec::Vec<Node> = if branch {
            std::vec![
                Node::Instr(InstrId(0)),
                Node::Instr(InstrId(1)),
                Node::Instr(InstrId(2)),
                Node::If { cond, then_: Range { first: 4, len: 1 }, else_: Range { first: 4, len: 1 } },
                Node::Instr(InstrId(3)),
            ]
        } else {
            (0..4).map(|i| Node::Instr(InstrId(i))).collect()
        };
        let m = Module {
            instrs: &instrs,
            nodes: &nodes,
            /* four nodes either way: the store, or the `if` whose arms hold it */
            body: Range { first: 0, len: 4 },
            ..Module::empty(SHC_STAGE_FRAGMENT)
        };
        run(&m, &Inputs { io: &[], uniforms: &[] })
    }

    #[test]
    fn a_reduction_read_wider_than_one_lane_is_undefined_there() {
        let o = run_dot([0, 1, 2, 3], false).expect("runs");
        assert_eq!(o.output_defined(IoSlot::FragColor(0)), Some([true, false, false, false]));
        assert_eq!(o.output_f32(IoSlot::FragColor(0)).expect("stored")[0], 6.0);
        assert_eq!(o.output(IoSlot::FragColor(0)).expect("stored")[1], UNSET, "nothing defined lane 1");
        let o = run_dot([0; 4], false).expect("runs");
        assert_eq!(o.output_defined(IoSlot::FragColor(0)), Some([true; 4]), "a splat reads lane 0 into all four");
        assert_eq!(o.output_f32(IoSlot::FragColor(0)), Some([6.0; 4]));
    }

    /// Section 5.9's undefined results are judged on the lanes the result
    /// defines. A scalar division reads its divisor's spare lanes from
    /// whatever follows it (a uniform block's padding: 0), and those must
    /// not stop the run: qt3d/qtparticleshaders_10's `index /
    /// qt_countPerSlice` did, for 8 pairs (tests/v3d_sim_mesa.rs,
    /// 2026-09-26). Here the divisor is `uint(dot(...))`: lane 0 is 4 and
    /// defined, lanes 1..3 come from NaN and are 0 (or 32 for the shifts),
    /// undefined. The spare lanes come back UNSET, and nothing panics.
    /// Revert check: judge `b.contains(&0)` (or `a`/`b` over all lanes) again
    /// and each op below stops the run.
    #[test]
    fn undefined_results_are_judged_on_defined_lanes_only() {
        let run_scalar = |op: AluOp, add: [i32; 4]| -> Result<Outcome, String> {
            let instrs = [
                Op::ConstF32(f32x4([2.0, 2.0, 0.0, 0.0])),
                Op::ConstF32(f32x4([1.0, 1.0, 0.0, 0.0])),
                Op::Alu(AluOp::Fdot2, [Src::id(ValueId(0)), Src::id(ValueId(1)), Src::id(ValueId(1))]),
                /* F2u of lanes 1..3 (UNSET, a NaN) is itself judged on lane 0 */
                Op::Alu(AluOp::F2u, [Src::id(ValueId(2)), Src::id(ValueId(2)), Src::id(ValueId(2))]),
                Op::ConstI32(add),
                Op::Alu(AluOp::Iadd, [Src::id(ValueId(3)), Src::id(ValueId(4)), Src::id(ValueId(4))]),
                Op::ConstI32([17; 4]),
                Op::Alu(op, [Src::id(ValueId(6)), Src::id(ValueId(5)), Src::id(ValueId(5))]),
                Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(7)) },
            ];
            let nodes: std::vec::Vec<Node> = (0..9).map(|i| Node::Instr(InstrId(i))).collect();
            let m = Module {
                instrs: &instrs,
                nodes: &nodes,
                body: Range { first: 0, len: 9 },
                ..Module::empty(SHC_STAGE_FRAGMENT)
            };
            run(&m, &Inputs { io: &[], uniforms: &[] })
        };
        for (op, add, want) in [
            (AluOp::Udiv, [0; 4], 17 / 4),
            (AluOp::Umod, [0; 4], 17 % 4),
            (AluOp::Idiv, [0; 4], 17 / 4),
            (AluOp::Imod, [0; 4], 17 % 4),
            (AluOp::Ushr, [0, 32, 32, 32], 17 >> 4),
            (AluOp::Ishr, [0, 32, 32, 32], 17 >> 4),
            (AluOp::Ishl, [0, 32, 32, 32], 17 << 4),
        ] {
            let o = run_scalar(op, add).unwrap_or_else(|e| panic!("{op:?}: a spare lane stopped the run: {e}"));
            let v = o.output(IoSlot::FragColor(0)).expect("stored");
            assert_eq!(v[0], want, "{op:?}: lane 0");
            assert_eq!(o.output_defined(IoSlot::FragColor(0)), Some([true, false, false, false]), "{op:?}");
        }
        /* and lane 0 itself is still judged */
        let why = run_scalar(AluOp::Ushr, [28, 0, 0, 0]).expect_err("a shift by 32 in lane 0");
        assert!(why.contains("undefined"), "{why}");
    }

    /// The trig hook is off by default, bit for bit: `run` (and so every
    /// oracle that does not ask for the shift) gives exactly `x.sin()` and
    /// `x.cos()`, -0.0 included, which `sin(x) + 0.0 * k` would turn into
    /// +0.0. And the shift, when asked, moves the result.
    /// Revert check: drop the `self.trig == 0.0` arms and sin(-0.0) fails.
    #[test]
    fn the_trig_hook_is_bit_identical_when_off() {
        for x in [-0.0f32, 0.0, 0.5, -1.25, 3.0, 90.928604, 1.0e6] {
            for (op, exact) in [(AluOp::Fsin, x.sin()), (AluOp::Fcos, x.cos())] {
                let instrs = [
                    Op::ConstF32(f32x4([x; 4])),
                    Op::Alu(op, [Src::id(ValueId(0)), Src::id(ValueId(0)), Src::id(ValueId(0))]),
                    Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(1)) },
                ];
                let nodes: std::vec::Vec<Node> = (0..3).map(|i| Node::Instr(InstrId(i))).collect();
                let m = Module {
                    instrs: &instrs,
                    nodes: &nodes,
                    body: Range { first: 0, len: 3 },
                    ..Module::empty(SHC_STAGE_FRAGMENT)
                };
                let input = Inputs { io: &[], uniforms: &[] };
                let out = |t: f32| {
                    run_with_trig_error(&m, &input, &[], t).expect("runs").output(IoSlot::FragColor(0)).expect("stored")
                        [0]
                };
                let got = run(&m, &input).expect("runs").output(IoSlot::FragColor(0)).expect("stored")[0];
                assert_eq!(got, exact.to_bits(), "{op:?}({x}): the default path");
                assert_eq!(out(0.0), exact.to_bits(), "{op:?}({x}): the hook at 0");
                assert_ne!(out(1.0e-3), exact.to_bits(), "{op:?}({x}): the hook shifts");
            }
        }
    }

    #[test]
    fn a_branch_on_an_undefined_lane_stops_the_run() {
        let why = run_dot([1, 1, 1, 1], true).expect_err("lane 1 of a dot product");
        assert!(why.starts_with("unset lane:"), "{why}");
        assert!(run_dot([0; 4], true).is_ok(), "lane 0 is defined");
    }
}

#[cfg(test)]
mod texture_tests {
    use super::*;
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use ir::{InstrId, ScalarType, TexDim, TexLod, ValueId};

    /// 2x2: texel (i, j) is (i, j, 0.5, 1).
    fn tex2x2() -> Texture {
        Texture::flat(2, 2, (0..4).map(|k| [(k % 2) as f32, (k / 2) as f32, 0.5, 1.0]).collect())
    }

    /// texture2D(binding 0, (s, t)) to the colour, run.
    fn sample(textures: &[Texture], st: [f32; 2], proj_q: Option<f32>) -> Result<[f32; 4], String> {
        let instrs = [
            Op::ConstF32(f32x4([st[0], st[1], 0.0, proj_q.unwrap_or(1.0)])),
            Op::Tex {
                dim: TexDim::D2,
                shadow: false,
                proj: proj_q.is_some(),
                ret: ScalarType::F32,
                sampler: 0,
                coord: Src::id(ValueId(0)),
                lod: TexLod::Implicit,
                offset: None,
            },
            Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(1)) },
        ];
        let nodes: std::vec::Vec<Node> = (0..3).map(|i| Node::Instr(InstrId(i))).collect();
        let m = Module {
            instrs: &instrs,
            nodes: &nodes,
            body: Range { first: 0, len: 3 },
            ..Module::empty(SHC_STAGE_FRAGMENT)
        };
        run_with_textures(&m, &Inputs { io: &[], uniforms: &[] }, textures)
            .map(|o| o.output_f32(IoSlot::FragColor(0)).expect("stored"))
    }

    /// A look-up of `dim` at `c` with the level of detail `lod` (a constant
    /// operand when `Some`, implicit when `None`).
    fn sample_at(textures: &[Texture], dim: TexDim, c: [f32; 3], lod: Option<f32>) -> Result<[f32; 4], String> {
        let instrs = [
            Op::ConstF32(f32x4([c[0], c[1], c[2], 1.0])),
            Op::ConstF32(f32x4([lod.unwrap_or(0.0); 4])),
            Op::Tex {
                dim,
                shadow: false,
                proj: false,
                ret: ScalarType::F32,
                sampler: 0,
                coord: Src::id(ValueId(0)),
                lod: lod.map_or(TexLod::Implicit, |_| TexLod::Lod(Src::id(ValueId(1)))),
                offset: None,
            },
            Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(2)) },
        ];
        let nodes: std::vec::Vec<Node> = (0..4).map(|i| Node::Instr(InstrId(i))).collect();
        let m = Module {
            instrs: &instrs,
            nodes: &nodes,
            body: Range { first: 0, len: 4 },
            ..Module::empty(SHC_STAGE_FRAGMENT)
        };
        run_with_textures(&m, &Inputs { io: &[], uniforms: &[] }, textures)
            .map(|o| o.output_f32(IoSlot::FragColor(0)).expect("stored"))
    }

    /// Three levels, level k's every texel of face f all 10k + f.
    fn mips(cube: bool) -> Texture {
        use super::super::texref::{Level, MinFilter};
        let lv = |k: u32| {
            let w = 4 >> k;
            let faces = (0..if cube { 6 } else { 1 }).map(|f| std::vec![[(10 * k + f) as f32; 4]; (w * w) as usize]);
            Level { width: w, height: w, faces: faces.collect() }
        };
        Texture { cube, min: MinFilter::NearestMipmapNearest, levels: (0..3).map(lv).collect() }
    }

    /// An explicit level of detail (the operand of `TexLod::Lod`) reads the
    /// level dEQP picks, 2D and cube; the implicit one on three levels, a
    /// cube look-up on a 2D texture and gradients stop by name.
    #[test]
    fn an_explicit_lod_and_a_cube_face_are_read_through_the_shared_model() {
        let t = [mips(false)];
        assert_eq!(sample_at(&t, TexDim::D2, [0.3, 0.3, 0.0], Some(0.9)).unwrap()[0], 10.0);
        assert_eq!(sample_at(&t, TexDim::D2, [0.3, 0.3, 0.0], Some(1.8)).unwrap()[0], 20.0);
        let e = sample_at(&t, TexDim::D2, [0.3, 0.3, 0.0], None).unwrap_err();
        assert!(e.contains("an implicit level of detail on a texture of 3 levels"), "{e}");
        let c = [mips(true)];
        assert_eq!(sample_at(&c, TexDim::Cube, [0.2, -1.0, 0.3], Some(0.0)).unwrap()[0], 3.0, "-Y");
        assert_eq!(sample_at(&c, TexDim::Cube, [0.2, 0.3, -1.0], Some(0.9)).unwrap()[0], 15.0, "-Z, level 1");
        let e = sample_at(&t, TexDim::Cube, [0.2, 0.3, -1.0], Some(0.0)).unwrap_err();
        assert!(e.contains("a cube look-up on a 2D texture"), "{e}");
    }

    /// Equations 3.14/3.15 with the CLAMP_TO_EDGE clamp: inside, past both
    /// edges, and through texture2DProj's divisor.
    #[test]
    fn nearest_clamp_to_edge_picks_the_texel() {
        let t = [tex2x2()];
        assert_eq!(sample(&t, [0.25, 0.25], None).unwrap(), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(sample(&t, [0.75, 0.25], None).unwrap(), [1.0, 0.0, 0.5, 1.0]);
        assert_eq!(sample(&t, [0.3, 0.9], None).unwrap(), [0.0, 1.0, 0.5, 1.0]);
        assert_eq!(sample(&t, [-3.0, 7.0], None).unwrap(), [0.0, 1.0, 0.5, 1.0]);
        assert_eq!(sample(&t, [1.0, 1.0], None).unwrap(), [1.0, 1.0, 0.5, 1.0]);
        assert_eq!(sample(&t, [1.5, 0.5], Some(2.0)).unwrap(), [1.0, 0.0, 0.5, 1.0]);
    }

    /// A coordinate on a texel edge is its own outcome, never a texel;
    /// 0.5 is the edge between the two columns of a 2-wide texture.
    #[test]
    fn a_texel_edge_is_refused() {
        let t = [tex2x2()];
        let e = sample(&t, [0.5, 0.25], None).unwrap_err();
        assert_eq!(e, "a texture coordinate within 1e-3 of a texel edge");
        assert!(sample(&t, [0.25, 0.5004], None).is_err());
        assert!(sample(&t, [0.25, 0.502], None).is_ok());
    }

    /// textureSize(s, 0) of a 2D texture is its size; another level stops.
    #[test]
    fn texture_size_is_level_zeros() {
        let size = |lod: i32| {
            let instrs = [
                Op::ConstI32([lod; 4]),
                Op::TexSize { sampler: 0, dim: TexDim::D2, lod: Src::id(ValueId(0)) },
                Op::StoreOutput { slot: IoSlot::FragColor(0), src: Src::id(ValueId(1)) },
            ];
            let nodes: std::vec::Vec<Node> = (0..3).map(|i| Node::Instr(InstrId(i))).collect();
            let m = Module {
                instrs: &instrs,
                nodes: &nodes,
                body: Range { first: 0, len: 3 },
                ..Module::empty(SHC_STAGE_FRAGMENT)
            };
            let t = [Texture::flat(2, 1, std::vec![[0.0; 4]; 2])];
            run_with_textures(&m, &Inputs { io: &[], uniforms: &[] }, &t).map(|o| o.output(IoSlot::FragColor(0)))
        };
        assert_eq!(size(0).unwrap(), Some([2, 1, 0, 0]));
        assert!(size(1).is_err());
    }

    /// run() keeps stopping at a look-up; a missing binding, a texel an
    /// f16 cannot hold and a non-finite coordinate stop the run.
    #[test]
    fn what_is_not_supplied_stops_the_run() {
        assert_eq!(sample(&[], [0.25, 0.25], None).unwrap_err(), "the evaluator has no textures");
        let mut t = tex2x2();
        t.levels[0].faces[0][0][0] = 0.1;
        assert_eq!(sample(&[t], [0.25, 0.25], None).unwrap_err(), "a texel that is not exact in f16");
        assert!(sample(&[tex2x2()], [f32::NAN, 0.25], None).is_err());
        let two = [tex2x2(), tex2x2()];
        assert!(sample(&two[1..], [0.25, 0.25], None).is_ok());
        let bad = [Texture::flat(2, 2, std::vec![])];
        assert!(sample(&bad, [0.25, 0.25], None).is_err());
    }
}
