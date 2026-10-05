// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! IR to USC instruction groups, in virtual registers.
//!
//! The PowerVR half of what `super::super::select` does for the GC7000, and
//! deliberately the same shape: walk the IR, resolve every operand, emit one
//! description per operation, and let the machine half below the seam
//! ([`super::sel::SelGroup`]) decide phases, slots and internal wiring.
//!
//! # The one structural difference: this machine is scalar
//!
//! The GC7000 is a vec4 machine, so one IR operation is one instruction with
//! a write mask. **The USC is scalar**, so one IR operation over an
//! `n`-component value is `n` groups, one per component. Every value
//! therefore has up to four virtual registers rather than one, and a source
//! names a component.
//!
//! `ir.md` chose the vector IR knowing this: "Scalarising a vector IR for the
//! USC is a mechanical per-component expansion. Going the other way ... is a
//! pattern-matching pass over sequences that no longer say they belong
//! together." This file is that mechanical expansion, and it is about sixty
//! lines of it.
//!
//! # What it refuses, and why each refusal is here rather than a guess
//!
//! Three things are not settled at the seam yet, and each would be a guess
//! about somebody else's half:
//!
//! * **`Fdot2`/`Fdot3`/`Fdot4`.** `builtins::shape` calls them
//!   `Shape::Lower("fmul then fmad per component")`, and `sel.rs` says a
//!   `Shape::Lower` expands "inside the builder, never here". Those two are
//!   consistent only if the builder can see the components, and it cannot:
//!   `SelGroup::srcs` is three scalars and `Fdot3` needs six. Either
//!   selection expands it -- which is what `ir.md` predicted -- or the seam
//!   grows a vector form. Asked of pvrgpu-platform; refused until answered.
//! * **Fragment varyings.** A varying arrives through the coefficient file
//!   and may need an iterator (`fitr`) the builder emits. Until that is said,
//!   a fragment shader that reads a varying is refused by name.
//! * **Vertex outputs.** `abi.md`: the program writes them with the
//!   `uvsw_write` family, which is a backend operation and not an [`AluOp`],
//!   so it cannot be a `SelGroup` at all. [`Selected::outputs`] hands the
//!   builder the per-component sources instead and the builder emits the
//!   writes -- the same split the GC7000 side uses for its io lists.
//!
//! A refusal that names the open question is worth more than code written
//! against a guess: the refusal is a gap somebody can see.

use super::sel::{Bank, MachineOp, Mods, OpType, PredArm, SelDst, SelGroup, SelOp, SelSrc, SmpLod, Trig, VReg};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir;
use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// What selection produces for the USC builder.
pub struct Selected<'a> {
    /// The program, one group per operation per component.
    pub groups: &'a [SelGroup],
    /// How many virtual registers were handed out; the allocator sizes its
    /// map from this.
    pub vregs: u32,
    /// Where each written output's components come from, in order.
    ///
    /// NOT groups: a vertex output is a `uvsw_write` and the fragment colour
    /// is a move to `po0`, neither of which is an [`ir::Op`]. The builder
    /// emits them, and this says what to write.
    pub outputs: &'a [(ir::IoSlot, [Option<SelSrc>; 4])],
    /// Which virtual register each fragment varying's components are read
    /// into, before the first group runs.
    ///
    /// The mirror image of `outputs`, and for the same reason. A fragment
    /// shader does not read a varying out of a register file: the coefficient
    /// file holds the plane equation and an iterator evaluates it at this
    /// fragment. That iterator is not an [`ir::Op`] either, so the builder
    /// emits it, one per component, writing into the [`VReg`] named here --
    /// after which every group reads an ordinary virtual register and nothing
    /// above this line knows the coefficient file exists.
    ///
    /// Ruled 2026-09-22: selection could instead have named [`Bank::Coeff`]
    /// directly in a `SelSrc::Fixed`, the way it names the shared file for a
    /// uniform. It does not, because a uniform really is readable as an
    /// operand and a varying is not -- it has to be evaluated first. Letting
    /// selection write `Bank::Coeff` as a source would make every group
    /// carrying one silently wrong.
    ///
    /// Empty in a vertex shader, which has no varying reads.
    pub inputs: &'a [(ir::IoSlot, [Option<VReg>; 4])],
    /// A fragment shader that discards: the boolean (all ones or zero) that
    /// says this fragment is killed, after the whole epilogue pco's
    /// `pco_nir_pfo` builds (the shader's own discards, alpha to coverage
    /// and the sample mask). The builder turns it into the ISP feedback.
    /// Like an output, it is read by the builder rather than by a group.
    pub discard: Option<SelSrc>,
    /// A shader that discards: the colour alpha BEFORE the control word's
    /// alpha-to-one, and the flag that says to apply it. `outputs` has it
    /// applied; a blend variant blends the alpha before and applies the flag
    /// to the blended colour, as pco's `pco_nir_pfo` orders them.
    pub alpha_to_one: Option<(SelSrc, SelSrc)>,
}

/// Where the per-draw control word sits relative to the uniform block, and
/// its fields: pco's `PVR_FS_META_*` (Mesa `src/imagination/common/
/// pvr_iface.h`, MIT). The driver writes `SHC_FS_META_DEFAULT` there.
const META_ALPHA_TO_ONE: (u32, u32) = (0, 1);
const META_SAMPLE_MASK: (u32, u32) = (9, 16);
const META_ALPHA_TO_COVERAGE: (u32, u32) = (25, 1);
/// `samp_num`: the sample this invocation shades (`pco_isa.py`).
const SR_SAMP_NUM: u16 = 53;

/// The special registers `gl_FragCoord.xy` is read from (Mesa
/// `pco_isa.py`): the pixel centre, and the sample position.
const SR_X_P: u16 = 97;
const SR_X_S: u16 = 98;
const SR_Y_P: u16 = 100;
const SR_Y_S: u16 = 101;

/// `face_ccw`: the ISP's facing bit, nonzero on an anticlockwise
/// primitive (Mesa `pco_isa.py`).
const SR_FACE_CCW: u16 = 44;

/// Does `op` sample with the implicit level at a smooth varying read as
/// it is, so that [`Self::plane_gradients`] can give it exact gradients
/// (`shaderc::pvr_proj_lod`)? The number of coordinates when it does.
/// Asked by selection, and by the online back end at link
/// ([`samples_with_plane_gradients`]).
pub fn gradients_for(m: &ir::Module<'_>, op: &ir::Op) -> Option<u32> {
    let ir::Op::Tex {
        dim,
        shadow: false,
        proj: false,
        ret: ir::ScalarType::F32,
        coord,
        lod: ir::TexLod::Implicit,
        offset: None,
        sampler,
        ..
    } = *op
    else {
        return None;
    };
    let mode = crate::shaderc::proj_lod_mode();
    /* the program's vertex shader writes a constant W: every varying is
     * affine and the TPU's own level is exact (`shaderc::fs_w_constant`) */
    if crate::shaderc::fs_w_constant() {
        return None;
    }
    /* a sampler-state variant for a sampler with no mipmap filter: the
     * level cannot change a texel, so the TPU's own is exact
     * (`shaderc::LodKey::mip_free`) */
    if sampler < 32 && crate::shaderc::compile_lod_key().mip_free & (1 << sampler) != 0 {
        return None;
    }
    let n = match dim {
        ir::TexDim::D2 if mode >= 1 => 2,
        ir::TexDim::Cube if mode >= 2 && crate::shaderc::native_cube_enabled() => 3,
        _ => return None,
    };
    if m.stage != SHC_STAGE_FRAGMENT {
        return None;
    }
    let ir::Op::LoadInput { slot: ir::IoSlot::Varying(loc), .. } = *m.def(coord.val)? else {
        return None;
    };
    let d = m.inputs.iter().find(|d| d.kind == ir::IoKind::Varying && d.location == loc)?;
    (d.interp == ir::INTERP_SMOOTH).then_some(n)
}

/// Does this fragment shader sample with exact plane gradients, so that a
/// program whose vertex shader writes a constant W compiles it again
/// without them (the online back end's link)?
pub fn samples_with_plane_gradients(m: &ir::Module<'_>) -> bool {
    m.instrs.iter().any(|op| gradients_for(m, op).is_some())
}

/// Does this fragment shader read `gl_FragCoord`?
pub fn reads_frag_coord(m: &ir::Module<'_>) -> bool {
    m.stage == SHC_STAGE_FRAGMENT
        && m.instrs.iter().any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::FragCoord, .. }))
}

/// Where the driver's `gl_FragCoord.y` pair (scale, offset) sits in the
/// shared file, before the texture descriptors move it: straight after the
/// uniforms, when the shader reads `gl_FragCoord`.
pub fn ytf_dword(m: &ir::Module<'_>) -> Option<u16> {
    reads_frag_coord(m).then_some(m.uniform_dwords)
}

/// Where the per-draw control word sits, likewise: after the uniforms, and
/// after the `gl_FragCoord` pair when there is one.
pub fn meta_dword(m: &ir::Module<'_>) -> u16 {
    m.uniform_dwords + if reads_frag_coord(m) { 2 } else { 0 }
}

/// Does this fragment shader read `gl_FrontFacing`?
pub fn reads_front_facing(m: &ir::Module<'_>) -> bool {
    m.stage == SHC_STAGE_FRAGMENT
        && m.instrs.iter().any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::FrontFacing, .. }))
}

/// Where the driver's front-face op sits (the record's
/// `fs_front_face_off1` less one, before the texture descriptors move it):
/// after the uniforms, the `gl_FragCoord` pair and the control word, each
/// when the shader has one.
pub fn ff_dword(m: &ir::Module<'_>) -> Option<u16> {
    reads_front_facing(m).then(|| meta_dword(m) + u16::from(Sel::discards(m)))
}

/// Why selection could not produce a program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectError {
    /// An IR construct this does not handle yet, named.
    Unsupported(&'static str),
    /// A source naming a value no instruction produced.
    UndefinedValue(ir::ValueId),
    /// The scratch arena ran out.
    OutOfScratch,
}

impl From<ArenaError> for SelectError {
    fn from(_: ArenaError) -> Self {
        SelectError::OutOfScratch
    }
}

/// Components per value, and so virtual registers per value.
const LANES: u32 = 4;

/// `Sel::loaded` before the load ran.
const UNLOADED: VReg = VReg(u32::MAX);

/// A float constant with a source modifier applied: floor, then abs, then
/// negate. Floor is computed without `libm` (this crate is `no_std`): a
/// float of magnitude 2^23 or more, or a NaN or infinity, is already
/// integral; below that the truncation through `i32` is exact, and one is
/// subtracted for a negative non-integer.
fn fold_mods(bits: u32, m: Mods) -> u32 {
    let mut b = bits;
    if m.flr {
        let f = f32::from_bits(b);
        if f.is_finite() && f.abs() < 8_388_608.0 {
            let t = f as i32 as f32;
            let r = if t > f { t - 1.0 } else { t };
            /* floor(-0.5) is -1, floor(-0.0) is -0.0: keep the sign of a zero. */
            b = if r == 0.0 { (0.0f32).to_bits() | (b & 0x8000_0000) } else { r.to_bits() };
        }
    }
    if m.abs {
        b &= 0x7fff_ffff;
    }
    if m.neg {
        b ^= 0x8000_0000;
    }
    b
}

/// A look-up's gradients: d/dx and d/dy of up to three coordinates.
type Gradients = [[Option<SelSrc>; 2]; 3];

struct Sel<'a> {
    m: &'a ir::Module<'a>,
    /// The attribute locations that get registers (`abi::attribs_placed`):
    /// a location's vertex-input registers are its rank among them.
    attribs_read: u32,
    arena: &'a Arena<'a>,
    /// The register each local slot's components currently live in.
    ///
    /// Four per slot. `if_convert` redirects them while it selects an arm --
    /// the same table the GC7000 side keeps (`src/shaderc/online/select.rs`)
    /// and for the same reason: every read and every write of a slot goes
    /// through `local_reg`, and one of them missing the redirection is a
    /// shader that computes the wrong thing without failing.
    locals_in: &'a mut [VReg],
    /// By value and lane: the register a `LoadLocal` read, fixed when it
    /// ran. A load's value is the slot's value AT the load (the IR is SSA
    /// in straight-line code); resolving it to the slot's register at each
    /// USE read whatever a later store put there -- `x = i++` got the
    /// incremented `i`, and `gl_FragColor = c; c = ...;` the new `c`.
    loaded: &'a mut [VReg],
    /// The next virtual register to hand out, for scratch this file invents.
    next_vreg: u32,
    /// Where a texture sample writes its four lanes, when not into its own
    /// value's registers: a shadow look-up samples the depth into a scratch
    /// block through the plain look-up's arm, then compares
    /// ([`Sel::shadow_compare`]). `None` everywhere else.
    tex_dst: Option<VReg>,
    groups: AVec<'a, SelGroup>,
    inputs: AVec<'a, (ir::IoSlot, [Option<VReg>; 4])>,
    outputs: AVec<'a, (ir::IoSlot, [Option<SelSrc>; 4])>,
    /// See [`Selected::discard`].
    discard: Option<SelSrc>,
    /// See [`Selected::alpha_to_one`].
    alpha_to_one: Option<(SelSrc, SelSrc)>,
    /// The loops the unroller kept as loops (`unroll::Trip`).
    trips: &'a [super::unroll::Trip],
    /// `gl_FragCoord.x` and `.y`, computed once at the program's start
    /// (`frag_coord_prologue`).
    frag_coord: [Option<VReg>; 2],
    /// `gl_PointCoord.st`, filled by the chain's point-sprite iterator.
    point_coord: [Option<VReg>; 2],
    /// The execution mask counter, once a loop has needed it (set up once,
    /// before the first such loop: `MachineOp::CndInit`).
    emc: Option<VReg>,
    /// `gl_FrontFacing`, computed once at the program's start
    /// (`front_face_prologue`).
    front_facing: Option<VReg>,
    /// `gl_FragCoord.z` and `.w`, filled by the chain's plane iterators.
    frag_zw: [Option<VReg>; 2],
    /// Kept loops the walk is inside ([`Self::loop_`]): a value computed at
    /// depth 0 is computed on every path, before everything after it.
    loop_depth: u32,
    /// 1/Q and W's two slopes, once computed at loop depth 0
    /// ([`Self::plane_gradients`]).
    grad_w: Option<[SelSrc; 3]>,
    /// Gradients computed at loop depth 0, by varying location and swizzle,
    /// for the next look-up of the same coordinate to reuse (each read of a
    /// varying is a value of its own, so the key is the varying).
    grad_cache: [Option<(u8, [u8; 4], Gradients)>; 4],
}

impl<'a> Sel<'a> {
    /// The register a value's component lives in.
    ///
    /// Four per value whether the value uses four or one. Dense numbering
    /// would save the allocator nothing -- it maps what it is given -- and
    /// costs this file the one property that makes it readable: `VReg` is a
    /// pure function of `(value, component)`, so no table can disagree with
    /// it.
    fn reg(v: ir::ValueId, lane: u32) -> VReg {
        VReg(v.0 * LANES + lane)
    }

    /// The register block a declared fragment input lands in.
    ///
    /// After the values and the local slots. A varying gets ONE block however
    /// many times the shader reads it, because the builder iterates it once;
    /// giving each read its own would be one iterator per read.
    fn varying_reg(&self, idx: usize, lane: u32) -> VReg {
        VReg((self.m.instrs.len() as u32 + Self::slots(self.m) + idx as u32) * LANES + lane)
    }

    /// The local slots: the module's, and in a shader that discards one
    /// more after them, the discard flag. A `discard` is a store of `true`
    /// to it, so an `if` joins it like any other slot and the flag at the
    /// end is whether this fragment was discarded on the path it took.
    fn slots(m: &ir::Module<'_>) -> u32 {
        m.locals.len() as u32 + u32::from(Self::discards(m))
    }

    /// A fragment shader that discards, or one compiled for the draw's
    /// sample state ([`crate::shaderc::LodKey::sample_kill`]): both carry
    /// the epilogue, whose sample mask and alpha-to-coverage kill samples.
    fn discards(m: &ir::Module<'_>) -> bool {
        m.stage == SHC_STAGE_FRAGMENT
            && (crate::shaderc::compile_lod_key().sample_kill
                || m.instrs.iter().any(|op| matches!(op, ir::Op::Discard)))
    }

    /// The discard flag's slot, in a shader that discards.
    fn discard_local(&self) -> Option<ir::LocalId> {
        Self::discards(self.m).then_some(ir::LocalId(self.m.locals.len() as u32))
    }

    /// Which declaration a varying location is, by position in the module's
    /// input list -- the same question `Selected::inputs` answers, asked once.
    fn varying_index(&self, loc: u8) -> Option<usize> {
        self.m.inputs.iter().position(|d| d.kind == ir::IoKind::Varying && d.location == loc)
    }

    /// Fill [`Selected::inputs`] for every varying the shader reads.
    ///
    /// Declared and never read gets no block and no row: the builder would
    /// otherwise emit an iterator for a coefficient nothing consumes.
    fn declare_inputs(&mut self) -> Result<(), SelectError> {
        for (idx, d) in self.m.inputs.iter().enumerate() {
            if d.kind != ir::IoKind::Varying {
                continue;
            }
            let read = self
                .m
                .instrs
                .iter()
                .any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::Varying(l), .. } if *l == d.location));
            if !read {
                continue;
            }
            let mut regs: [Option<VReg>; 4] = [None; 4];
            for (lane, r) in regs.iter_mut().enumerate().take(d.components as usize) {
                *r = Some(self.varying_reg(idx, lane as u32));
            }
            self.inputs.push(self.arena, (ir::IoSlot::Varying(d.location), regs))?;
        }
        /* `gl_PointCoord`: two registers the chain's point-sprite iterator
         * fills, like a varying's. */
        if self.m.stage == SHC_STAGE_FRAGMENT
            && self.m.instrs.iter().any(|op| matches!(op, ir::Op::LoadInput { slot: ir::IoSlot::PointCoord, .. }))
        {
            let (s, t) = (VReg(self.next_vreg), VReg(self.next_vreg + 1));
            self.next_vreg += 2;
            self.point_coord = [Some(s), Some(t)];
            self.inputs.push(self.arena, (ir::IoSlot::PointCoord, [Some(s), Some(t), None, None]))?;
        }
        /* `gl_FragCoord.z` and `.w`: iterated with no divide from the Z and
         * W coefficient sets (Mesa pco_trans_nir.c trans_load_input_fs,
         * VARYING_SLOT_POS, NOPERSPECTIVE), registers the chain fills like a
         * varying's. Both are declared; `prune_frag_zw` drops the one the
         * program does not read, so only what is read is iterated, as pco
         * does. */
        if reads_frag_coord(self.m) {
            let (z, w) = (VReg(self.next_vreg), VReg(self.next_vreg + 1));
            self.next_vreg += 2;
            self.frag_zw = [Some(z), Some(w)];
            self.inputs.push(self.arena, (ir::IoSlot::FragCoord, [None, None, Some(z), Some(w)]))?;
        }
        /* The W plane's value (gl_FragCoord.w's iterate), which exact
         * gradients divide by (`plane_gradients`), when the program does not
         * read gl_FragCoord itself. */
        if self.frag_zw[1].is_none() && self.m.instrs.iter().any(|op| self.gradients_for(op).is_some()) {
            let w = VReg(self.next_vreg);
            self.next_vreg += 1;
            self.frag_zw = [None, Some(w)];
            self.inputs.push(self.arena, (ir::IoSlot::FragCoord, [None, None, None, Some(w)]))?;
        }
        Ok(())
    }

    /// [`gradients_for`] for this module.
    fn gradients_for(&self, op: &ir::Op) -> Option<u32> {
        gradients_for(self.m, op)
    }

    /// Exact per-pixel gradients of the `n` coordinates of `coord`, a smooth
    /// varying (`gradients_for`), as the [`SelOp::SampleGrad`] groups a
    /// [`SmpLod::Gradients`] sample takes.
    ///
    /// The iterator gives u = P / Q, P the component's plane and Q the W
    /// plane (`ditrp`), both linear in the pixel's x and y: so du/dx =
    /// (A_P - u A_Q) / Q and du/dy = (B_P - u B_Q) / Q, from the planes'
    /// A and B coefficients (`SelOp::PlaneCoeff`) and Q iterated with no
    /// divide (gl_FragCoord.w's register). This is the derivative dEQP's
    /// reference takes at the pixel; the TPU's own takes one per 2x2 quad.
    fn plane_gradients(&mut self, coord: ir::Src, n: u32) -> Result<(), SelectError> {
        use ir::AluOp::{Ffma, Fmul, Frcp};
        /* Computed at loop depth 0, a value is computed on every path before
         * all that follows (an `if` is if-converted: both arms run), so a
         * later look-up of the same coordinate reuses it; inside a kept loop
         * it is computed afresh. */
        let top = self.loop_depth == 0;
        let Some(ir::Op::LoadInput { slot: ir::IoSlot::Varying(loc), .. }) = self.m.def(coord.val).copied() else {
            return Err(SelectError::Unsupported("exact gradients of a value that is not a varying"));
        };
        let hit = self
            .grad_cache
            .iter()
            .flatten()
            .find(|(l, sw, _)| top && *l == loc && *sw == coord.swizzle)
            .map(|(_, _, d)| *d);
        let all = match hit {
            Some(d) => d,
            None => {
                let [rq, wa, wb] = match self.grad_w.filter(|_| top) {
                    Some(w) => w,
                    None => {
                        let q =
                            self.frag_zw[1].ok_or(SelectError::Unsupported("exact gradients without the W plane"))?;
                        let q = SelSrc::Virtual { v: q, mods: Mods::default() };
                        let rq = self.fresh_typed(SelOp::Alu(Frcp), op_type(Frcp), [Some(q), None, None])?;
                        let wa = self.fresh_typed(SelOp::PlaneCoeff { term: 0 }, OpType::F32, [None; 3])?;
                        let wb = self.fresh_typed(SelOp::PlaneCoeff { term: 1 }, OpType::F32, [None; 3])?;
                        if top {
                            self.grad_w = Some([rq, wa, wb]);
                        }
                        [rq, wa, wb]
                    }
                };
                /* A - u A_Q as one fmad, -u * A_Q + A; then times 1/Q */
                let mut all = [[None; 2]; 3];
                for (k, d) in all.iter_mut().enumerate().take(n as usize) {
                    let c = self.src(coord, k as u32)?;
                    let SelSrc::Virtual { v, mods } = c else {
                        return Err(SelectError::Unsupported("a varying component that is not a register"));
                    };
                    let neg_c = SelSrc::Virtual { v, mods: Mods { neg: !mods.neg, ..mods } };
                    for (term, wq) in [(0u8, wa), (1, wb)] {
                        let pa = self.fresh_typed(SelOp::PlaneCoeff { term }, OpType::F32, [Some(c), None, None])?;
                        let e = self.fresh_typed(SelOp::Alu(Ffma), op_type(Ffma), [Some(neg_c), Some(wq), Some(pa)])?;
                        d[usize::from(term)] =
                            Some(self.fresh_typed(SelOp::Alu(Fmul), op_type(Fmul), [Some(e), Some(rq), None])?);
                    }
                }
                if top {
                    if let Some(slot) = self.grad_cache.iter_mut().find(|x| x.is_none()) {
                        *slot = Some((loc, coord.swizzle, all));
                    }
                }
                all
            }
        };
        /* the markers back to back: nothing may come between them and their
         * sample (`eliminate_dead`) */
        for d in &all[..n as usize] {
            self.emit(SelGroup { op: SelOp::SampleGrad, ty: OpType::F32, dst: None, srcs: [d[0], d[1], None] })?;
        }
        Ok(())
    }

    /// Drop the `gl_FragCoord.z` / `.w` inputs nothing reads after dead-code
    /// removal (the prologue reads x and y; a shader reading only those
    /// iterates neither plane).
    fn prune_frag_zw(&mut self) {
        let groups = self.groups.as_slice();
        let outs = self.outputs.as_slice();
        let read = |v: VReg| {
            let hit = |s: &SelSrc| matches!(s, SelSrc::Virtual { v: x, .. } if *x == v);
            groups.iter().any(|g| g.srcs.iter().flatten().any(hit))
                || outs.iter().any(|(_, c)| c.iter().flatten().any(hit))
                || self.discard.iter().any(hit)
        };
        let mut lanes = [None; 4];
        if let Some((_, l)) = self.inputs.as_slice().iter().find(|(slot, _)| *slot == ir::IoSlot::FragCoord) {
            lanes = *l;
        }
        for lane in &mut lanes {
            if lane.is_some_and(|v| !read(v)) {
                *lane = None;
            }
        }
        if let Some((_, l)) = self.inputs.as_mut_slice().iter_mut().find(|(slot, _)| *slot == ir::IoSlot::FragCoord) {
            *l = lanes;
        }
    }

    /// Bounds-checked rather than indexed: a slot the module does not declare
    /// is malformed IR, and a compiler that panics on malformed input is a
    /// compiler that takes the whole process down instead of returning a log
    /// line.
    fn local_reg(&self, local: ir::LocalId, lane: u32) -> Result<VReg, SelectError> {
        self.locals_in
            .get(local.0 as usize * LANES as usize + lane as usize)
            .copied()
            .ok_or(SelectError::Unsupported("a local slot the module does not declare"))
    }

    /// The register a slot's component lives in when nothing redirects it.
    fn local_home(m: &ir::Module<'_>, local: u32, lane: u32) -> VReg {
        VReg((m.instrs.len() as u32 + local) * LANES + lane)
    }

    /// One component of a source operand, with modifiers folded.
    ///
    /// `lane` is the DESTINATION component; the swizzle says which of the
    /// source's components it takes. Same rule as the GC7000 side and as
    /// `ir.md`'s `StoreLocal` note, because getting it backwards permutes a
    /// value silently.
    fn src(&self, s: ir::Src, lane: u32) -> Result<SelSrc, SelectError> {
        self.src_mods(s, lane, Mods::default())
    }

    fn src_mods(&self, s: ir::Src, lane: u32, mods: Mods) -> Result<SelSrc, SelectError> {
        let pick = u32::from(s.swizzle[lane as usize % 4]);
        let def = self.m.def(s.val).ok_or(SelectError::UndefinedValue(s.val))?;
        match *def {
            /* An immediate is an operand on this machine (`bbyp0bm_imm32`,
             * 33,013 in the corpus), so a constant costs no instruction and
             * no constant-file entry. */
            /* `SelSrc::Imm` carries no modifiers, so a modifier reaching a
             * constant is APPLIED to it here: `x - 1.0` arrives as
             * `x + neg(1.0)`, and before this the `neg` was dropped and the
             * program computed `x + 1.0` -- found by running our bytes
             * against pco's on main.vert (usc-backend, 2026-09-23). Exact:
             * floor of a float constant, then abs and negate as sign-bit
             * operations, the order a source modifier applies in. */
            ir::Op::ConstF32(v) => Ok(SelSrc::Imm(fold_mods(v[pick as usize % 4], mods))),
            ir::Op::ConstI32(v) if mods == Mods::default() => Ok(SelSrc::Imm(v[pick as usize % 4] as u32)),
            /* A bool is all-ones, the same convention both machines use. */
            ir::Op::ConstBool(v) if mods == Mods::default() => {
                Ok(SelSrc::Imm(if v[pick as usize % 4] { u32::MAX } else { 0 }))
            }
            ir::Op::ConstI32(_) | ir::Op::ConstBool(_) => {
                Err(SelectError::Unsupported("a float modifier on an integer or boolean constant"))
            }
            /* The shared file is readable as a source, so a uniform read is
             * folded into its consumer and produces no group of its own.
             *
             * **`reg` is the dword's offset in the UNIFORM BLOCK, not an
             * absolute shared-file register.** On this machine the sampler
             * descriptors sit at sh0 and the uniforms follow (`abi.md`:
             * "20-dword descriptors at sh0.., uniforms after"), so the base
             * depends on how many samplers the program keeps -- which is
             * decided below this seam, when the texture path exists. Whoever
             * emits adds the base.
             *
             * Today the two are the same number for every program that
             * selects, because a program with a sampler is refused at its
             * first fetch. That makes this a convention nobody can currently
             * violate and therefore one worth writing down before the texture
             * path makes it violable. Asked of pvrgpu-platform. */
            ir::Op::LoadUniform { dword_off, .. } => {
                Ok(SelSrc::Fixed { bank: Bank::Shared, reg: dword_off.wrapping_add(pick as u16), mods })
            }
            ir::Op::LoadInput { slot, .. } => self.input_src(slot, pick, mods),
            /* The three modifiers are encoded on the operand, so a value
             * produced by one of them IS its operand, carrying the modifier.
             * `builtins::shape` calls these `Shape::Modifier` for exactly
             * this reason; emitting a group for them would cost an `mbyp`
             * each and the corpus shows none. */
            /* The walk goes from the outermost operation in, so `mods` are
             * applied AFTER the one met here, and the hardware applies a
             * set in one order: floor, abs, negate. An inner negate under an
             * outer abs vanishes (|-x| = |x|); under an outer floor, and an
             * inner abs under an outer floor, no set says it -- `floor(-x)`
             * folded to `-floor(x)` and `floor(abs(x))` to `abs(floor(x))`,
             * found by running `fract(-y)` against the IR. Refused by name
             * until a consumer can ask for the value to be computed. */
            ir::Op::Alu(ir::AluOp::Fneg, ref a) => {
                if mods.flr {
                    return Err(SelectError::Unsupported("a floor of a negated value: no source modifier set says it"));
                }
                let neg = if mods.abs { mods.neg } else { !mods.neg };
                self.src_mods(swizzle_through(a[0], s.swizzle), lane, Mods { neg, ..mods })
            }
            ir::Op::Alu(ir::AluOp::Fabs, ref a) => {
                if mods.flr {
                    return Err(SelectError::Unsupported(
                        "a floor of an absolute value: no source modifier set says it",
                    ));
                }
                self.src_mods(swizzle_through(a[0], s.swizzle), lane, Mods { abs: true, ..mods })
            }
            ir::Op::Alu(ir::AluOp::Ffloor, ref a) => {
                self.src_mods(swizzle_through(a[0], s.swizzle), lane, Mods { flr: true, ..mods })
            }
            ir::Op::LoadLocal { index: None, .. } => {
                let v = self.loaded.get((s.val.0 * LANES + pick) as usize).copied().unwrap_or(UNLOADED);
                if v == UNLOADED {
                    return Err(SelectError::Unsupported("a local's value used before the load that reads it"));
                }
                Ok(SelSrc::Virtual { v, mods })
            }
            _ => Ok(SelSrc::Virtual { v: Self::reg(s.val, pick), mods }),
        }
    }

    fn input_src(&self, slot: ir::IoSlot, lane: u32, mods: Mods) -> Result<SelSrc, SelectError> {
        match slot {
            ir::IoSlot::Attrib(loc) => {
                Ok(SelSrc::Fixed { bank: Bank::VtxIn, reg: super::abi::vtxin_reg(self.attribs_read, loc, lane), mods })
            }
            /* The builder's iterator lands it in a register before the
             * program runs, so a read is an ordinary register read. */
            ir::IoSlot::Varying(loc) => {
                let idx = self
                    .varying_index(loc)
                    .ok_or(SelectError::Unsupported("a varying read that the module does not declare"))?;
                Ok(SelSrc::Virtual { v: self.varying_reg(idx, lane), mods })
            }
            ir::IoSlot::FragCoord => match lane {
                0 | 1 => match self.frag_coord[lane as usize] {
                    Some(v) => Ok(SelSrc::Virtual { v, mods }),
                    None => Err(SelectError::Unsupported("gl_FragCoord read before its prologue")),
                },
                _ => match self.frag_zw.get(lane as usize - 2).copied().flatten() {
                    Some(v) => Ok(SelSrc::Virtual { v, mods }),
                    None => Err(SelectError::Unsupported("gl_FragCoord.z or .w read outside a fragment shader")),
                },
            },
            ir::IoSlot::PointCoord => match self.point_coord.get(lane as usize).copied().flatten() {
                Some(v) => Ok(SelSrc::Virtual { v, mods }),
                None => Err(SelectError::Unsupported("gl_PointCoord read outside its two lanes")),
            },
            /* gl_VertexID / gl_InstanceID: the vtxin register the vertex
             * fetch writes it to (`abi::vs_id_regs`, the record's
             * vs_vertex_id_reg1 / vs_instance_id_reg1). */
            ir::IoSlot::VertexId | ir::IoSlot::InstanceId if self.m.stage != SHC_STAGE_FRAGMENT => {
                let (vid, iid) = super::abi::vs_id_regs(self.m);
                let reg = if slot == ir::IoSlot::VertexId { vid } else { iid };
                let reg = reg.ok_or(SelectError::Unsupported("a vertex built-in read the module does not load"))?;
                Ok(SelSrc::Fixed { bank: Bank::VtxIn, reg, mods })
            }
            ir::IoSlot::FrontFacing if self.m.stage == SHC_STAGE_FRAGMENT => match self.front_facing {
                Some(v) if lane == 0 => Ok(SelSrc::Virtual { v, mods }),
                _ => Err(SelectError::Unsupported("gl_FrontFacing read before its prologue")),
            },
            _ => Err(SelectError::Unsupported("a built-in fragment input: not agreed at the seam yet")),
        }
    }

    fn emit(&mut self, g: SelGroup) -> Result<(), SelectError> {
        self.groups.push(self.arena, g)?;
        Ok(())
    }

    /// One IR operation, expanded across its components.
    fn instr(&mut self, id: ir::InstrId, op: &ir::Op) -> Result<(), SelectError> {
        match *op {
            /* Folded into their consumers by `src`, so they produce nothing
             * here. Reaching one of these as an instruction is not an error;
             * it means nothing needed it. */
            ir::Op::ConstF32(_)
            | ir::Op::ConstI32(_)
            | ir::Op::ConstBool(_)
            | ir::Op::LoadUniform { .. }
            | ir::Op::LoadInput { .. } => Ok(()),

            /* The registers the slot is in now, for every use of this
             * value (`Sel::loaded`). */
            ir::Op::LoadLocal { local, index: None, .. } => {
                for lane in 0..LANES {
                    let r = self.local_reg(local, lane)?;
                    if let Some(x) = self.loaded.get_mut((id.0 * LANES + lane) as usize) {
                        *x = r;
                    }
                }
                Ok(())
            }

            ir::Op::Alu(o, ref args) => self.alu(id, o, args),

            /* A store points the slot at the value's register: registers
             * are written once, so no load or output that read the slot
             * before can see this store. A value with a modifier, a
             * constant or a fixed register is copied into a fresh register
             * first (`SelOp::Copy`, `mbyp`: the bits unchanged). */
            ir::Op::StoreLocal { local, index: None, src, write_mask } => {
                for lane in 0..LANES {
                    if write_mask & (1 << lane) == 0 {
                        continue;
                    }
                    let from = self.src(src, lane)?;
                    self.store(local, lane, from)?;
                }
                Ok(())
            }

            ir::Op::StoreOutput { slot, src } => {
                let mut comps: [Option<SelSrc>; 4] = [None; 4];
                let n = self.output_components(slot);
                for (lane, c) in comps.iter_mut().enumerate().take(n) {
                    *c = Some(self.src(src, lane as u32)?);
                }
                /* A slot stored again keeps the LAST value, per component:
                 * one entry per slot, so the epilogue writes each output
                 * dword once (visualization.vert assigns gl_Position twice,
                 * and was written twice to the same dwords). */
                if let Some((_, old)) = self.outputs.as_mut_slice().iter_mut().find(|(s, _)| *s == slot) {
                    for (o, c) in old.iter_mut().zip(comps) {
                        if c.is_some() {
                            *o = c;
                        }
                    }
                    return Ok(());
                }
                self.outputs.push(self.arena, (slot, comps))?;
                Ok(())
            }

            /* What selection would hand the builder for one `smp`, written
             * down at the lead's request so that pvrgpu-platform answers a
             * concrete question rather than designing in the dark. Five
             * things, and every one of them is known here and nowhere else:
             *
             * 1. **The coordinate, per component, as ordinary sources** --
             *    two for `Tex2D`, three for a cube map, and for the
             *    projective forms the divide has already happened in the
             *    front end (`ir.md`: `textureProj` is expanded), so the
             *    builder never sees a `w`.
             * 2. **The sampler binding**, as the GLSL-declared index. NOT a
             *    descriptor address: where the descriptor sits in the shared
             *    file is the base question under `LoadUniform` above, and it
             *    is decided below this seam.
             * 3. **The destination, per component.** ES 2.0's `texture2D`
             *    always yields four, but the consumer may read one -- the
             *    corpus is full of `.a` for a glyph -- and on a scalar
             *    machine a component nothing reads is a register and a cycle.
             *    So: four destination registers, each `Option`, `None` where
             *    nothing reads it.
             * 4. **The LOD**, which in ES 2.0 is exactly three cases and no
             *    more: implicit (fragment stage, the hardware computes the
             *    derivative), an explicit bias (`texture2D(s, c, bias)`,
             *    fragment only), and an explicit level (`texture2DLod`,
             *    vertex only -- and a vertex stage has no derivative, so an
             *    implicit fetch there is an error the front end already
             *    rejects). A fourth case would be ES 3.00 and is refused
             *    earlier.
             * 5. **Nothing about the texture's format, layout or state.**
             *    Selection knows none of it and must not appear to: the
             *    swizzle a format needs, the descriptor's dwords, whether
             *    the fetch is a `smp` or a gather, are the builder's.
             *
             * What I would NOT hand over, and why it matters that the answer
             * comes back rather than being assumed: whether `smp` is one
             * group or several, which phase it issues in, and whether its
             * result lands in a register or an internal one that a following
             * group must move. Those are the same class of fact as the
             * phases, and this file has none of them by design. */
            /* `texture2D(s, uv)` in a fragment shader: the marker the chain
             * expands (`SelOp::Sample`). The four results are this value's
             * four registers. The other forms -- bias, projective with a
             * level, cube, array, 3D -- and any fetch in a vertex shader take
             * other `smp` forms and are refused until each is transcribed. */
            /* `texture2DProj(s, p)` as pco emits it: the hardware divides,
             * given (x, y, q) in the data block and `smp`'s `proj`. IR v1
             * puts the divisor in lane 3 whatever the GLSL width (it was
             * lane 2 until then). */
            op @ ir::Op::Tex { sampler, coord, .. }
                if matches!(op.es100_tex(), Some((ir::Es100Tex::Tex2D | ir::Es100Tex::Tex2DProj, None))) =>
            {
                let mut proj = matches!(op.es100_tex(), Some((ir::Es100Tex::Tex2DProj, _)));
                let mut u = self.src(coord, 0)?;
                let mut v = self.src(coord, 1)?;
                let mut q = if proj { Some(self.src(coord, 3)?) } else { None };
                /* pvr_proj_divide: the divide in the shader, the quotient
                 * sampled as texture2D is (the TPU's level then comes from
                 * the quotient's derivatives; shaderc::pvr_proj_divide) */
                if let (true, Some(qv)) = (proj && crate::shaderc::proj_divide_enabled(), q) {
                    use ir::AluOp::{Fmul, Frcp};
                    let r = self.fresh_typed(SelOp::Alu(Frcp), op_type(Frcp), [Some(qv), None, None])?;
                    u = self.fresh_typed(SelOp::Alu(Fmul), op_type(Fmul), [Some(u), Some(r), None])?;
                    v = self.fresh_typed(SelOp::Alu(Fmul), op_type(Fmul), [Some(v), Some(r), None])?;
                    (proj, q) = (false, None);
                }
                let lod = if self.gradients_for(&op).is_some() {
                    self.plane_gradients(coord, 2)?;
                    SmpLod::Gradients
                } else {
                    SmpLod::Implicit
                };
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj, lod, fetch: false, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(u), Some(v), q],
                })
            }
            /* `textureLod(s, uv, lod)` on a 2D sampler in a fragment shader:
             * pco's `smp` with the level replacing the implicit one, the
             * level after the coordinates in the data block
             * (pco_nir_tex.c pco_emit_nir_smp: coordinates, divisor, level,
             * lookup); `texture(s, uv, bias)` the same with the bias in that
             * word and `lodm` bias. With a constant offset (`textureOffset`,
             * `textureLodOffset`) the block ends with the lookup word: u in
             * bits 0..5, v in 6..11, each truncated to its width as pco's
             * bitfield_insert does (rogue_hw_defs.h ROGUE_SMP_LOOKUP_*). */
            ir::Op::Tex {
                dim: ir::TexDim::D2,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset,
            } => {
                let u = self.src(coord, 0)?;
                let v = self.src(coord, 1)?;
                let (mut mode, level) = self.smp_level(lod)?;
                if self.gradients_for(op).is_some() {
                    self.plane_gradients(coord, 2)?;
                    mode = SmpLod::Gradients;
                }
                let offset = offset.map(|[du, dv, _]| ((du as u16) & 0x3f) | (((dv as u16) & 0x3f) << 6));
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: mode, fetch: false, offset, cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(u), Some(v), level],
                })
            }
            /* `texelFetch(s, p, lod)` on a 2D sampler in a fragment shader:
             * `smp` with the coordinate in texels (`nncoords`) and the level
             * replaced, as pco samples it -- but at the texel's CENTRE,
             * p + 0.5, where pco passes p itself. At the corner a LINEAR
             * sampler (the binding's own: a combined image-sampler gets no
             * point sampler, pco_nir_vk.c) blends the four texels around it,
             * which gl_test case 48 read on the table path; at the centre
             * every filter returns the texel exactly. */
            ir::Op::Tex {
                dim: ir::TexDim::D2,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(l),
                offset: None,
            } => {
                use ir::AluOp::{Fadd, I2f};
                let g =
                    |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
                let half = Some(SelSrc::Imm(0.5f32.to_bits()));
                let mut uv = [SelSrc::Imm(0); 2];
                for (k, c) in uv.iter_mut().enumerate() {
                    let p = self.src(coord, k as u32)?;
                    let pf = g(self, I2f, [Some(p), None, None])?;
                    *c = g(self, Fadd, [Some(pf), half, None])?;
                }
                let level = self.src(l, 0)?;
                let lf = g(self, I2f, [Some(level), None, None])?;
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: SmpLod::Replace, fetch: true, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(uv[0]), Some(uv[1]), Some(lf)],
                })
            }
            /* `texelFetch` on a 2D array: glsl2usc's pvr_arrFetch, texel
             * (x, y + z * (h / layers)) of the stacked image at level `lod`
             * (h the image's height there), fetched at the texel's centre as
             * a 2D texelFetch is. */
            ir::Op::Tex {
                dim: ir::TexDim::D2Array,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(l),
                offset: None,
            } => {
                use ir::AluOp::{F2i, Fadd, I2f, Iadd, Imul};
                let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
                let level = self.src(l, 0)?;
                let [_, height] = self.tex_size_2d(sampler, level)?;
                let layers = SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
                let count = g(self, F2i, [Some(layers), None, None])?;
                let h = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.idiv_into(height, count, false, SelDst::Virtual(h))?;
                let h = SelSrc::Virtual { v: h, mods: Mods::default() };
                let x = self.src(coord, 0)?;
                let y = self.src(coord, 1)?;
                let z = self.src(coord, 2)?;
                let zh = g(self, Imul, [Some(z), Some(h), None])?;
                let row = g(self, Iadd, [Some(y), Some(zh), None])?;
                let half = Some(SelSrc::Imm(0.5f32.to_bits()));
                let mut uv = [SelSrc::Imm(0); 2];
                for (c, p) in uv.iter_mut().zip([x, row]) {
                    let pf = g(self, I2f, [Some(p), None, None])?;
                    *c = g(self, Fadd, [Some(pf), half, None])?;
                }
                let lf = g(self, I2f, [Some(level), None, None])?;
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: SmpLod::Replace, fetch: true, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(uv[0]), Some(uv[1]), Some(lf)],
                })
            }
            /* texelFetch on a 3D texture: glsl2usc's pvr_volFetch, texel
             * c.xy + ivec2(c.z % cols, c.z / cols) * slice of the stacked
             * image, at level 0 whatever `lod` (as the helper does), fetched at
             * the texel's centre as a 2D texelFetch is. */
            ir::Op::Tex {
                dim: ir::TexDim::D3,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Fetch(_),
                offset: None,
            } => {
                use ir::AluOp::{Fadd, I2f, Iadd, Imul};
                let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
                let layers = SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
                let (cols, slice, _) = self.vol_grid(sampler, layers)?;
                let z = self.src(coord, 2)?;
                let mut cell = [SelSrc::Imm(0); 2];
                for (k, rem) in [(0usize, true), (1, false)] {
                    let d = VReg(self.next_vreg);
                    self.next_vreg += 1;
                    self.idiv_into(z, cols, rem, SelDst::Virtual(d))?;
                    cell[k] = SelSrc::Virtual { v: d, mods: Mods::default() };
                }
                let half = Some(SelSrc::Imm(0.5f32.to_bits()));
                let mut uv = [SelSrc::Imm(0); 2];
                for k in 0..2 {
                    let p = self.src(coord, k as u32)?;
                    let off = g(self, Imul, [Some(cell[k]), Some(slice[k]), None])?;
                    let t = g(self, Iadd, [Some(p), Some(off), None])?;
                    let tf = g(self, I2f, [Some(t), None, None])?;
                    uv[k] = g(self, Fadd, [Some(tf), half, None])?;
                }
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: SmpLod::Replace, fetch: true, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(uv[0]), Some(uv[1]), Some(SelSrc::Imm(0.0f32.to_bits()))],
                })
            }
            /* A 3D texture, through the grid of slices it is kept in:
             * glsl2usc's pvr_vol -- the two slices around c.z * layers - 0.5
             * sampled (`vol_uv`) and mixed -- and pvr_volLod, which samples
             * level 0 whatever level it is given, as the helper does. */
            ir::Op::Tex {
                dim: ir::TexDim::D3,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_)),
                offset: None,
            } => {
                use ir::AluOp::{Fadd, Ffma, Fmax, Fmin, Fmul};
                let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
                let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
                let layers = SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
                let uv = [self.src(coord, 0)?, self.src(coord, 1)?];
                let cz = self.src(coord, 2)?;
                let zl = g(self, Fmul, [Some(cz), Some(layers), None])?;
                let zm = g(self, Fadd, [Some(zl), f(-0.5), None])?;
                let last = g(self, Fadd, [Some(layers), f(-1.0), None])?;
                let z0m = g(self, Fmax, [Some(zm), f(0.0), None])?;
                let z = g(self, Fmin, [Some(z0m), Some(last), None])?;
                let z0 = self.modified(z, Mods { neg: false, abs: false, flr: true })?;
                let z0p = g(self, Fadd, [Some(z0), f(1.0), None])?;
                let z1 = g(self, Fmin, [Some(z0p), Some(last), None])?;
                let nz0 = self.modified(z0, Mods { neg: true, abs: false, flr: false })?;
                let t = g(self, Fadd, [Some(z), Some(nz0), None])?;
                let level = match lod {
                    ir::TexLod::Lod(_) => Some(SelSrc::Imm(0.0f32.to_bits())),
                    _ => None,
                };
                let mut taps = [VReg(0); 2];
                for (tap, k) in taps.iter_mut().zip([z0, z1]) {
                    let [u, v] = self.vol_uv(sampler, layers, uv, k)?;
                    *tap = VReg(self.next_vreg);
                    self.next_vreg += LANES;
                    self.emit(SelGroup {
                        op: SelOp::Sample {
                            binding: sampler,
                            proj: false,
                            lod: if level.is_some() { SmpLod::Replace } else { SmpLod::Implicit },
                            fetch: false,
                            offset: None,
                            cube: false,
                        },
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(*tap)),
                        srcs: [Some(u), Some(v), level],
                    })?;
                }
                /* mix(a, b, t) = ffma(t, b - a, a), as the front end lowers it */
                for lane in 0..LANES {
                    let a = SelSrc::Virtual { v: VReg(taps[0].0 + lane), mods: Mods::default() };
                    let b = SelSrc::Virtual { v: VReg(taps[1].0 + lane), mods: Mods::default() };
                    let na = self.modified(a, Mods { neg: true, abs: false, flr: false })?;
                    let d = g(self, Fadd, [Some(b), Some(na), None])?;
                    self.emit(SelGroup {
                        op: SelOp::Alu(Ffma),
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                        srcs: [Some(t), Some(d), Some(a)],
                    })?;
                }
                Ok(())
            }
            /* A 2D array, sampled through the stacked 2D image it is kept in
             * (`array_uv`), as glsl2usc's pvr_arr / pvr_arrLod sample it; a
             * bias applies to the stacked image's implicit level (glsl2usc
             * has no form for it). */
            ir::Op::Tex {
                dim: ir::TexDim::D2Array,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset: None,
            } => {
                let (u, v) = self.array_uv(sampler, coord)?;
                let (mode, level) = self.smp_level(lod)?;
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: mode, fetch: false, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(u), Some(v), level],
                })
            }
            /* A cube, sampled through the stacked 2D image it is kept in
             * (`cube_uv`), with the implicit level or a replacing one, as
             * glsl2usc's pvr_cube / pvr_cubeLod sample it, or a bias on the
             * stacked image's implicit level (glsl2usc has no form for it). */
            ir::Op::Tex {
                dim: ir::TexDim::Cube,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset: None,
            } => {
                let (mode, level) = self.smp_level(lod)?;
                /* The TPU's own cube (`gl::native_cube`, switch
                 * `pvr_native_cube` at compile time): the direction as it
                 * is, three coordinates, as pco's program for a samplerCube
                 * reads it (`smp` 3d); the record says so (`native_cube`),
                 * and the draw binds a twiddled TEXTYPE_CUBE copy. */
                if crate::shaderc::native_cube_enabled() {
                    let (x, y, z) = (self.src(coord, 0)?, self.src(coord, 1)?, self.src(coord, 2)?);
                    let mut mode = mode;
                    if self.gradients_for(op).is_some() {
                        self.plane_gradients(coord, 3)?;
                        mode = SmpLod::Gradients;
                    }
                    self.emit(SelGroup { op: SelOp::SampleZ, ty: OpType::F32, dst: None, srcs: [Some(z), None, None] })?;
                    return self.emit(SelGroup {
                        op: SelOp::Sample { binding: sampler, proj: false, lod: mode, fetch: false, offset: None, cube: true },
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(self.tex_out(id))),
                        srcs: [Some(x), Some(y), level],
                    });
                }
                /* an explicit level clamps to that level's face (`cube_uv`) */
                let explicit = if matches!(lod, ir::TexLod::Lod(_)) { level } else { None };
                let (u, v) = self.cube_uv(sampler, coord, explicit)?;
                self.emit(SelGroup {
                    op: SelOp::Sample { binding: sampler, proj: false, lod: mode, fetch: false, offset: None , cube: false },
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(self.tex_out(id))),
                    srcs: [Some(u), Some(v), level],
                })
            }
            /* A shadow look-up, as Mesa's pco builds one (pco_nir_tex.c
             * lower_tex_shadow, MIT): the depth sampled as the plain look-up
             * of the same sampler samples it -- the arm above, into a scratch
             * block -- then compared in the shader against the reference.
             * The reference follows the coordinate: lane 2 for a 2D sampler,
             * lane 3 for a cube or a 2D array (ir::Op::Tex). */
            ir::Op::Tex {
                dim: dim @ (ir::TexDim::D2 | ir::TexDim::Cube | ir::TexDim::D2Array),
                shadow: true,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: lod @ (ir::TexLod::Implicit | ir::TexLod::Lod(_) | ir::TexLod::Bias(_)),
                offset: None,
            } if self.tex_dst.is_none() => {
                let depth = VReg(self.next_vreg);
                self.next_vreg += LANES;
                let plain = ir::Op::Tex {
                    dim,
                    shadow: false,
                    proj: false,
                    ret: ir::ScalarType::F32,
                    sampler,
                    coord,
                    lod,
                    offset: None,
                };
                self.tex_dst = Some(depth);
                let sampled = self.instr(id, &plain);
                self.tex_dst = None;
                sampled?;
                let reference = self.src(coord, if dim == ir::TexDim::D2 { 2 } else { 3 })?;
                self.shadow_compare(id, sampler, SelSrc::Virtual { v: depth, mods: Mods::default() }, reference)
            }
            ir::Op::Tex { .. } => Err(SelectError::Unsupported(
                "a texture fetch with a projective level or bias, a bias on a 3D texture, a shadow compare, an integer result or gradients, or an offset on a stacked texture, is not selected yet",
            )),
            /* `textureSize(s, lod)` on a 2D sampler in a fragment shader, as
             * pco computes it (usclib/tex.cl usclib_tex_state_size, MIT, for
             * two components): width and height less one from the image
             * state's first pair (bits 34..47, 48..61), the base level (the
             * second pair's bits 60..63) added to `lod` unless the image is
             * a STRIDE one (textype 4, bits 0..2) -- YakoGL's copy of it,
             * which the table's programs were compiled with -- each size
             * shifted right by that level and at least 1. */
            ir::Op::TexSize { sampler, dim: ir::TexDim::D2, lod } => {
                let lod = self.src(lod, 0)?;
                let size = self.tex_size_2d(sampler, lod)?;
                for (lane, r) in size.into_iter().enumerate() {
                    self.emit(SelGroup {
                        op: SelOp::Copy,
                        ty: OpType::U32,
                        dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane as u32))),
                        srcs: [Some(r), None, None],
                    })?;
                }
                Ok(())
            }
            /* textureSize of a stacked texture, as glsl2usc's pvr_arrSize and
             * pvr_cubeSize read the stacked 2D image: a 2D array is (w,
             * h / layers, layers), a cube (w, h / 6), integer divisions
             * (`idiv_into`, nir_lower_idiv's). */
            ir::Op::TexSize { sampler, dim: dim @ (ir::TexDim::D2Array | ir::TexDim::Cube), lod } => {
                let lod = self.src(lod, 0)?;
                let [w, h] = self.tex_size_2d(sampler, lod)?;
                let count = if dim == ir::TexDim::Cube {
                    SelSrc::Imm(6)
                } else {
                    let layers =
                        SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
                    self.fresh_typed(SelOp::Alu(ir::AluOp::F2i), op_type(ir::AluOp::F2i), [Some(layers), None, None])?
                };
                let per = SelDst::Virtual(Self::reg(ir::ValueId(id.0), 1));
                self.idiv_into(h, count, false, per)?;
                /* lane 2, the layer count, for an array only (no_std: no Vec) */
                let lanes = [Some((0u32, w)), (dim == ir::TexDim::D2Array).then_some((2, count))];
                for (lane, r) in lanes.into_iter().flatten() {
                    self.emit(SelGroup {
                        op: SelOp::Copy,
                        ty: OpType::U32,
                        dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                        srcs: [Some(r), None, None],
                    })?;
                }
                Ok(())
            }
            /* textureSize of a 3D texture: glsl2usc's pvr_volSize, one
             * slice's size and the slice count (level 0 whatever `lod`, as
             * the helper does). */
            ir::Op::TexSize { sampler, dim: ir::TexDim::D3, .. } => {
                let layers = SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
                let (_, [sw, sh], n) = self.vol_grid(sampler, layers)?;
                for (lane, r) in [sw, sh, n].into_iter().enumerate() {
                    self.emit(SelGroup {
                        op: SelOp::Copy,
                        ty: OpType::U32,
                        dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane as u32))),
                        srcs: [Some(r), None, None],
                    })?;
                }
                Ok(())
            }
            /* `true` into all four lanes of the flag slot: a whole store, so
             * an arm that discards needs no seed copy of the flag. */
            ir::Op::Discard => {
                let local =
                    self.discard_local().ok_or(SelectError::Unsupported("`discard` outside a fragment shader"))?;
                for lane in 0..LANES {
                    self.store(local, lane, SelSrc::Imm(u32::MAX))?;
                }
                Ok(())
            }
            /* `block[base + clamp(index, 0, len - 1) * stride + lane]`, as
             * pco reads it: the dword set into index register 0, then one
             * `mbyp` per lane from `shared[idx0 + lane]`. The clamp is the
             * IR's rule (an index outside the array reads the array's end,
             * never another uniform or a descriptor). The block moves past
             * the sampler descriptors after selection, and the address
             * computed here moves with it: `chain::expand_samples` adds the
             * descriptors' size to every idx0 write. */
            ir::Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
                if len == 0 {
                    return Err(SelectError::Unsupported("a dynamically indexed uniform array of no elements"));
                }
                use ir::AluOp::{Bcsel, Iadd, Ilt, Imul};
                let imm = |x: u32| Some(SelSrc::Imm(x));
                let i = self.src(index, 0)?;
                let below = self.fresh_group(SelOp::Alu(Ilt), [Some(i), imm(0), None])?;
                let i = self.fresh_group(SelOp::Alu(Bcsel), [Some(below), imm(0), Some(i)])?;
                let last = u32::from(len - 1);
                let above = self.fresh_group(SelOp::Alu(Ilt), [imm(last), Some(i), None])?;
                let i = self.fresh_group(SelOp::Alu(Bcsel), [Some(above), imm(last), Some(i)])?;
                let i = if stride_dw == 1 {
                    i
                } else {
                    self.fresh_group(SelOp::Alu(Imul), [Some(i), imm(u32::from(stride_dw)), None])?
                };
                let at = if base_dw == 0 {
                    i
                } else {
                    self.fresh_group(SelOp::Alu(Iadd), [Some(i), imm(u32::from(base_dw)), None])?
                };
                self.emit(SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::U32,
                    dst: Some(SelDst::Fixed { bank: Bank::Idx0, reg: super::sel::IDX_SELF }),
                    srcs: [Some(at), None, None],
                })?;
                for lane in 0..u32::from(comps).min(LANES) {
                    self.emit(SelGroup {
                        op: SelOp::Copy,
                        ty: OpType::F32,
                        dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                        srcs: [
                            Some(SelSrc::Fixed {
                                bank: Bank::Idx0,
                                reg: super::sel::idx_shared(lane as u16),
                                mods: Mods::default(),
                            }),
                            None,
                            None,
                        ],
                    })?;
                }
                Ok(())
            }
            ir::Op::LoadLocal { .. } | ir::Op::StoreLocal { .. } => {
                Err(SelectError::Unsupported("an indexed local slot: arrays need a dynamic register index"))
            }
        }
    }

    /// How many components an output takes. Position is four; everything else
    /// is asked of the declaration, and a varying the module does not declare
    /// is one the front end should not have stored to.
    fn output_components(&self, slot: ir::IoSlot) -> usize {
        match slot {
            ir::IoSlot::Position | ir::IoSlot::FragColor(_) => 4,
            ir::IoSlot::PointSize | ir::IoSlot::FragDepth => 1,
            ir::IoSlot::Varying(loc) => self
                .m
                .outputs
                .iter()
                .find(|d| d.kind == ir::IoKind::Varying && d.location == loc)
                .map_or(4, |d| d.components as usize),
            _ => 4,
        }
    }

    fn alu(&mut self, id: ir::InstrId, o: ir::AluOp, args: &[ir::Src; 3]) -> Result<(), SelectError> {
        use ir::AluOp::*;
        /* Folded by `src`; see the arms there. */
        if matches!(o, Fneg | Fabs | Ffloor) {
            return Ok(());
        }
        if matches!(o, Fdot2 | Fdot3 | Fdot4) {
            return self.dot(id, terms(o), args);
        }
        if matches!(o, Bany | Ball) {
            return self.reduce(id, if o == Bany { Ior } else { Iand }, args[0]);
        }
        if matches!(o, Fsin | Fcos) {
            return self.trig(id, if o == Fsin { Trig::Sin } else { Trig::Cos }, args[0]);
        }
        /* `fract(x)`, as pco's program for it (NIR `lower_ffract`):
         * `f = floor(x) + 0.0` -- the floor a source modifier -- then
         * `-f + x`. */
        if o == Ffract {
            for lane in 0..self.result_lanes(id) {
                let x = self.src(args[0], lane)?;
                /* An operand carrying abs or negate is computed first: the
                 * floor would otherwise apply before them (see `src_mods`). */
                let x = match x {
                    SelSrc::Virtual { mods, .. } | SelSrc::Fixed { mods, .. } if mods.neg || mods.abs => {
                        let t = VReg(self.next_vreg);
                        self.next_vreg += 1;
                        self.emit(SelGroup {
                            op: SelOp::Copy,
                            ty: OpType::F32,
                            dst: Some(SelDst::Virtual(t)),
                            srcs: [Some(x), None, None],
                        })?;
                        SelSrc::Virtual { v: t, mods: Mods::default() }
                    }
                    other => other,
                };
                let floored = match x {
                    SelSrc::Virtual { v, mods } => SelSrc::Virtual { v, mods: Mods { flr: true, ..mods } },
                    SelSrc::Fixed { bank, reg, mods } => SelSrc::Fixed { bank, reg, mods: Mods { flr: true, ..mods } },
                    SelSrc::Imm(b) => SelSrc::Imm(fold_mods(b, Mods { flr: true, ..Mods::default() })),
                };
                let f = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.emit(SelGroup {
                    op: SelOp::Alu(Fadd),
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(f)),
                    srcs: [Some(floored), Some(SelSrc::Imm(0)), None],
                })?;
                self.emit(SelGroup {
                    op: SelOp::Alu(Fadd),
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                    srcs: [Some(SelSrc::Virtual { v: f, mods: Mods { neg: true, ..Mods::default() } }), Some(x), None],
                })?;
            }
            return Ok(());
        }
        if matches!(o, Idiv | Imod) {
            return self.idiv(id, o == Imod, args);
        }
        if matches!(o, Udiv | Umod) {
            for lane in 0..self.result_lanes(id) {
                let (x, y) = (self.src(args[0], lane)?, self.src(args[1], lane)?);
                self.udiv_into(x, y, o == Umod, SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane)))?;
            }
            return Ok(());
        }
        if !SelGroup::is_buildable(o) {
            return Err(SelectError::Unsupported("an operation the machine half refuses; see `builtins::shape`"));
        }
        let n = SelGroup::arity(o);
        let ty = op_type(o);
        /* The result's width is the widest operand's, which for a
         * component-wise operation is what the IR guarantees. */
        let lanes = self.result_lanes(id);
        for lane in 0..lanes {
            let mut srcs: [Option<SelSrc>; 3] = [None; 3];
            for (k, s) in srcs.iter_mut().enumerate().take(n) {
                *s = Some(self.src(args[k], lane)?);
            }
            let g = SelGroup {
                op: SelOp::Alu(o),
                ty,
                dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                srcs,
            };
            self.emit(g)?;
        }
        Ok(())
    }

    /// `Bany`/`Ball`: an `or`/`and` chain over all four lanes, into the
    /// result's lane 0.
    ///
    /// All four, by the IR's contract (`lower.rs` `compare_vector`): the
    /// front end repeats a narrow vector's last real lane into the spare
    /// ones, which `or` and `and` absorb, and the reduction cannot know the
    /// width. A per-lane group would compute nothing like a reduction.
    fn reduce(&mut self, id: ir::InstrId, op: ir::AluOp, arg: ir::Src) -> Result<(), SelectError> {
        let dst = Self::reg(ir::ValueId(id.0), 0);
        let first = self.src(arg, 0)?;
        for k in 1..LANES {
            let acc = if k == 1 { first } else { SelSrc::Virtual { v: dst, mods: Mods::default() } };
            let next = self.src(arg, k)?;
            self.emit(SelGroup {
                op: SelOp::Alu(op),
                ty: OpType::U32,
                dst: Some(SelDst::Virtual(dst)),
                srcs: [Some(acc), Some(next), None],
            })?;
        }
        Ok(())
    }

    /// A dot product, expanded here rather than below the seam.
    ///
    /// `Fdot3` has six external operands and `SelGroup::srcs` has three, so
    /// the builder cannot be handed one: it would need the component
    /// numbering, which is selection's. `ir.md` predicted this when it kept
    /// the dot products in the IR kernel for the GC7000's sake -- "on the USC
    /// an `Fdot*` scalarises into the same `Fmul`/`Ffma` chain the front end
    /// would have written". Ruled by the lead 2026-09-22.
    ///
    /// ```text
    ///   fmul r0, a.x, b.x
    ///   fmad r0, a.y, b.y, r0
    ///   fmad r0, a.z, b.z, r0
    /// ```
    ///
    /// **One register, accumulating into the destination** -- and that is a
    /// correction to what this file did first. I wrote the partial sums into
    /// the result value's own spare lanes, to avoid assuming that an `fmad`
    /// may read the register it writes, and pinned the caution in a test:
    /// "no group reads its own destination, until someone measures that it
    /// may". pvrgpu-platform measured it (a4a6935): **266,932 groups in the
    /// corpus read a register they also write**, with `fmad` accumulating
    /// into its destination as the common case. So the caution was wrong, in
    /// the direction that costs a register per dot, and the test now asserts
    /// the corpus fact instead.
    fn dot(&mut self, id: ir::InstrId, n: u32, args: &[ir::Src; 3]) -> Result<(), SelectError> {
        let v = ir::ValueId(id.0);
        let dst = Self::reg(v, 0);
        for k in 0..n {
            let a = self.src(args[0], k)?;
            let b = self.src(args[1], k)?;
            let g = if k == 0 {
                SelGroup {
                    op: SelOp::Alu(ir::AluOp::Fmul),
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(dst)),
                    srcs: [Some(a), Some(b), None],
                }
            } else {
                /* Accumulate into the destination. */
                let acc = SelSrc::Virtual { v: dst, mods: Mods::default() };
                SelGroup {
                    op: SelOp::Alu(ir::AluOp::Ffma),
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(dst)),
                    srcs: [Some(a), Some(b), Some(acc)],
                }
            };
            self.emit(g)?;
        }
        Ok(())
    }

    /// Walk a run of nodes.
    fn nodes(&mut self, r: ir::Range) -> Result<(), SelectError> {
        for n in self.m.nodes_of(r) {
            match *n {
                ir::Node::Instr(id) => {
                    let op = *self.m.instr(id).ok_or(SelectError::Unsupported("a node naming no instruction"))?;
                    self.instr(id, &op)?;
                }
                /* The end of the program, which the builder marks. */
                ir::Node::Return => {}
                ir::Node::If { cond, then_, else_ } => self.if_convert(cond, then_, else_)?,
                ir::Node::Loop { body } if self.trips.iter().any(|t| t.first == body.first) => self.loop_(body)?,
                ir::Node::Loop { .. } | ir::Node::Break | ir::Node::Continue => {
                    return Err(SelectError::Unsupported(
                        "a loop or a jump: the USC half has no branch, and `SelGroup` carries no target",
                    ))
                }
            }
        }
        Ok(())
    }

    /// A loop the unroller kept (`unroll::Trip`): its body once, between a
    /// [`SelOp::LoopBegin`] and the back edge.
    ///
    /// ```text
    ///   h[l] = <local l>        every local lane the body stores, carried
    ///   c = 0                   the loop's counter
    ///   LoopBegin
    ///   <body>                  reads h[l], stores fresh registers
    ///   h[l] = <local l>        the value the next iteration starts from
    ///   c = c + 1
    ///   b = (trip <= c)
    ///   LoopTest b              p0 = (b == 0), that is c < trip
    ///   LoopEnd                 br if(p0) to LoopBegin
    /// ```
    ///
    /// Every exit is a flag the unroller made, so the body is straight-line
    /// code under selects, and the branch reads the counter alone: the same
    /// in every instance (`verify_loops`). A carried register `h` is the one
    /// kind of register written twice; every pass that assumes one write
    /// leaves it alone because it counts writes.
    fn loop_(&mut self, body: ir::Range) -> Result<(), SelectError> {
        let (count, exit, divergent) = self
            .trips
            .iter()
            .find(|t| t.first == body.first)
            .map(|t| (t.count, t.exit, t.divergent))
            .ok_or(SelectError::Unsupported("a loop the unroller did not keep"))?;
        if self.stores_output(body) {
            return Err(SelectError::Unsupported("an output written inside a loop"));
        }
        let n = self.locals_in.len();
        let carried = self.arena.alloc_filled(n, false)?;
        self.mark_carried(body, carried);
        let homes = self.arena.alloc_filled(n, VReg(u32::MAX))?;
        let plain = |v: VReg| Some(SelSrc::Virtual { v, mods: Mods::default() });
        for idx in 0..n {
            if !carried[idx] {
                continue;
            }
            let h = VReg(self.next_vreg);
            self.next_vreg += 1;
            self.emit(SelGroup {
                op: SelOp::Copy,
                ty: OpType::F32,
                dst: Some(SelDst::Virtual(h)),
                srcs: [plain(self.locals_in[idx]), None, None],
            })?;
            self.locals_in[idx] = h;
            homes[idx] = h;
        }
        let c = VReg(self.next_vreg);
        self.next_vreg += 1;
        self.emit(SelGroup {
            op: SelOp::Copy,
            ty: OpType::U32,
            dst: Some(SelDst::Virtual(c)),
            srcs: [Some(SelSrc::Imm(0)), None, None],
        })?;
        /* A loop whose instances may leave at different passes runs under
         * the execution mask (pco's construction, `powervr::cf`): the mask
         * set up once, a loop level entered before the body. */
        let emc = if divergent {
            let emc = match self.emc {
                Some(e) => e,
                None => {
                    let e = VReg(self.next_vreg);
                    self.next_vreg += 1;
                    self.emit(SelGroup {
                        op: SelOp::Machine(MachineOp::CndInit),
                        ty: OpType::U32,
                        dst: Some(SelDst::Virtual(e)),
                        srcs: [Some(SelSrc::Fixed { bank: Bank::Special, reg: 0, mods: Mods::default() }), None, None],
                    })?;
                    self.emc = Some(e);
                    e
                }
            };
            self.emit(SelGroup {
                op: SelOp::Machine(MachineOp::CndEnter),
                ty: OpType::U32,
                dst: Some(SelDst::Virtual(emc)),
                srcs: [plain(emc), None, None],
            })?;
            Some(emc)
        } else {
            None
        };
        self.emit(SelGroup { op: SelOp::LoopBegin, ty: OpType::U32, dst: None, srcs: [None; 3] })?;
        self.loop_depth += 1;
        self.nodes(body)?;
        self.loop_depth -= 1;
        for idx in 0..n {
            if !carried[idx] {
                continue;
            }
            if self.locals_in[idx] != homes[idx] {
                self.emit(SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(homes[idx])),
                    srcs: [plain(self.locals_in[idx]), None, None],
                })?;
            }
            self.locals_in[idx] = homes[idx];
        }
        self.emit(SelGroup {
            op: SelOp::Alu(ir::AluOp::Iadd),
            ty: op_type(ir::AluOp::Iadd),
            dst: Some(SelDst::Virtual(c)),
            srcs: [plain(c), Some(SelSrc::Imm(1)), None],
        })?;
        let b = VReg(self.next_vreg);
        self.next_vreg += 1;
        self.emit(SelGroup {
            op: SelOp::Alu(ir::AluOp::Ile),
            ty: op_type(ir::AluOp::Ile),
            dst: Some(SelDst::Virtual(b)),
            srcs: [Some(SelSrc::Imm(count)), plain(c), None],
        })?;
        /* A loop counted at run time (a uniform bound, capped at `count`,
         * unroll::UNIFORM_LOOP_CAP): it also leaves when its break flag --
         * every write of which the unroller traced to uniform values -- is
         * set. `leave = (count <= c) | flag`, the same in every instance. */
        let b = match exit {
            None => b,
            Some(flag) => {
                let f = *self
                    .locals_in
                    .get((flag.0 * LANES) as usize)
                    .ok_or(SelectError::Unsupported("a loop's exit flag that has no slot"))?;
                let e = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.emit(SelGroup {
                    op: SelOp::Alu(ir::AluOp::Ior),
                    ty: op_type(ir::AluOp::Ior),
                    dst: Some(SelDst::Virtual(e)),
                    srcs: [plain(b), plain(f), None],
                })?;
                e
            }
        };
        self.emit(SelGroup {
            op: SelOp::Machine(MachineOp::LoopTest),
            ty: OpType::U32,
            dst: None,
            srcs: [plain(b), None, None],
        })?;
        /* Under the mask: p0 is clear where this instance is done (its
         * flag, or the cap); those instances leave the loop's level, and p0
         * is then set again where the instance is still in -- the back edge
         * is taken while any instance is. */
        if let Some(emc) = emc {
            for op in [MachineOp::CndLeave, MachineOp::CndLoopEnd] {
                self.emit(SelGroup {
                    op: SelOp::Machine(op),
                    ty: OpType::U32,
                    dst: Some(SelDst::Virtual(emc)),
                    srcs: [plain(emc), None, None],
                })?;
            }
        }
        self.emit(SelGroup { op: SelOp::LoopEnd, ty: OpType::U32, dst: None, srcs: [None; 3] })?;
        Ok(())
    }

    /// Does `r` store an output anywhere?
    fn stores_output(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match *n {
            ir::Node::Instr(id) => matches!(self.m.instr(id), Some(ir::Op::StoreOutput { .. })),
            ir::Node::If { then_, else_, .. } => self.stores_output(then_) || self.stores_output(else_),
            ir::Node::Loop { body } => self.stores_output(body),
            _ => false,
        })
    }

    /// Mark every local lane `r` stores (the discard flag's four for a
    /// `discard`).
    fn mark_carried(&self, r: ir::Range, carried: &mut [bool]) {
        for n in self.m.nodes_of(r) {
            match *n {
                ir::Node::Instr(id) => match self.m.instr(id) {
                    Some(ir::Op::StoreLocal { local, index, write_mask, .. }) => {
                        for lane in 0..LANES {
                            if index.is_some() || write_mask & (1 << lane) != 0 {
                                if let Some(c) = carried.get_mut((local.0 * LANES + lane) as usize) {
                                    *c = true;
                                }
                            }
                        }
                    }
                    Some(ir::Op::Discard) => {
                        if let Some(local) = self.discard_local() {
                            for lane in 0..LANES {
                                if let Some(c) = carried.get_mut((local.0 * LANES + lane) as usize) {
                                    *c = true;
                                }
                            }
                        }
                    }
                    _ => {}
                },
                ir::Node::If { then_, else_, .. } => {
                    self.mark_carried(then_, carried);
                    self.mark_carried(else_, carried);
                }
                ir::Node::Loop { body } => self.mark_carried(body, carried),
                _ => {}
            }
        }
    }

    /// An `if` without a branch: the then arm writes its own registers and a
    /// `movc` picks at the join.
    ///
    /// The same design as the GC7000 side, arrived at there by measurement
    /// over the whole corpus, and there is no reason for the two machines to
    /// differ: `Bcsel` IS `movc` here (`builtins::shape` maps it), so the join
    /// costs one group per component per written slot and needs no seam change
    /// at all.
    ///
    /// ```text
    ///   <then arm>            its stores land in then[l]
    ///   <else arm>            its stores land in slot[l], which still holds
    ///                         the pre-`if` value because the then arm went
    ///                         elsewhere
    ///   movc slot[l], cond, then[l], slot[l]
    /// ```
    ///
    /// **A nested `if` too**, level by level: each `if` saves the local
    /// mapping in force before it and restores that (not the home registers)
    /// between its arms, so an inner join's result is what the outer join
    /// sees. The cost is a select per written component per level -- the
    /// GC7000 measured 60 temps against the reference's 10 on one shader this
    /// way and branches instead; here there is no branch yet (`SelGroup`
    /// carries no target), and the one corpus program that outgrows the temps
    /// file is refused by the allocator by name. A loop or a jump inside an
    /// arm still refuses. Outputs stored in an arm are joined the same way.
    ///
    /// **A seed copy refuses** rather than being invented. An arm that reads
    /// the slot it writes, or writes only some of its components, needs its
    /// register to start as a copy of the slot -- and a copy is the one thing
    /// this file cannot emit until the seam has `mbyp`. It falls into the same
    /// named refusal as every other copy, so it is counted with them rather
    /// than hidden inside control flow.
    fn if_convert(&mut self, cond: ir::Src, then_: ir::Range, else_: ir::Range) -> Result<(), SelectError> {
        /* A nested `if` is if-converted too: both arms of every level run and
         * each level joins its locals with a select, which composes because
         * each level restores the mapping in force before it (below) rather
         * than the locals' home registers. Anything else in an arm -- a
         * loop, a jump -- needs a real branch. */
        if self.arm_has_non_if_flow(then_) || self.arm_has_non_if_flow(else_) {
            return Err(SelectError::Unsupported(
                "a loop or a jump inside an `if`: it needs a real branch, and `SelGroup` carries no target",
            ));
        }
        /* Both arms run, so an output stored in an arm is joined like a
         * local: the outputs as they stood before, each arm's, and a select
         * per component that differs (below). Stored unconditionally it would
         * take the else arm's value whatever the condition. */
        /* The copies below are the join's working state: each is given
         * back to the arena at the end of this `if` (`AVec::release`), which
         * reuses them for the next. Kept, they cost two copies of the local
         * map per `if` for the rest of the compile -- 14 KB each on a Qt
         * shape with a large local set, 156 of them past 2 MiB. */
        let mut outputs_before: AVec<'a, (ir::IoSlot, [Option<SelSrc>; 4])> = AVec::new();
        outputs_before.extend_from_slice(self.arena, self.outputs.as_slice())?;
        /* The mapping in force before this `if`. Each arm starts from it
         * and moves slots to the registers it stores (registers are
         * written once); the join selects between the two mappings. */
        let mut saved: AVec<'a, VReg> = AVec::new();
        saved.extend_from_slice(self.arena, self.locals_in)?;
        /* pco predicates an `if` it can (pco_cf.c can_pred_exec): a
         * tstz.p0 of the condition, the then arm's groups run where p0 is
         * clear, the else arm's where it is set. The arms' values are fresh
         * registers joined by the selects below on the same condition, so
         * an arm that does not run leaves only values nothing reads, and the
         * hardware passes over its groups at about no cost where no
         * instance runs them (gl_test 186/187: the whole fill_color gap). */
        let pred = self.predicable(then_) && self.predicable(else_);
        if pred {
            let c = self.src(cond, 0)?;
            self.emit(SelGroup {
                op: SelOp::Machine(MachineOp::LoopTest),
                ty: OpType::U32,
                dst: None,
                srcs: [Some(c), None, None],
            })?;
            self.emit(SelGroup { op: SelOp::Pred(PredArm::Then), ty: OpType::U32, dst: None, srcs: [None; 3] })?;
        }
        self.nodes(then_)?;
        let mut then_map: AVec<'a, VReg> = AVec::new();
        then_map.extend_from_slice(self.arena, self.locals_in)?;
        self.locals_in.copy_from_slice(saved.as_slice());
        saved.release(self.arena);
        /* the then arm's outputs, taken rather than copied; the else arm
         * starts from the outputs as they stood before */
        let outputs_then = core::mem::replace(&mut self.outputs, outputs_before);

        if pred {
            self.emit(SelGroup { op: SelOp::Pred(PredArm::Else), ty: OpType::U32, dst: None, srcs: [None; 3] })?;
        }
        self.nodes(else_)?;
        if pred {
            self.emit(SelGroup { op: SelOp::Pred(PredArm::End), ty: OpType::U32, dst: None, srcs: [None; 3] })?;
        }

        /* The condition is a value from before the `if`, so the arms cannot
         * have changed what it reads. */
        let c = self.src(cond, 0)?;
        for (k, &t) in then_map.as_slice().iter().enumerate() {
            let e = self.locals_in[k];
            if t == e {
                continue;
            }
            let d = VReg(self.next_vreg);
            self.next_vreg += 1;
            self.emit(SelGroup {
                op: SelOp::Alu(ir::AluOp::Bcsel),
                ty: OpType::U32,
                dst: Some(SelDst::Virtual(d)),
                srcs: [
                    Some(c),
                    Some(SelSrc::Virtual { v: t, mods: Mods::default() }),
                    Some(SelSrc::Virtual { v: e, mods: Mods::default() }),
                ],
            })?;
            self.locals_in[k] = d;
        }

        /* The outputs: every component whose value differs between the two
         * arms (either may have left it as it was) becomes a select. A
         * component one side never wrote is undefined there in GLSL; zero
         * stands in, so nothing unwritten is read. */
        let lane_of = |list: &[(ir::IoSlot, [Option<SelSrc>; 4])], slot: ir::IoSlot, lane: usize| {
            list.iter().find(|(s, _)| *s == slot).and_then(|(_, c)| c[lane])
        };
        let mut slots: AVec<'a, ir::IoSlot> = AVec::new();
        for (sl, _) in outputs_then.as_slice().iter().chain(self.outputs.as_slice().iter()) {
            if !slots.as_slice().contains(sl) {
                slots.push(self.arena, *sl)?;
            }
        }
        for &sl in slots.as_slice() {
            for lane in 0..LANES as usize {
                let t = lane_of(outputs_then.as_slice(), sl, lane);
                let e = lane_of(self.outputs.as_slice(), sl, lane);
                if t == e {
                    continue;
                }
                let d = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.emit(SelGroup {
                    op: SelOp::Alu(ir::AluOp::Bcsel),
                    ty: OpType::U32,
                    dst: Some(SelDst::Virtual(d)),
                    srcs: [Some(c), Some(t.unwrap_or(SelSrc::Imm(0))), Some(e.unwrap_or(SelSrc::Imm(0)))],
                })?;
                let joined = Some(SelSrc::Virtual { v: d, mods: Mods::default() });
                match self.outputs.as_mut_slice().iter_mut().find(|(s, _)| *s == sl) {
                    Some((_, comps)) => comps[lane] = joined,
                    None => {
                        let mut comps = [None; 4];
                        comps[lane] = joined;
                        self.outputs.push(self.arena, (sl, comps))?;
                    }
                }
            }
        }
        slots.release(self.arena);
        then_map.release(self.arena);
        outputs_then.release(self.arena);
        Ok(())
    }

    /// A shader that discards: pco's fragment epilogue in full mode, which
    /// reads the per-draw control word at the end of the uniform block
    /// (`pco_nir_pfo`, `pco_nir_lower_alpha_to_coverage`, Mesa
    /// `src/imagination/pco/pco_nir_pvfio.c`, MIT, read and transcribed, in
    /// the order pco's program computes it):
    ///
    /// ```text
    ///   alpha to coverage: a' = meta.a2c ? a : 1.0
    ///                      kill |= (pck.cov(a') & savmsk & 1 << samp_num) == 0
    ///   the sample mask:   kill |= (meta.sample_mask & savmsk & 1 << samp_num) == 0
    ///   alpha to one:      a = meta.a2one ? 1.0 : a
    /// ```
    ///
    /// Each field is `ubfe` guarded against a zero width by a select, as
    /// pco's `ubfe` lowering leaves it. With the driver's default word
    /// (`SHC_FS_META_DEFAULT`: no alpha to coverage, no alpha to one, every
    /// sample) all of it leaves the shader's own discard as the answer.
    fn discard_epilogue(&mut self) -> Result<(), SelectError> {
        let Some(local) = self.discard_local() else { return Ok(()) };
        let plain = Mods::default();
        let flag = SelSrc::Virtual { v: self.local_reg(local, 0)?, mods: plain };
        let meta = SelSrc::Fixed { bank: Bank::Shared, reg: meta_dword(self.m), mods: plain };
        let alpha = self
            .outputs
            .as_slice()
            .iter()
            .find(|(s, _)| *s == ir::IoSlot::FragColor(0))
            .and_then(|(_, c)| c[3])
            .ok_or(SelectError::Unsupported("a shader that discards and writes no colour alpha"))?;
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let one = 1.0f32.to_bits();
        use ir::AluOp::{Bcsel, Iand, Ieq, Ine, Ior, Ishl};
        let op = Self::fresh_group;

        let e = self.meta_field(meta, META_ALPHA_TO_COVERAGE)?;
        let a2c = op(self, SelOp::Alu(Ine), [Some(e), imm(0), None])?;
        let a = op(self, SelOp::Alu(Bcsel), [Some(a2c), Some(alpha), imm(one)])?;
        let cov = op(self, SelOp::Machine(MachineOp::PckCov), [Some(a), None, None])?;
        let covered = op(self, SelOp::Machine(MachineOp::SavMsk), [None; 3])?;
        let samp = SelSrc::Fixed { bank: Bank::Special, reg: SR_SAMP_NUM, mods: plain };
        let bit = op(self, SelOp::Alu(Ishl), [imm(1), Some(samp), None])?;
        let t = op(self, SelOp::Alu(Iand), [Some(covered), Some(bit), None])?;
        let t = op(self, SelOp::Alu(Iand), [Some(cov), Some(t), None])?;
        let z = op(self, SelOp::Alu(Ieq), [Some(t), imm(0), None])?;
        let kill = op(self, SelOp::Alu(Ior), [Some(flag), Some(z), None])?;

        let mask = self.meta_field(meta, META_SAMPLE_MASK)?;
        let t = op(self, SelOp::Alu(Iand), [Some(mask), Some(covered), None])?;
        let t = op(self, SelOp::Alu(Iand), [Some(bit), Some(t), None])?;
        let z = op(self, SelOp::Alu(Ieq), [Some(t), imm(0), None])?;
        let kill = op(self, SelOp::Alu(Ior), [Some(kill), Some(z), None])?;

        let e = self.meta_field(meta, META_ALPHA_TO_ONE)?;
        let a2one = op(self, SelOp::Alu(Ine), [Some(e), imm(0), None])?;
        let a = op(self, SelOp::Alu(Bcsel), [Some(a2one), imm(one), Some(alpha)])?;
        if let Some((_, c)) = self.outputs.as_mut_slice().iter_mut().find(|(s, _)| *s == ir::IoSlot::FragColor(0)) {
            c[3] = Some(a);
        }
        self.alpha_to_one = Some((alpha, a2one));
        self.discard = Some(kill);
        Ok(())
    }

    /// `sin`/`cos` per component, as pco's `trans_trig` builds them:
    /// `fred` part A, part B, `fsinc`, `psel_trig`. The last writes the
    /// value's own register; the others fresh ones.
    fn trig(&mut self, id: ir::InstrId, t: Trig, arg: ir::Src) -> Result<(), SelectError> {
        for lane in 0..self.result_lanes(id) {
            let x = self.src(arg, lane)?;
            let a = self.fresh_group(SelOp::Machine(MachineOp::FredA(t)), [Some(x), None, None])?;
            let b = self.fresh_group(SelOp::Machine(MachineOp::FredB(t)), [Some(x), Some(a), None])?;
            let s = self.fresh_group(SelOp::Machine(MachineOp::Fsinc(t)), [Some(b), None, None])?;
            self.emit(SelGroup {
                op: SelOp::Machine(MachineOp::PselTrig),
                ty: OpType::F32,
                dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane))),
                srcs: [Some(s), Some(b), None],
            })?;
        }
        Ok(())
    }

    /// Point `local`'s `lane` at `from`'s register, or at a fresh copy of
    /// it when it is not a plain register.
    fn store(&mut self, local: ir::LocalId, lane: u32, from: SelSrc) -> Result<(), SelectError> {
        let r = match from {
            SelSrc::Virtual { v, mods } if mods == Mods::default() => v,
            other => {
                let t = VReg(self.next_vreg);
                self.next_vreg += 1;
                self.emit(SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(t)),
                    srcs: [Some(other), None, None],
                })?;
                t
            }
        };
        let at = (local.0 * LANES + lane) as usize;
        *self.locals_in.get_mut(at).ok_or(SelectError::Unsupported("a local slot the module does not declare"))? = r;
        Ok(())
    }

    /// `x / y` and `x % y` on 32-bit signed integers, exactly, as pco
    /// computes them: Mesa `nir_lower_idiv.c`, `emit_idiv` over `emit_udiv`
    /// (MIT, read and transcribed), which pco runs for every integer divide
    /// (`pco_nir.c`). The IR's `Imod` is the remainder with the dividend's
    /// sign (`eval.rs`: `wrapping_rem`), NIR's `irem`.
    ///
    /// ```text
    ///   a = |x|, b = |y|
    ///   r = f2u(rcp(u2f(b)) * 4294966784.0)       a reciprocal estimate
    ///   r = r + umul_high(r, r * -b)              one Newton step
    ///   q = umul_high(a, r), m = a - q * b        quotient and remainder
    ///   twice: if m >= b { q += 1; m -= b }       the two refinements
    ///   x / y = sign(x ^ y) < 0 ? -q : q,  x % y = x < 0 ? -m : m
    /// ```
    ///
    /// A zero divisor, and `INT_MIN / -1`, are undefined in GLSL; this
    /// computes whatever the sequence computes for them.
    fn idiv(&mut self, id: ir::InstrId, rem: bool, args: &[ir::Src; 3]) -> Result<(), SelectError> {
        for lane in 0..self.result_lanes(id) {
            let (x, y) = (self.src(args[0], lane)?, self.src(args[1], lane)?);
            for s in [x, y] {
                if matches!(s, SelSrc::Virtual { mods, .. } | SelSrc::Fixed { mods, .. } if mods != Mods::default()) {
                    return Err(SelectError::Unsupported("a float modifier on an integer divide's operand"));
                }
            }
            self.idiv_into(x, y, rem, SelDst::Virtual(Self::reg(ir::ValueId(id.0), lane)))?;
        }
        Ok(())
    }

    /// One lane of [`Sel::idiv`]: `x / y` (or `x % y` when `rem`) into `dst`.
    fn idiv_into(&mut self, x: SelSrc, y: SelSrc, rem: bool, dst: SelDst) -> Result<(), SelectError> {
        use ir::AluOp::*;
        let imm = |v: u32| Some(SelSrc::Imm(v));
        let a = self.fresh_group(SelOp::Machine(MachineOp::Iabs), [Some(x), None, None])?;
        let b = self.fresh_group(SelOp::Machine(MachineOp::Iabs), [Some(y), None, None])?;
        let res = self.udiv_value(a, b, rem)?;
        /* emit_idiv's sign: the quotient's is x ^ y's, the remainder's
         * the dividend's. */
        let sign_of = if rem { x } else { self.fresh_typed(SelOp::Alu(Ixor), OpType::U32, [Some(x), Some(y), None])? };
        let neg = self.fresh_typed(SelOp::Alu(Ilt), OpType::S32, [Some(sign_of), imm(0), None])?;
        let nres = self.fresh_typed(SelOp::Alu(Ineg), OpType::S32, [Some(res), None, None])?;
        self.emit(SelGroup {
            op: SelOp::Alu(Bcsel),
            ty: OpType::U32,
            dst: Some(dst),
            srcs: [Some(neg), Some(nres), Some(res)],
        })?;
        Ok(())
    }

    /// One lane of GLSL ES 3.00's unsigned `x / y` (or `x % y` when `rem`)
    /// into `dst`: nir_lower_idiv's `emit_udiv` on the operands as they are.
    fn udiv_into(&mut self, x: SelSrc, y: SelSrc, rem: bool, dst: SelDst) -> Result<(), SelectError> {
        let res = self.udiv_value(x, y, rem)?;
        self.emit(SelGroup { op: SelOp::Copy, ty: OpType::U32, dst: Some(dst), srcs: [Some(res), None, None] })
    }

    /// nir_lower_idiv's `emit_udiv(a, b)` (MIT, transcribed): a reciprocal
    /// estimate, one umul_high refinement of it, the quotient and remainder
    /// from it, and two corrections. The quotient, or the remainder when
    /// `rem`.
    fn udiv_value(&mut self, a: SelSrc, b: SelSrc, rem: bool) -> Result<SelSrc, SelectError> {
        use ir::AluOp::*;
        let imm = |v: u32| Some(SelSrc::Imm(v));
        let fb = self.fresh_group(SelOp::Machine(MachineOp::U2f), [Some(b), None, None])?;
        let fr = self.fresh_typed(SelOp::Alu(Frcp), OpType::F32, [Some(fb), None, None])?;
        let fr =
            self.fresh_typed(SelOp::Alu(Fmul), OpType::F32, [Some(fr), imm(4_294_966_784.0f32.to_bits()), None])?;
        let r = self.fresh_group(SelOp::Machine(MachineOp::F2u), [Some(fr), None, None])?;
        let nb = self.fresh_typed(SelOp::Alu(Ineg), OpType::S32, [Some(b), None, None])?;
        let rnb = self.fresh_typed(SelOp::Alu(Imul), OpType::S32, [Some(r), Some(nb), None])?;
        let hi = self.fresh_group(SelOp::Machine(MachineOp::UmulHi), [Some(r), Some(rnb), None])?;
        let r = self.fresh_typed(SelOp::Alu(Iadd), OpType::S32, [Some(r), Some(hi), None])?;
        let q = self.fresh_group(SelOp::Machine(MachineOp::UmulHi), [Some(a), Some(r), None])?;
        let qb = self.fresh_typed(SelOp::Alu(Imul), OpType::S32, [Some(q), Some(b), None])?;
        let nqb = self.fresh_typed(SelOp::Alu(Ineg), OpType::S32, [Some(qb), None, None])?;
        let mut m = self.fresh_typed(SelOp::Alu(Iadd), OpType::S32, [Some(a), Some(nqb), None])?;
        let mut q = q;
        for _ in 0..2 {
            let ge = self.fresh_group(SelOp::Machine(MachineOp::Uge), [Some(m), Some(b), None])?;
            if !rem {
                let q1 = self.fresh_typed(SelOp::Alu(Iadd), OpType::S32, [Some(q), imm(1), None])?;
                q = self.fresh_group(SelOp::Alu(Bcsel), [Some(ge), Some(q1), Some(q)])?;
            }
            let m1 = self.fresh_typed(SelOp::Alu(Iadd), OpType::S32, [Some(m), Some(nb), None])?;
            m = self.fresh_group(SelOp::Alu(Bcsel), [Some(ge), Some(m1), Some(m)])?;
        }
        Ok(if rem { m } else { q })
    }

    /// [`Sel::fresh_group`] with the operation's own type.
    fn fresh_typed(&mut self, op: SelOp, ty: OpType, srcs: [Option<SelSrc>; 3]) -> Result<SelSrc, SelectError> {
        let d = VReg(self.next_vreg);
        self.next_vreg += 1;
        self.emit(SelGroup { op, ty, dst: Some(SelDst::Virtual(d)), srcs })?;
        Ok(SelSrc::Virtual { v: d, mods: Mods::default() })
    }

    /// `gl_FragCoord.xy`, as pco reads it (Mesa `pco_trans_nir.c`,
    /// `trans_load_input_fs`, MIT, read and transcribed): the pixel centre's
    /// special registers `x_p`/`y_p` (97, 100), or `x_s`/`y_s` (98, 101)
    /// when the draw is multisampled, chosen at run time by
    /// `fs_is_single_sampled` -- the coverage mask of 1.0, shifted right by
    /// one, tested for zero. A special-class register is read through S1
    /// only (`movs1`). The +0.5 is already in them (gl_test case 82 on the
    /// AM62P, both targets).
    ///
    /// y is then `y * scale + offset` with the pair the driver writes per
    /// draw at [`ytf_dword`]: (-1, height) on a target it flips, the
    /// window, and (1, 0) otherwise -- case 82 measured only y mirrored on
    /// the window, and GL counts y up from the bottom.
    fn frag_coord_prologue(&mut self) -> Result<(), SelectError> {
        let Some(pair) = ytf_dword(self.m) else { return Ok(()) };
        let plain = Mods::default();
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let special = |r: u16| Some(SelSrc::Fixed { bank: Bank::Special, reg: r, mods: plain });
        let mask = self.fresh_group(SelOp::Machine(MachineOp::PckCov), [imm(1.0f32.to_bits()), None, None])?;
        let half = self.fresh_group(SelOp::Machine(MachineOp::Ushr), [Some(mask), imm(1), None])?;
        let single = self.fresh_group(SelOp::Machine(MachineOp::Tstz), [Some(half), None, None])?;
        let mut xy = [None; 2];
        for (k, (p, smp)) in [(SR_X_P, SR_X_S), (SR_Y_P, SR_Y_S)].into_iter().enumerate() {
            let at_pixel = self.fresh_group(SelOp::Machine(MachineOp::MovS1), [special(p), None, None])?;
            let at_sample = self.fresh_group(SelOp::Machine(MachineOp::MovS1), [special(smp), None, None])?;
            xy[k] =
                Some(self.fresh_group(SelOp::Alu(ir::AluOp::Bcsel), [Some(single), Some(at_pixel), Some(at_sample)])?);
        }
        let shared = |r: u16| Some(SelSrc::Fixed { bank: Bank::Shared, reg: r, mods: plain });
        let y = VReg(self.next_vreg);
        self.next_vreg += 1;
        self.emit(SelGroup {
            op: SelOp::Alu(ir::AluOp::Ffma),
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(y)),
            srcs: [xy[1], shared(pair), shared(pair + 1)],
        })?;
        let Some(SelSrc::Virtual { v: x, .. }) = xy[0] else { return Err(SelectError::OutOfScratch) };
        self.frag_coord = [Some(x), Some(y)];
        Ok(())
    }

    /// `gl_FrontFacing` as pco computes it (Mesa `pco_nir_pvfio.c`
    /// lower_front_face, MIT, read and transcribed): front is the ISP's
    /// `face_ccw` being zero, then the driver's op at [`ff_dword`] chooses,
    /// as `nir_select_from_ssa_def_array` builds it:
    ///
    /// ```text
    ///   r = front;  r = op == 1 ? !front : r;  r = op == 2 ? true : r
    /// ```
    ///
    /// 0 keeps the ISP's sense, 1 swaps it, 2 is every point and line
    /// (src/pvr/backend.rs, which writes it per draw).
    fn front_face_prologue(&mut self) -> Result<(), SelectError> {
        let Some(op) = ff_dword(self.m) else { return Ok(()) };
        let plain = Mods::default();
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let special = Some(SelSrc::Fixed { bank: Bank::Special, reg: SR_FACE_CCW, mods: plain });
        let op = Some(SelSrc::Fixed { bank: Bank::Shared, reg: op, mods: plain });
        let face = self.fresh_group(SelOp::Machine(MachineOp::MovS1), [special, None, None])?;
        let front = self.fresh_group(SelOp::Machine(MachineOp::Tstz), [Some(face), None, None])?;
        let back = self.fresh_group(SelOp::Machine(MachineOp::Tstz), [Some(front), None, None])?;
        let swap = self.fresh_group(SelOp::Alu(ir::AluOp::Ieq), [op, imm(1), None])?;
        let r = self.fresh_group(SelOp::Alu(ir::AluOp::Bcsel), [Some(swap), Some(back), Some(front)])?;
        let always = self.fresh_group(SelOp::Alu(ir::AluOp::Ieq), [op, imm(2), None])?;
        let r = self.fresh_group(SelOp::Alu(ir::AluOp::Bcsel), [Some(always), imm(u32::MAX), Some(r)])?;
        let SelSrc::Virtual { v, .. } = r else { return Err(SelectError::OutOfScratch) };
        self.front_facing = Some(v);
        Ok(())
    }

    /// One epilogue group into a fresh register, which it returns.
    /// The first of the four registers a texture sample of `id` writes:
    /// its own value's, or [`Sel::tex_dst`] while a shadow look-up samples.
    fn tex_out(&self, id: ir::InstrId) -> VReg {
        self.tex_dst.unwrap_or(Self::reg(ir::ValueId(id.0), 0))
    }

    /// A shadow look-up's result from the sampled depth `d` and the
    /// reference `r` (ES 3.0.6 3.8.15.1, table 3.23): 1.0 when `r OP d`
    /// holds, else 0.0,
    /// into lane 0 of `id`.
    ///
    /// OP is read when the shader runs, from the sampler's descriptor:
    /// sampler meta word 0 (dword 12 of pvr_combined_image_sampler_descriptor,
    /// Mesa's PCO_SAMPLER_META_COMPARE_OP), which the backend fills with the
    /// compare function less GL_NEVER -- bit 0 "less", bit 1 "equal", bit 2
    /// "greater", so 0 is NEVER and 7 ALWAYS, the same bits as Vulkan's
    /// VkCompareOp that Mesa stores there. One program then serves every
    /// function, as a sampler object can change it without a recompile.
    ///
    /// The reference is clamped to [0, 1] when the depth is fixed-point
    /// (3.8.15.1): bit 3 of the same word. Mesa reads that from the image meta
    /// PCK_INFO (dword 7); here dword 7 is left as the table's programs read
    /// it, and depth is stored as floats whatever its internal format, so
    /// the GL core says which it was (GlesTextureBind::compare_op).
    fn shadow_compare(&mut self, id: ir::InstrId, sampler: u8, d: SelSrc, r: SelSrc) -> Result<(), SelectError> {
        use ir::AluOp::{Bcsel, Feq, Flt, Fmax, Fmin, Iand, Ine, Ior};
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
        let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
        let word = |s: &mut Self, w: u8| s.fresh_group(SelOp::TexWord { binding: sampler, word: w }, [None; 3]);

        let op = word(self, 12)?;
        let fixed_bit = g(self, Iand, [Some(op), imm(8), None])?;
        let fixed = g(self, Ine, [Some(fixed_bit), imm(0), None])?;
        let lo = g(self, Fmax, [Some(r), f(0.0), None])?;
        let clamped = g(self, Fmin, [Some(lo), f(1.0), None])?;
        let r = g(self, Bcsel, [Some(fixed), Some(clamped), Some(r)])?;

        let less = g(self, Flt, [Some(r), Some(d), None])?;
        let equal = g(self, Feq, [Some(r), Some(d), None])?;
        let greater = g(self, Flt, [Some(d), Some(r), None])?;
        let mut any = None;
        for (holds, bit) in [(less, 1u32), (equal, 2), (greater, 4)] {
            let wanted = g(self, Iand, [Some(op), imm(bit), None])?;
            let hit = g(self, Bcsel, [Some(holds), Some(wanted), imm(0)])?;
            any = Some(match any {
                None => hit,
                Some(acc) => g(self, Ior, [Some(acc), Some(hit), None])?,
            });
        }
        let pass = g(self, Ine, [any, imm(0), None])?;
        let compared = g(self, Bcsel, [Some(pass), f(1.0), f(0.0)])?;
        /* bit 4: TEXTURE_COMPARE_MODE is NONE, and "r = Dt" (3.8.15.1) */
        let off_bit = g(self, Iand, [Some(op), imm(16), None])?;
        let off = g(self, Ine, [Some(off_bit), imm(0), None])?;
        self.emit(SelGroup {
            op: SelOp::Alu(Bcsel),
            ty: op_type(Bcsel),
            dst: Some(SelDst::Virtual(Self::reg(ir::ValueId(id.0), 0))),
            srcs: [Some(off), Some(d), Some(compared)],
        })
    }

    fn fresh_group(&mut self, op: SelOp, srcs: [Option<SelSrc>; 3]) -> Result<SelSrc, SelectError> {
        self.fresh_typed(op, OpType::U32, srcs)
    }

    /// A field of the control word: `ubfe`, then the select pco's `ubfe`
    /// lowering leaves against a zero width.
    fn meta_field(&mut self, meta: SelSrc, (off, bits): (u32, u32)) -> Result<SelSrc, SelectError> {
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let e = self.fresh_group(SelOp::Machine(MachineOp::Ubfe), [Some(meta), imm(off), imm(bits)])?;
        self.fresh_group(SelOp::Alu(ir::AluOp::Bcsel), [imm(bits), Some(e), imm(0)])
    }

    /// How a sample of `lod` chooses its level, and the data word that
    /// carries it: none for the implicit level, the level for `textureLod`
    /// (`lodm` replace), the bias for `texture(s, p, bias)` (`lodm` bias) --
    /// pco_nir_tex.c pco_emit_nir_smp puts either word after the
    /// coordinates, with `pplod`.
    fn smp_level(&self, lod: ir::TexLod) -> Result<(SmpLod, Option<SelSrc>), SelectError> {
        Ok(match lod {
            ir::TexLod::Implicit => (SmpLod::Implicit, None),
            ir::TexLod::Lod(l) => (SmpLod::Replace, Some(self.src(l, 0)?)),
            ir::TexLod::Bias(b) => (SmpLod::Bias, Some(self.src(b, 0)?)),
            _ => return Err(SelectError::Unsupported("a sample level that is neither implicit, replaced nor biased")),
        })
    }

    /// `textureSize(s, lod)` of a 2D image, width and height, as pco computes
    /// it (usclib/tex.cl usclib_tex_state_size, MIT, for two components):
    /// width and height less one from the image state's first pair (bits
    /// 34..47, 48..61), the base level (the second pair's bits 60..63) added
    /// to `lod` unless the image is a STRIDE one (textype 4, bits 0..2) --
    /// YakoGL's copy of it, which the table's programs were compiled with --
    /// each size shifted right by that level and at least 1.
    fn tex_size_2d(&mut self, sampler: u8, lod: SelSrc) -> Result<[SelSrc; 2], SelectError> {
        use ir::AluOp::{Bcsel, Iadd, Iand, Ieq};
        let imm = |x: u32| Some(SelSrc::Imm(x));
        let word = |s: &mut Self, w: u8| s.fresh_group(SelOp::TexWord { binding: sampler, word: w }, [None; 3]);
        let (w0, w1, w3) = (word(self, 0)?, word(self, 1)?, word(self, 3)?);
        let textype = self.fresh_group(SelOp::Alu(Iand), [Some(w0), imm(7), None])?;
        let stride = self.fresh_group(SelOp::Alu(Ieq), [Some(textype), imm(4), None])?;
        let base = self.fresh_group(SelOp::Machine(MachineOp::Ushr), [Some(w3), imm(28), None])?;
        let plus = self.fresh_group(SelOp::Alu(Iadd), [Some(lod), Some(base), None])?;
        let level = self.fresh_group(SelOp::Alu(Bcsel), [Some(stride), Some(lod), Some(plus)])?;
        let mut out = [SelSrc::Imm(0); 2];
        for (lane, off) in [(0usize, 2u32), (1, 16)] {
            let less_one = self.meta_field(w1, (off, 14))?;
            let size = self.fresh_group(SelOp::Alu(Iadd), [Some(less_one), imm(1), None])?;
            let at = self.fresh_group(SelOp::Machine(MachineOp::Ushr), [Some(size), Some(level), None])?;
            let some = self.fresh_group(SelOp::Machine(MachineOp::Uge), [Some(at), imm(1), None])?;
            out[lane] = self.fresh_group(SelOp::Alu(Bcsel), [Some(some), Some(at), imm(1)])?;
        }
        Ok(out)
    }

    /// Where a cube look-up samples the stacked 2D image YakoGL keeps a cube
    /// in (six faces one above the other, +X first): glsl2usc's `pvr_cubeUV`
    /// (tools/yakogl-tools/src/glsl.rs EMU_HELPERS), which the table's
    /// programs sample through, one operation for one. The major axis and
    /// the face are chosen with selects where the helper branches (both
    /// compute every candidate the same way, so the values are the helper's):
    ///
    /// ```text
    /// x major (|x| >= |y| and |x| >= |z|): face 0 (-z, -y) if x > 0, else 1 (z, -y)
    /// else y major (|y| >= |z|):             face 2 (x, z)   if y > 0, else 3 (x, -z)
    /// else z major:                          face 4 (x, -y)  if z > 0, else 5 (-x, -y)
    /// uv = (uv / ma) * 0.5 + 0.5, clamped half a texel of the face's height
    /// (textureSize(s, 0).y / 6) in from its edges, v = (v + face) / 6
    /// ```
    fn cube_uv(&mut self, sampler: u8, coord: ir::Src, lod: Option<SelSrc>) -> Result<(SelSrc, SelSrc), SelectError> {
        use ir::AluOp::{Bcsel, Fadd, Fceil, Fexp2, Ffma, Fle, Flt, Fmax, Fmin, Fmul, Frcp, I2f, Iand};
        let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
        let mut d = [SelSrc::Imm(0); 3];
        for (k, dk) in d.iter_mut().enumerate() {
            let c = self.src(coord, k as u32)?;
            *dk = self.fresh_typed(SelOp::Copy, OpType::F32, [Some(c), None, None])?;
        }
        let mut abs = [SelSrc::Imm(0); 3];
        let mut neg = [SelSrc::Imm(0); 3];
        /* |d| and -d as values of their own: a modifier is a source
         * modifier on this machine, so each is a move carrying it. */
        let modded = |x: SelSrc, neg: bool, abs: bool| match x {
            SelSrc::Virtual { v, .. } => Some(SelSrc::Virtual { v, mods: Mods { neg, abs, flr: false } }),
            _ => None,
        };
        for k in 0..3 {
            abs[k] = self.fresh_typed(SelOp::Copy, OpType::F32, [modded(d[k], false, true), None, None])?;
            neg[k] = self.fresh_typed(SelOp::Copy, OpType::F32, [modded(d[k], true, false), None, None])?;
        }
        let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
        let y_le_x = g(self, Fle, [Some(abs[1]), Some(abs[0]), None])?;
        let z_le_x = g(self, Fle, [Some(abs[2]), Some(abs[0]), None])?;
        let x_major = g(self, Iand, [Some(y_le_x), Some(z_le_x), None])?;
        let y_major = g(self, Fle, [Some(abs[2]), Some(abs[1]), None])?;
        let mut pos = [SelSrc::Imm(0); 3];
        for k in 0..3 {
            pos[k] = g(self, Flt, [f(0.0), Some(d[k]), None])?;
        }
        let pick = |s: &mut Self, c: SelSrc, a: Option<SelSrc>, b: Option<SelSrc>| {
            s.fresh_typed(SelOp::Alu(Bcsel), op_type(Bcsel), [Some(c), a, b])
        };
        /* each major axis: (face, u, v) */
        let fx = pick(self, pos[0], f(0.0), f(1.0))?;
        let ux = pick(self, pos[0], Some(neg[2]), Some(d[2]))?;
        let fy = pick(self, pos[1], f(2.0), f(3.0))?;
        let vy = pick(self, pos[1], Some(d[2]), Some(neg[2]))?;
        let fz = pick(self, pos[2], f(4.0), f(5.0))?;
        let uz = pick(self, pos[2], Some(d[0]), Some(neg[0]))?;
        let choose = |s: &mut Self, x: SelSrc, y: SelSrc, z: SelSrc| -> Result<SelSrc, SelectError> {
            let yz = pick(s, y_major, Some(y), Some(z))?;
            pick(s, x_major, Some(x), Some(yz))
        };
        let ma = choose(self, abs[0], abs[1], abs[2])?;
        let face = choose(self, fx, fy, fz)?;
        let u = choose(self, ux, d[0], uz)?;
        let v = choose(self, neg[1], vy, neg[1])?;
        /* uv = (uv / ma) * 0.5 + 0.5 */
        let r = g(self, Frcp, [Some(ma), None, None])?;
        let mut uv = [u, v];
        for c in uv.iter_mut() {
            let q = g(self, Fmul, [Some(*c), Some(r), None])?;
            *c = g(self, Ffma, [Some(q), f(0.5), f(0.5)])?;
        }
        /* h = float(textureSize(s, 0).y) / 6.0; clamp(uv, 0.5 / h, 1.0 - 0.5 / h)
         *
         * With an explicit level the face is that level's: h is taken at the
         * coarser of the two levels a trilinear filter reads,
         * h = max(h * exp2(-ceil(max(lod, 0.0))), 1.0) (glsl2usc's
         * pvr_cubeLod). Clamped at level 0 instead, a level where a face is
         * a texel or two tall blended the rows of the neighbouring faces:
         * dEQP texture_functions.vertex.texturecubelod on the AM62P drew a
         * gradient across the high-LOD corner where the level is one flat
         * colour (am62p-board, 30899c1f). An implicit level or a bias is
         * the hardware's to choose, and keeps the level-0 clamp.
         *
         * Only v is clamped. A face spans the stack's whole width, so the
         * sampler's own wrap in u stays inside the face and is the face's:
         * GLES2 cube maps are not seamless and REPEAT reads the opposite
         * edge of the same face (dEQP's reference, mapGLSampler with
         * seamlessCubeMap false). Clamping u at the level's inset took that
         * away and texture.vertex.cube.*_repeat failed by 18 over 16 on the
         * AM62P (am62p-board, a60104c2); the level-0 inset had hidden it
         * behind a sub-texel margin. As pvr_arrUV does for an array. */
        let [_, height] = self.tex_size_2d(sampler, SelSrc::Imm(0))?;
        let hf = g(self, I2f, [Some(height), None, None])?;
        let mut h = g(self, Fmul, [Some(hf), f(1.0 / 6.0), None])?;
        if let Some(l) = lod {
            let lp = g(self, Fmax, [Some(l), f(0.0), None])?;
            let e = g(self, Fceil, [Some(lp), None, None])?;
            let s = g(self, Fexp2, [modded(e, true, false), None, None])?;
            let hl = g(self, Fmul, [Some(h), Some(s), None])?;
            h = g(self, Fmax, [Some(hl), f(1.0), None])?;
        }
        let rh = g(self, Frcp, [Some(h), None, None])?;
        let lo = g(self, Fmul, [f(0.5), Some(rh), None])?;
        let nlo = self.fresh_typed(SelOp::Copy, OpType::F32, [modded(lo, true, false), None, None])?;
        let hi = g(self, Fadd, [f(1.0), Some(nlo), None])?;
        let m = g(self, Fmax, [Some(uv[1]), Some(lo), None])?;
        uv[1] = g(self, Fmin, [Some(m), Some(hi), None])?;
        /* v = (v + face) / 6.0 */
        let vf = g(self, Fadd, [Some(uv[1]), Some(face), None])?;
        let v = g(self, Fmul, [Some(vf), f(1.0 / 6.0), None])?;
        Ok((uv[0], v))
    }

    /// The block dword the GL core writes sampler `binding`'s layer count to:
    /// the hidden uniform `pvr_layers_<sampler>` (the contract glsl2usc's
    /// stacked-texture emulation and draw.rs fill_stage_layer_uniforms
    /// share), found by name.
    fn layers_dword(&self, binding: u8) -> Result<u16, SelectError> {
        let sampler = self
            .m
            .uniforms
            .iter()
            .find(|u| u.binding == binding && u.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK)
            .ok_or(SelectError::Unsupported("a stacked-texture look-up whose sampler has no uniform row"))?;
        self.m
            .uniforms
            .iter()
            .find(|u| {
                u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK
                    && u.name.strip_prefix("pvr_layers_") == Some(sampler.name)
            })
            .map(|u| u.dword_off)
            .ok_or(SelectError::Unsupported(
                "a 2D array or 3D look-up: the front end does not declare its pvr_layers_ uniform yet",
            ))
    }

    /// Where a 2D array look-up samples the stacked 2D image YakoGL keeps
    /// the array in (the layers one above the other): glsl2usc's
    /// `pvr_arrUV` (tools/yakogl-tools/src/glsl.rs EMU_HELPERS), one
    /// operation for one, the layer count from `pvr_layers_<sampler>`:
    ///
    /// ```text
    /// l = clamp(floor(c.z + 0.5), 0.0, layers - 1.0)
    /// h = float(textureSize(s, 0).y) / layers
    /// v = clamp(c.y, 0.5 / h, 1.0 - 0.5 / h)
    /// uv = (c.x, (v + l) / layers)
    /// ```
    fn array_uv(&mut self, sampler: u8, coord: ir::Src) -> Result<(SelSrc, SelSrc), SelectError> {
        use ir::AluOp::{Fadd, Fmax, Fmin, Fmul, Frcp, I2f};
        let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
        let layers = SelSrc::Fixed { bank: Bank::Shared, reg: self.layers_dword(sampler)?, mods: Mods::default() };
        let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
        let u = self.src(coord, 0)?;
        let y = self.src(coord, 1)?;
        let z = self.src(coord, 2)?;
        /* floor is a source modifier on this machine: a move carrying it */
        let zh = g(self, Fadd, [Some(z), f(0.5), None])?;
        let SelSrc::Virtual { v: zv, .. } = zh else { return Err(SelectError::OutOfScratch) };
        let floored = SelSrc::Virtual { v: zv, mods: Mods { neg: false, abs: false, flr: true } };
        let fl = self.fresh_typed(SelOp::Copy, OpType::F32, [Some(floored), None, None])?;
        let last = g(self, Fadd, [Some(layers), f(-1.0), None])?;
        let lo0 = g(self, Fmax, [Some(fl), f(0.0), None])?;
        let l = g(self, Fmin, [Some(lo0), Some(last), None])?;
        let [_, height] = self.tex_size_2d(sampler, SelSrc::Imm(0))?;
        let hf = g(self, I2f, [Some(height), None, None])?;
        let rl = g(self, Frcp, [Some(layers), None, None])?;
        let h = g(self, Fmul, [Some(hf), Some(rl), None])?;
        let rh = g(self, Frcp, [Some(h), None, None])?;
        let lo = g(self, Fmul, [f(0.5), Some(rh), None])?;
        let SelSrc::Virtual { v: lov, .. } = lo else { return Err(SelectError::OutOfScratch) };
        let nlo = self.fresh_typed(
            SelOp::Copy,
            OpType::F32,
            [Some(SelSrc::Virtual { v: lov, mods: Mods { neg: true, abs: false, flr: false } }), None, None],
        )?;
        let hi = g(self, Fadd, [f(1.0), Some(nlo), None])?;
        let m = g(self, Fmax, [Some(y), Some(lo), None])?;
        let v = g(self, Fmin, [Some(m), Some(hi), None])?;
        let vl = g(self, Fadd, [Some(v), Some(l), None])?;
        let out_v = g(self, Fmul, [Some(vl), Some(rl), None])?;
        Ok((u, out_v))
    }

    /// A move of `x` carrying a source modifier (floor, negate): the value
    /// of the modifier on its own, which this machine has no operation for.
    fn modified(&mut self, x: SelSrc, mods: Mods) -> Result<SelSrc, SelectError> {
        let SelSrc::Virtual { v, .. } = x else { return Err(SelectError::OutOfScratch) };
        self.fresh_typed(SelOp::Copy, OpType::F32, [Some(SelSrc::Virtual { v, mods }), None, None])
    }

    /// A 3D texture's slice grid in integers, as glsl2usc's pvr_volSize and
    /// pvr_volFetch compute it: `cols = int(ceil(sqrt(layers - 0.5)))`,
    /// `rows = (int(layers) + cols - 1) / cols`, and one slice's size
    /// `textureSize(s, 0) / ivec2(cols, rows)`. Returns (cols, the slice's
    /// width and height, int(layers)).
    fn vol_grid(&mut self, sampler: u8, layers: SelSrc) -> Result<(SelSrc, [SelSrc; 2], SelSrc), SelectError> {
        use ir::AluOp::{F2i, Fadd, Fceil, Frcp, Frsq, Iadd};
        let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
        let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
        let lm = g(self, Fadd, [Some(layers), f(-0.5), None])?;
        let rs = g(self, Frsq, [Some(lm), None, None])?;
        let sq = g(self, Frcp, [Some(rs), None, None])?;
        let cf = g(self, Fceil, [Some(sq), None, None])?;
        let cols = g(self, F2i, [Some(cf), None, None])?;
        let n = g(self, F2i, [Some(layers), None, None])?;
        let nc = g(self, Iadd, [Some(n), Some(cols), None])?;
        let nc1 = g(self, Iadd, [Some(nc), Some(SelSrc::Imm((-1i32) as u32)), None])?;
        let div = |s: &mut Self, x: SelSrc, y: SelSrc| -> Result<SelSrc, SelectError> {
            let d = VReg(s.next_vreg);
            s.next_vreg += 1;
            s.idiv_into(x, y, false, SelDst::Virtual(d))?;
            Ok(SelSrc::Virtual { v: d, mods: Mods::default() })
        };
        let rows = div(self, nc1, cols)?;
        let [w, h] = self.tex_size_2d(sampler, SelSrc::Imm(0))?;
        let sw = div(self, w, cols)?;
        let sh = div(self, h, rows)?;
        Ok((cols, [sw, sh], n))
    }

    /// Slice `k` of a 3D texture, at `(u, v)`, in the stacked 2D image YakoGL
    /// keeps it in (the slices in a grid, `cols` across): glsl2usc's
    /// `pvr_volUV` (tools/yakogl-tools/src/glsl.rs EMU_HELPERS), one
    /// operation for one:
    ///
    /// ```text
    /// cols = ceil(sqrt(layers - 0.5)); rows = ceil(layers / cols)
    /// sz = vec2(textureSize(s, 0)) / vec2(cols, rows)
    /// st = clamp(uv, 0.5 / sz, 1.0 - 0.5 / sz)
    /// (vec2(mod(k, cols), floor(k / cols)) + st) / vec2(cols, rows)
    /// ```
    fn vol_uv(&mut self, sampler: u8, layers: SelSrc, uv: [SelSrc; 2], k: SelSrc) -> Result<[SelSrc; 2], SelectError> {
        use ir::AluOp::{Fadd, Fceil, Ffma, Fmax, Fmin, Fmul, Frcp, Frsq, I2f};
        let f = |x: f32| Some(SelSrc::Imm(x.to_bits()));
        let g = |s: &mut Self, op: ir::AluOp, x: [Option<SelSrc>; 3]| s.fresh_typed(SelOp::Alu(op), op_type(op), x);
        let flr = Mods { neg: false, abs: false, flr: true };
        let neg = Mods { neg: true, abs: false, flr: false };
        let lm = g(self, Fadd, [Some(layers), f(-0.5), None])?;
        /* sqrt is frcp(frsq(x)), as pco lowers it (nir lower_fsqrt) */
        let rs = g(self, Frsq, [Some(lm), None, None])?;
        let sq = g(self, Frcp, [Some(rs), None, None])?;
        let cols = g(self, Fceil, [Some(sq), None, None])?;
        let rcols = g(self, Frcp, [Some(cols), None, None])?;
        let lc = g(self, Fmul, [Some(layers), Some(rcols), None])?;
        let rows = g(self, Fceil, [Some(lc), None, None])?;
        let rrows = g(self, Frcp, [Some(rows), None, None])?;
        let size = self.tex_size_2d(sampler, SelSrc::Imm(0))?;
        let mut st = [SelSrc::Imm(0); 2];
        for (lane, r) in [(0usize, rcols), (1, rrows)] {
            let whf = g(self, I2f, [Some(size[lane]), None, None])?;
            let sz = g(self, Fmul, [Some(whf), Some(r), None])?;
            let rsz = g(self, Frcp, [Some(sz), None, None])?;
            let lo = g(self, Fmul, [f(0.5), Some(rsz), None])?;
            let nlo = self.modified(lo, neg)?;
            let hi = g(self, Fadd, [f(1.0), Some(nlo), None])?;
            let m = g(self, Fmax, [Some(uv[lane]), Some(lo), None])?;
            st[lane] = g(self, Fmin, [Some(m), Some(hi), None])?;
        }
        /* mod(k, cols) = k - cols * floor(k / cols) */
        let kc = g(self, Fmul, [Some(k), Some(rcols), None])?;
        let row = self.modified(kc, flr)?;
        let ncols = self.modified(cols, neg)?;
        let col = g(self, Ffma, [Some(ncols), Some(row), Some(k)])?;
        let cu = g(self, Fadd, [Some(col), Some(st[0]), None])?;
        let cv = g(self, Fadd, [Some(row), Some(st[1]), None])?;
        Ok([g(self, Fmul, [Some(cu), Some(rcols), None])?, g(self, Fmul, [Some(cv), Some(rrows), None])?])
    }

    /// Give zero to every local slot register the program can read before
    /// anything writes it.
    ///
    /// GLSL leaves a local's value undefined until it is assigned, and
    /// if-conversion reads a slot's "before" value on both sides of every
    /// `if`: a slot first assigned inside an arm (an inlined function's
    /// return value, a variable declared in the arm) is read unwritten by the
    /// join, and on the hardware that read returns whatever the register
    /// last held -- another invocation's data. The select never PICKS it on
    /// a path where it matters, but that is a property of the conditions,
    /// which the corpus check cannot see (SaturationLightness.frag's nested
    /// early returns). A zero costs one move per such register and makes the
    /// program read nothing it did not write.
    fn zero_unwritten_locals(&mut self) -> Result<(), SelectError> {
        let lo = self.m.instrs.len() as u32 * LANES;
        let hi = lo + Self::slots(self.m) * LANES;
        let is_home = |v: VReg| (lo..hi).contains(&v.0);
        /* One flag per home register, written and already listed, instead of
         * searching two growing lists: this ran once per operand over every
         * group, quadratic on dEQP's tmp_array shapes (a 40-element local
         * array, thousands of home writes). All given back at the end. */
        let span = (hi - lo) as usize;
        let mut written_v = filled_vec(self.arena, span, false)?;
        let mut listed_v = filled_vec(self.arena, span, false)?;
        let (written, listed) = (written_v.as_mut_slice(), listed_v.as_mut_slice());
        let at = |v: u32| (v - lo) as usize;
        let mut need: AVec<'a, u32> = AVec::new();
        for g in self.groups.as_slice() {
            for src in g.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = src {
                    if is_home(*v) && !written[at(v.0)] && !listed[at(v.0)] {
                        listed[at(v.0)] = true;
                        need.push(self.arena, v.0)?;
                    }
                }
            }
            if let Some(SelDst::Virtual(v)) = g.dst {
                if is_home(v) {
                    written[at(v.0)] = true;
                }
            }
        }
        /* A vertex shader's varying component that holds a slot lane no
         * group on any path writes is undefined everywhere, and pco writes
         * no output for it (NIR sees an undef): Qt's particle shaders set
         * `spriteData.x` and never `.y`. Drop the write rather than zero it,
         * so the program writes the dwords pco's does. */
        if self.m.stage != SHC_STAGE_FRAGMENT {
            for (slot, comps) in self.outputs.as_mut_slice().iter_mut() {
                if !matches!(slot, ir::IoSlot::Varying(_)) {
                    continue;
                }
                for c in comps.iter_mut() {
                    if let Some(SelSrc::Virtual { v, .. }) = c {
                        if is_home(*v) && !written[at(v.0)] {
                            *c = None;
                        }
                    }
                }
            }
        }
        /* The outputs and the discard flag are read too, by the builder
         * after the last group: a slot lane no path wrote reaches them
         * directly now that a store moves the slot rather than writing its
         * register (`Sel::store`). */
        for src in
            self.outputs.as_slice().iter().flat_map(|(_, comps)| comps.iter().flatten()).chain(self.discard.iter())
        {
            if let SelSrc::Virtual { v, .. } = src {
                if is_home(*v) && !written[at(v.0)] && !listed[at(v.0)] {
                    listed[at(v.0)] = true;
                    need.push(self.arena, v.0)?;
                }
            }
        }
        if need.is_empty() {
            need.release(self.arena);
            listed_v.release(self.arena);
            written_v.release(self.arena);
            return Ok(());
        }
        let mut out: AVec<'a, SelGroup> = AVec::with_capacity(self.arena, need.len() + self.groups.len())?;
        for &v in need.as_slice() {
            out.push(
                self.arena,
                SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::F32,
                    dst: Some(SelDst::Virtual(VReg(v))),
                    srcs: [Some(SelSrc::Imm(0)), None, None],
                },
            )?;
        }
        out.extend_from_slice(self.arena, self.groups.as_slice())?;
        need.release(self.arena);
        listed_v.release(self.arena);
        written_v.release(self.arena);
        core::mem::replace(&mut self.groups, out).release(self.arena);
        Ok(())
    }

    /// Does this arm, at any depth, hold control flow other than `if` and a
    /// loop kept as a loop? Such a loop runs whichever arm it is in, as the
    /// arm's instructions do, and the join keeps or drops what it computed:
    /// it stores no output (`loop_` refuses one) and ends at its count.
    /// May this arm of an `if` run predicated (pco_cf.c can_pred_exec)? Only
    /// straight-line arithmetic: no nested `if` or loop, no jump, and no
    /// operation whose groups read or write p0 or need every instance of
    /// the quad -- a look-up (its data fence takes no condition), a varying
    /// read (the iteration), a derivative, sine and cosine (`fsinc` writes
    /// p0, `psel_trig` reads it), a discard, an indexed uniform read.
    fn predicable(&self, r: ir::Range) -> bool {
        use ir::AluOp::{Fcos, Fddx, Fddy, Fsin};
        self.m.nodes_of(r).iter().all(|n| match *n {
            ir::Node::Instr(id) => match self.m.instr(id) {
                Some(ir::Op::Alu(Fsin | Fcos | Fddx | Fddy, _)) => false,
                Some(
                    ir::Op::Tex { .. }
                    | ir::Op::TexSize { .. }
                    | ir::Op::Discard
                    | ir::Op::LoadInput { .. }
                    | ir::Op::LoadUniformIdx { .. },
                ) => false,
                Some(_) => true,
                None => false,
            },
            _ => false,
        })
    }

    fn arm_has_non_if_flow(&self, r: ir::Range) -> bool {
        self.m.nodes_of(r).iter().any(|n| match n {
            ir::Node::Instr(_) => false,
            ir::Node::If { then_, else_, .. } => self.arm_has_non_if_flow(*then_) || self.arm_has_non_if_flow(*else_),
            ir::Node::Loop { body } => self.arm_has_non_if_flow(*body),
            _ => true,
        })
    }

    /// `bcsel(c1, a, bcsel(c2, a, b))` as `bcsel(c1 | c2, a, b)`: a select
    /// whose other arm is a select of the same value, as an unrolled loop's
    /// guards nest them (a value carried past a `break` is kept when the loop
    /// had already left OR leaves now). `bcsel` tests its condition against
    /// zero (`tst z` on u32), so the or of two conditions is the condition
    /// either one is; the or of one pair of flags serves every value the
    /// pair guards, and the inner select, read by nothing else, is dead.
    ///
    /// Every operand that moves from the inner select to the outer one is a
    /// virtual register written once or an immediate, so it reads the same
    /// value at the outer select as at the inner.
    fn merge_nested_selects(&mut self) -> Result<(), SelectError> {
        let n = self.next_vreg as usize;
        /* Every table here is given back at the end (they are sized by all
         * the registers and the whole program: 650 KB of a 2 MiB arena on
         * dEQP's tmp_array shapes when they were kept). */
        let mut defs_v = filled_vec(self.arena, n, 0u8)?;
        let mut def_at_v = filled_vec(self.arena, n, u32::MAX)?;
        let mut uses_v = filled_vec(self.arena, n, 0u8)?;
        let (defs, def_at, uses) = (defs_v.as_mut_slice(), def_at_v.as_mut_slice(), uses_v.as_mut_slice());
        let bcsel = SelOp::Alu(ir::AluOp::Bcsel);
        for (i, g) in self.groups.as_slice().iter().enumerate() {
            if let Some(SelDst::Virtual(v)) = g.dst {
                let width = if matches!(g.op, SelOp::Sample { .. }) { 4 } else { 1 };
                for r in v.0..v.0 + width {
                    if let (Some(d), Some(at)) = (defs.get_mut(r as usize), def_at.get_mut(r as usize)) {
                        *d = d.saturating_add(1);
                        *at = i as u32;
                    }
                }
            }
        }
        let reads = self.groups.as_slice().iter().flat_map(|g| g.srcs.iter().flatten());
        let roots = self.outputs.as_slice().iter().flat_map(|(_, c)| c.iter().flatten()).chain(self.discard.iter());
        for s in reads.chain(roots) {
            if let SelSrc::Virtual { v, .. } = s {
                if let Some(u) = uses.get_mut(v.0 as usize) {
                    *u = u.saturating_add(1);
                }
            }
        }
        let defs: &[u8] = defs;
        let groups = self.groups.as_slice();
        /* An operand read at group `to` reads what it read at `from` when no
         * group between them writes it: always for a register written once,
         * and for a loop's carried register (written before the loop and at
         * the end of its body) between two points of one pass of the body. */
        let unchanged = |s: SelSrc, from: usize, to: usize| match s {
            SelSrc::Virtual { v, mods } => {
                mods == Mods::default()
                    && groups.get(from + 1..to.max(from + 1)).is_some_and(|between| {
                        !between.iter().any(|x| match x.dst {
                            Some(SelDst::Virtual(d)) => {
                                let width = if matches!(x.op, SelOp::Sample { .. }) { 4 } else { 1 };
                                (d.0..d.0 + width).contains(&v.0)
                            }
                            _ => false,
                        })
                    })
            }
            SelSrc::Imm(_) => true,
            SelSrc::Fixed { .. } => false,
        };
        /* Sized once: a merge adds at most one `or` per select, and an AVec
         * grown by doubling leaves every smaller buffer behind in the arena. */
        let selects = groups.iter().filter(|g| g.op == bcsel).count();
        if selects < 2 {
            uses_v.release(self.arena);
            def_at_v.release(self.arena);
            defs_v.release(self.arena);
            return Ok(());
        }
        let mut out: AVec<'a, SelGroup> = AVec::with_capacity(self.arena, groups.len() + selects)?;
        /* Each or made: its two conditions, its register and where it was
         * made. */
        let mut ors: AVec<'a, (SelSrc, SelSrc, VReg, usize)> = AVec::new();
        let mut merged = 0usize;
        /* Where each group of the program landed in `out`: the inner select
         * is read as already rewritten, so a chain of three merges into one. */
        let mut placed_v = filled_vec(self.arena, groups.len(), usize::MAX)?;
        let placed = placed_v.as_mut_slice();
        for (i, g) in groups.iter().enumerate() {
            let mut g = *g;
            if let (true, [Some(c1), Some(a), Some(SelSrc::Virtual { v, mods })]) = (g.op == bcsel, g.srcs) {
                let j = def_at.get(v.0 as usize).map_or(usize::MAX, |j| *j as usize);
                let inner = placed.get(j).and_then(|k| out.as_slice().get(*k)).copied();
                let inner = inner.as_ref();
                let only = defs.get(v.0 as usize) == Some(&1) && uses.get(v.0 as usize) == Some(&1);
                if let Some(&SelGroup { op, ty, srcs: [Some(c2), Some(a2), Some(b)], .. }) = inner {
                    /* c2, a and b move from the inner select to this one; c1
                     * is read here either way. */
                    if only
                        && mods == Mods::default()
                        && op == bcsel
                        && ty == g.ty
                        && a2 == a
                        && j < i
                        && unchanged(c1, i, i)
                        && [c2, a, b].into_iter().all(|x| unchanged(x, j, i))
                    {
                        let found = ors
                            .as_slice()
                            .iter()
                            .find(|(x, y, _, at)| {
                                *x == c1 && *y == c2 && unchanged(c1, *at, i) && unchanged(c2, *at, i)
                            })
                            .map(|o| o.2);
                        let c = match found {
                            Some(c) => c,
                            None => {
                                let c = VReg(self.next_vreg);
                                self.next_vreg += 1;
                                out.push(
                                    self.arena,
                                    SelGroup {
                                        op: SelOp::Alu(ir::AluOp::Ior),
                                        ty: OpType::U32,
                                        dst: Some(SelDst::Virtual(c)),
                                        srcs: [Some(c1), Some(c2), None],
                                    },
                                )?;
                                ors.push(self.arena, (c1, c2, c, i))?;
                                c
                            }
                        };
                        g.srcs = [Some(SelSrc::Virtual { v: c, mods: Mods::default() }), Some(a), Some(b)];
                        merged += 1;
                    }
                }
            }
            placed[i] = out.len();
            out.push(self.arena, g)?;
        }
        ors.release(self.arena);
        placed_v.release(self.arena);
        uses_v.release(self.arena);
        def_at_v.release(self.arena);
        defs_v.release(self.arena);
        /* The list that is not kept goes back too: the old one when
         * anything merged, the copy otherwise. */
        if merged > 0 {
            core::mem::replace(&mut self.groups, out).release(self.arena);
        } else {
            out.release(self.arena);
        }
        Ok(())
    }

    /// Operations on constants are computed when compiling, exactly as the
    /// IR defines them (eval.rs), as Mesa folds them in NIR
    /// (nir_opt_constant_folding) and as a GLSL `const` initializer is
    /// evaluated. On the USC they ran: `sqrt` is `frcp(frsq(x))` (pco's
    /// lowering, `prepare.rs`), two roundings even on an exact model, so
    /// `sqrt(12.25)` was 3.4999998 and dEQP random all_features.fragment.53's
    /// `const int n = ivec2(exp(o), sqrt(float(12.25)) + (-1.5)).g` truncated
    /// to 1, not 2: the branch dEQP's reference takes was skipped (AM62P,
    /// the same case and cause the V3D fixed in `fold::fold_float`,
    /// which this uses). The hardware's `frcp`/`frsq`/`fexp`/`flog` and the
    /// `fred`/`fsinc` chain are not exact either, and constants built from
    /// them feed integer conversions and equality tests in these shaders
    /// (vertex.39: `sin(0.0) == 0.0`, `int(sqrt(9.0))`,
    /// `int(inversesqrt(exp(0.0)))`).
    ///
    /// A register is a constant when exactly one group defines it and that
    /// group's operands are constants at that point, in program order: a
    /// value written once is the same wherever it is read after its
    /// definition (a read before it, at a loop's head, is left alone), and a
    /// local's home written at several joins is never one. Only what
    /// `fold_float` (and the integer and boolean rules here) compute exactly
    /// is folded; anything else, a NaN, an infinity or a denormal included,
    /// stays on the machine.
    ///
    /// Only a value an approximate unit would compute -- that unit's own
    /// result, or anything computed from one -- is REPLACED by a move of the
    /// constant; its readers keep reading the register. Constant arithmetic
    /// the machine computes exactly anyway (IEEE add and multiply, integer
    /// operations, conversions of exact values) is tracked but left as it
    /// was: replacing it too made many unrelated expressions the same
    /// immediate move, which `prepare::share_common` then shares from its
    /// first definition to its last read, and dEQP light_amount's 14omni and
    /// the pre-run's two largest shaders ran out of temporaries (253 and 289
    /// needed).
    fn fold_constants(&mut self) -> Result<(), SelectError> {
        use ir::AluOp::*;
        let n = self.next_vreg as usize;
        let mut defs_v = filled_vec(self.arena, n, 0u8)?;
        let mut def_at_v = filled_vec(self.arena, n, u32::MAX)?;
        let mut known_v = filled_vec(self.arena, n, None::<u32>)?;
        /* Computed, in part, by an approximate unit. */
        let mut inexact_v = filled_vec(self.arena, n, false)?;
        let (defs, def_at, known) = (defs_v.as_mut_slice(), def_at_v.as_mut_slice(), known_v.as_mut_slice());
        let inexact = inexact_v.as_mut_slice();
        for (i, g) in self.groups.as_slice().iter().enumerate() {
            if let Some(SelDst::Virtual(v)) = g.dst {
                let width = if matches!(g.op, SelOp::Sample { .. }) { 4 } else { 1 };
                for r in v.0..v.0 + width {
                    if let (Some(d), Some(at)) = (defs.get_mut(r as usize), def_at.get_mut(r as usize)) {
                        *d = d.saturating_add(1);
                        *at = i as u32;
                    }
                }
            }
        }
        /* A declared input is written before the first group. */
        for v in self.inputs.as_slice().iter().flat_map(|(_, lanes)| lanes.iter().flatten()) {
            if let Some(d) = defs.get_mut(v.0 as usize) {
                *d = d.saturating_add(1);
            }
        }
        let b = |c: bool| if c { u32::MAX } else { 0 };
        let groups = self.groups.as_mut_slice();
        for i in 0..groups.len() {
            let g = groups[i];
            let Some(SelDst::Virtual(d)) = g.dst else { continue };
            if defs.get(d.0 as usize) != Some(&1) {
                continue;
            }
            let val = |s: Option<SelSrc>| -> Option<u32> {
                match s? {
                    SelSrc::Imm(k) => Some(k),
                    SelSrc::Virtual { v, mods } => {
                        known.get(v.0 as usize).copied().flatten().map(|k| fold_mods(k, mods))
                    }
                    SelSrc::Fixed { .. } => None,
                }
            };
            let folded = match g.op {
                SelOp::Copy => val(g.srcs[0]),
                SelOp::Alu(Bcsel) => val(g.srcs[0]).and_then(|c| val(if c != 0 { g.srcs[1] } else { g.srcs[2] })),
                SelOp::Alu(o) => (|| {
                    let x = val(g.srcs[0])?;
                    let y = if o.arity() >= 2 { val(g.srcs[1])? } else { 0 };
                    let z = if o.arity() >= 3 { val(g.srcs[2])? } else { 0 };
                    if let Some(r) = crate::shaderc::online::fold::fold_float(o, x, y, z) {
                        return Some(r);
                    }
                    let (xi, yi) = (x as i32, y as i32);
                    Some(match o {
                        Iadd => x.wrapping_add(y),
                        Imul => x.wrapping_mul(y),
                        Ineg => x.wrapping_neg(),
                        Iand => x & y,
                        Ior => x | y,
                        Ixor => x ^ y,
                        Ilt => b(xi < yi),
                        Ile => b(xi <= yi),
                        Ieq => b(x == y),
                        Ine => b(x != y),
                        I2b => b(x != 0),
                        B2i => u32::from(x != 0),
                        Bnot => !x,
                        _ => return None,
                    })
                })(),
                /* sin(0) and cos(0), the only exact ones (`fold_float`):
                 * the chain's `fred` part B holds x. */
                SelOp::Machine(MachineOp::PselTrig) => (|| {
                    let SelSrc::Virtual { v: pb, mods } = g.srcs[1]? else { return None };
                    let at = *def_at.get(pb.0 as usize)? as usize;
                    let SelOp::Machine(MachineOp::FredB(t)) = groups.get(at)?.op else { return None };
                    if mods != Mods::default() || defs.get(pb.0 as usize) != Some(&1) {
                        return None;
                    }
                    let x = val(groups[at].srcs[0])?;
                    let o = if t == Trig::Sin { Fsin } else { Fcos };
                    crate::shaderc::online::fold::fold_float(o, x, 0, 0)
                })(),
                _ => None,
            };
            /* `x * 1.0` is x on every input (-0.0 and NaN included): the
             * group becomes a copy of x, which `forward_copies` then lets
             * its readers read through. NIR folds `fmul(a, 1.0)` the same
             * way; a matrix times a vector with a 1.0 component made one a
             * lane (fill_color's `scissorMat * vec3(p, 1.0)`, gl_test 186). */
            if folded.is_none() && g.op == SelOp::Alu(Fmul) && g.ty == OpType::F32 {
                let one = 1.0f32.to_bits();
                let other = if val(g.srcs[1]) == Some(one) {
                    g.srcs[0]
                } else if val(g.srcs[0]) == Some(one) {
                    g.srcs[1]
                } else {
                    None
                };
                if let Some(x) = other {
                    groups[i] = SelGroup { op: SelOp::Copy, ty: OpType::F32, dst: g.dst, srcs: [Some(x), None, None] };
                    continue;
                }
            }
            let Some(k) = folded else { continue };
            known[d.0 as usize] = Some(k);
            let unit =
                matches!(g.op, SelOp::Alu(Frcp | Frsq | Fsqrt | Fexp2 | Flog2) | SelOp::Machine(MachineOp::PselTrig));
            let from_unit = g.srcs.iter().flatten().any(|s| match s {
                SelSrc::Virtual { v, .. } => inexact.get(v.0 as usize).copied().unwrap_or(false),
                _ => false,
            });
            if unit || from_unit {
                inexact[d.0 as usize] = true;
                groups[i] =
                    SelGroup { op: SelOp::Copy, ty: OpType::F32, dst: g.dst, srcs: [Some(SelSrc::Imm(k)), None, None] };
            }
        }
        inexact_v.release(self.arena);
        known_v.release(self.arena);
        def_at_v.release(self.arena);
        defs_v.release(self.arena);
        Ok(())
    }

    /// Readers of a copy read its source instead, where the source is a
    /// value nothing rewrites: a virtual register written once (or a
    /// declared input), a uniform, a special register or vertex input, or
    /// an immediate one of the constant registers holds. A uniform or
    /// another immediate is forwarded only to a copy's single reader:
    /// `prepare` moves an operand a form cannot take, one move per reader,
    /// so a copy read twice is kept as the one move. A reader's modifiers
    /// and the copy's compose when one of the two sets is empty; otherwise
    /// the reader keeps reading the copy. The copy itself stays and
    /// `eliminate_dead` drops it once nothing reads it.
    ///
    /// fill_color's `mat3(frag[0].xyz, ...)` and `vec3(p, 1.0)` went
    /// through local copies, and `x * 1.0` (`fold_constants`) is one: the
    /// table's program has none of those moves (gl_test 186).
    fn forward_copies(&mut self) -> Result<(), SelectError> {
        let n = self.next_vreg as usize;
        let mut defs_v = filled_vec(self.arena, n, 0u8)?;
        let mut reads_v = filled_vec(self.arena, n, 0u8)?;
        let mut to_v = filled_vec(self.arena, n, None::<SelSrc>)?;
        let (defs, reads, to) = (defs_v.as_mut_slice(), reads_v.as_mut_slice(), to_v.as_mut_slice());
        let bump = |t: &mut [u8], v: VReg, by: u8| {
            if let Some(c) = t.get_mut(v.0 as usize) {
                *c = c.saturating_add(by);
            }
        };
        for g in self.groups.as_slice() {
            if let Some(SelDst::Virtual(v)) = g.dst {
                let width = if matches!(g.op, SelOp::Sample { .. }) { 4 } else { 1 };
                for k in 0..width {
                    bump(defs, VReg(v.0 + k), 1);
                }
            }
            for s in g.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = s {
                    bump(reads, *v, 1);
                }
            }
        }
        for v in self.inputs.as_slice().iter().flat_map(|(_, lanes)| lanes.iter().flatten()) {
            bump(defs, *v, 1);
        }
        /* outputs, the discard and alpha-to-one read too: never forwarded
         * into (`prepare` places what they read), so they count as two */
        let side = self.outputs.as_slice().iter().flat_map(|(_, c)| c.iter().flatten().copied());
        for s in side.chain(self.discard).chain(self.alpha_to_one.iter().flat_map(|(x, f)| [*x, *f])) {
            if let SelSrc::Virtual { v, .. } = s {
                bump(reads, v, 2);
            }
        }
        let compose = |s: SelSrc, outer: Mods| -> Option<SelSrc> {
            match s {
                SelSrc::Virtual { v, mods } if mods == Mods::default() => Some(SelSrc::Virtual { v, mods: outer }),
                SelSrc::Virtual { .. } if outer == Mods::default() => Some(s),
                SelSrc::Fixed { bank, reg, mods } if mods == Mods::default() => {
                    Some(SelSrc::Fixed { bank, reg, mods: outer })
                }
                SelSrc::Fixed { .. } if outer == Mods::default() => Some(s),
                SelSrc::Imm(k) if outer == Mods::default() => Some(SelSrc::Imm(k)),
                _ => None,
            }
        };
        for g in self.groups.as_mut_slice().iter_mut() {
            for s in g.srcs.iter_mut().flatten() {
                if let SelSrc::Virtual { v, mods } = *s {
                    if let Some(f) = to.get(v.0 as usize).copied().flatten().and_then(|f| compose(f, mods)) {
                        *s = f;
                    }
                }
            }
            if g.op != SelOp::Copy {
                continue;
            }
            let (Some(SelDst::Virtual(d)), Some(src)) = (g.dst, g.srcs[0]) else { continue };
            if defs.get(d.0 as usize) != Some(&1) {
                continue;
            }
            let one_reader = reads.get(d.0 as usize) == Some(&1);
            let stable = match src {
                SelSrc::Virtual { v, .. } => defs.get(v.0 as usize) == Some(&1),
                SelSrc::Fixed { bank: Bank::Special | Bank::VtxIn, .. } => true,
                SelSrc::Fixed { bank: Bank::Shared, .. } => one_reader,
                SelSrc::Fixed { .. } => false,
                SelSrc::Imm(k) => super::prepare::const_reg(k).is_some() || one_reader,
            };
            if stable {
                to[d.0 as usize] = Some(src);
            }
        }
        to_v.release(self.arena);
        reads_v.release(self.arena);
        defs_v.release(self.arena);
        Ok(())
    }

    /// How many components this instruction's result has.
    ///
    /// The front end records it (es100-frontend, `Module::comps_of`, 1a2c75c)
    /// so this no longer estimates. Exact on the component-wise paths and on
    /// the reductions, where the rule lives on the opcode; 4 where nothing
    /// was recorded, which is the old over-estimate and is never a narrow
    /// answer that would mask a lane away.
    /// Delete groups nothing reads.
    ///
    /// This machine is scalar and selection gives every value four registers,
    /// one per component, whether the shader uses four or one. A `vec4`
    /// expression whose `.x` alone is read therefore costs four groups and
    /// three of them compute a number no one asks for. The GC7000 side, a
    /// vector machine, has no such arithmetic: there one instruction covers
    /// the four lanes and a narrower write mask is the whole of the saving.
    ///
    /// So the same pass is worth a different amount on each machine, and on
    /// this one it is structural rather than incidental.
    ///
    /// What may go is narrow on purpose:
    ///
    /// * only a write to a VIRTUAL register. A `SelDst::Fixed` is a register
    ///   the generator chose for a reason this file cannot see.
    /// * **never a register an OUTPUT names.** `Selected::outputs` carries a
    ///   `SelSrc` per component that the BUILDER reads -- no group reads it --
    ///   so a liveness that counted only group operands would delete every
    ///   value the shader actually produces and leave a program that computes
    ///   nothing. That is the one way this pass can be catastrophically
    ///   wrong, and the test named after it is the guard.
    /// * a group that reads its own destination (a dot product accumulates
    ///   into it) is not a reader of itself: `j != i`.
    ///
    /// Iterated to a fixed point, because deleting one group can make the
    /// group that produced its operand dead in turn -- which is exactly what
    /// a dot product's chain does. Bounded by the group count so a bug
    /// cannot hang the compiler.
    fn eliminate_dead(&mut self) -> Result<(), SelectError> {
        let n = self.groups.len();
        if n == 0 {
            return Ok(());
        }
        /* One backward pass over straight-line code (control flow is
         * converted or refused): a register is live between a read and the
         * definition before it. A group whose result nothing reads before
         * the next definition of the same register is dead.
         *
         * This used to ask "does ANY group read this register", which is
         * exact for a value written once and not for a local's home
         * register, written at every `if` that joins it: a read BEFORE a
         * definition kept it alive, so the joins of lanes 1..3 of every
         * scalar local fed each other to the end and SaturationLightness.frag
         * needed 270 temporaries. */
        let live = self.arena.alloc_filled(self.next_vreg as usize, false)?;
        let dead = self.arena.alloc_filled(n, false)?;
        fn mark(live: &mut [bool], v: VReg, on: bool) {
            if let Some(x) = live.get_mut(v.0 as usize) {
                *x = on;
            }
        }
        /* The outputs, the discard flag and alpha-to-one's pair (a blend
         * variant blends that alpha) are read by the builder, after the last
         * group. */
        let a2one = self.alpha_to_one.iter().flat_map(|(x, f)| [*x, *f]);
        let side = self.outputs.as_slice().iter().flat_map(|(_, comps)| comps.iter().flatten().copied());
        for s in side.chain(self.discard).chain(a2one) {
            if let SelSrc::Virtual { v, .. } = s {
                mark(live, v, true);
            }
        }
        for i in (0..n).rev() {
            let g = self.groups.as_slice()[i];
            /* The end of a loop: the next iteration reads whatever the body
             * reads, so all of it is live here, whatever comes after. */
            if g.op == SelOp::LoopEnd {
                let body = self.groups.as_slice()[..i].iter().rev().take_while(|x| x.op != SelOp::LoopBegin);
                for s in body.flat_map(|x| x.srcs.iter().flatten()) {
                    if let SelSrc::Virtual { v, .. } = s {
                        mark(live, *v, true);
                    }
                }
            }
            /* A cube sample's third coordinate is an OPERAND of the cube
             * sample right after it, carried in a group of its own only
             * because a group has three sources (`SelOp::SampleZ`). It has
             * no destination, so the rule below would keep it as an effect,
             * and with it the z it reads, after the sample it belongs to
             * was deleted: the chain then found no sample to fold it into
             * and the assembler refused the program (dEQP
             * shaders.random.*, a textureCube whose result nothing reads,
             * native cube on; am62p 2026-09-25). It lives and dies with its
             * sample, as a NIR tex's vec3 coordinate does. */
            if g.op == SelOp::SampleZ {
                let next = self.groups.as_slice().get(i + 1);
                if !matches!(next.map(|x| x.op), Some(SelOp::Sample { cube: true, .. })) {
                    return Err(SelectError::Unsupported("a cube coordinate with no cube sample after it"));
                }
                if dead[i + 1] {
                    dead[i] = true;
                    continue;
                }
            }
            /* A sample's gradients likewise (`SelOp::SampleGrad`, before the
             * cube's z when there is one): they die with the sample. */
            if g.op == SelOp::SampleGrad {
                let next = self.groups.as_slice().get(i + 1);
                if !matches!(
                    next.map(|x| x.op),
                    Some(SelOp::SampleGrad | SelOp::SampleZ | SelOp::Sample { lod: SmpLod::Gradients, .. })
                ) {
                    return Err(SelectError::Unsupported("gradients with no gradient sample after them"));
                }
                if dead[i + 1] {
                    dead[i] = true;
                    continue;
                }
            }
            if let Some(SelDst::Virtual(v0)) = g.dst {
                /* A sample defines FOUR registers from its destination
                 * (`SelOp::Sample`); a shader reading only `.w` of a fetch
                 * reads none of lane 0, and judging the group by lane 0
                 * deleted the fetch and left `.w` read from nothing
                 * (8bittextmask_a.frag, caught by the survey). */
                let width = if matches!(g.op, SelOp::Sample { .. }) { 4 } else { 1 };
                let regs = v0.0..v0.0 + width;
                let used = regs.clone().any(|r| live.get(r as usize).copied().unwrap_or(false));
                if !used {
                    dead[i] = true;
                    continue;
                }
                for r in regs {
                    mark(live, VReg(r), false);
                }
            }
            /* A group with no result or a fixed one (an index register, an
             * output) is an effect and stays. */
            for s in g.srcs.iter().flatten() {
                if let SelSrc::Virtual { v, .. } = s {
                    mark(live, *v, true);
                }
            }
        }
        /* What is still live at the program's start is read before any
         * group writes it: only a declared input may be. Anything else is a
         * register nothing writes, and the program would read whatever it
         * held -- a lane of a one-lane value a vector read without a splat
         * (faceforward's condition, dEQP constant_expressions.faceforward_
         * vec2..4). Refused, where it used to draw a wrong value.
         *
         * Only without a kept loop: a loop's end marks every read in its
         * body live, dead groups' included, so there the pass over-reports
         * (51 of the Qt lit materials, all refused by allocation anyway) and
         * this check would refuse sound programs. A lane read past its value
         * inside a loop is not caught. */
        let straight = !self.groups.as_slice().iter().any(|g| g.op == SelOp::LoopBegin);
        for (r, _) in live.iter().enumerate().filter(|(_, l)| straight && **l) {
            let declared = self.inputs.as_slice().iter().any(|(_, lanes)| lanes.contains(&Some(VReg(r as u32))));
            if !declared {
                return Err(SelectError::Unsupported("a register read that nothing writes"));
            }
        }
        let gone = dead.iter().filter(|d| **d).count();
        if gone == 0 {
            return Ok(());
        }
        /* Compact in place. There are no branch targets to remap here --
         * control flow is refused in this back end -- so an index into the
         * group list is nobody's fixed reference and dropping the tail is
         * the whole of it. */
        let slice = self.groups.as_mut_slice();
        let mut w = 0;
        for r in 0..n {
            if !dead[r] {
                slice[w] = slice[r];
                w += 1;
            }
        }
        while self.groups.len() > w {
            self.groups.pop();
        }
        Ok(())
    }

    fn result_lanes(&self, id: ir::InstrId) -> u32 {
        u32::from(self.m.comps_of(ir::ValueId(id.0))).clamp(1, LANES)
    }
}

/// A source read through an outer swizzle: lane *i* of the result takes the
/// inner source's `swizzle[outer[i]]`.
fn swizzle_through(inner: ir::Src, outer: [u8; 4]) -> ir::Src {
    let mut sw = [0u8; 4];
    for (i, o) in outer.iter().enumerate() {
        sw[i] = inner.swizzle[*o as usize % 4];
    }
    ir::Src { val: inner.val, swizzle: sw }
}

/// How many terms a dot product sums.
fn terms(o: ir::AluOp) -> u32 {
    match o {
        ir::AluOp::Fdot2 => 2,
        ir::AluOp::Fdot3 => 3,
        _ => 4,
    }
}

/// The `tst_type` an operation's operands are read as.
///
/// Float unless the operation is about integers or booleans. The USC spells
/// this in three bits with seven values and NOT the GC7000's eight, which is
/// why the two machines' `OpType` are two types.
fn op_type(o: ir::AluOp) -> OpType {
    use ir::AluOp::*;
    match o {
        Iadd | Imul | Idiv | Imod | Ineg | Ishl | Ishr | Ilt | Ile | Ieq | Ine | F2i | Imin | Imax => OpType::S32,
        Iand | Ior | Ixor | Bnot | Bany | Ball | B2f | B2i | Bcsel | I2b => OpType::U32,
        Udiv | Umod | Ushr | Ult | Ule | Umin | Umax | Inot | F2u => OpType::U32,
        _ => OpType::F32,
    }
}

/// Refuse a program whose back edge could be driven by anything but its
/// loop's own counter.
///
/// The builder emits `br if(p0)` for a [`SelOp::LoopEnd`] and nothing else,
/// and a plain branch is only sound when every instance takes it or none
/// does: pco's `cnd*` execution-mask machinery exists for the divergent
/// case, and this path does not use it. So p0 at the back edge must come
/// from a value the same in every instance -- the loop's counter -- and this
/// checks the exact shape [`Sel::loop_`] emits, so that a later change that
/// breaks it fails here, by name, before a board sees it:
///
/// ```text
///   c = 0                   before the LoopBegin; c written nowhere else
///   LoopBegin               (no second one before the end)
///   ...
///   c = c + 1               Iadd(c, Imm 1)
///   b = (N <= c)            Ile(Imm N, c), N a constant; b written once
///   LoopTest b
///   LoopEnd
/// ```
fn verify_loops(groups: &[SelGroup]) -> Result<(), SelectError> {
    const WHY: &str = "a back edge not driven by its loop's counter alone";
    let bad = || SelectError::Unsupported(WHY);
    let plain = |s: Option<SelSrc>| match s {
        Some(SelSrc::Virtual { v, mods }) if mods == Mods::default() => Some(v),
        _ => None,
    };
    let writes = |v: VReg| groups.iter().filter(|g| g.dst == Some(SelDst::Virtual(v))).count();
    /* The open loops' tops, innermost last: a kept loop may sit inside
     * another (as many as the assembler's and the allocator's stacks). */
    let mut open: [usize; 4] = [0; 4];
    let mut depth = 0usize;
    for (i, g) in groups.iter().enumerate() {
        match g.op {
            SelOp::LoopBegin => {
                *open.get_mut(depth).ok_or(SelectError::Unsupported("loops kept as loops nested too deeply"))? = i;
                depth += 1;
            }
            SelOp::LoopEnd => {
                depth = depth.checked_sub(1).ok_or_else(bad)?;
                let top = open[depth];
                /* Under the execution mask the test is followed by the
                 * instances' leaving and the loop's end (`cndsm`, `cndlt`),
                 * both on the one emc, which then drives the back edge. */
                let mut end = i;
                if let [.., leave, lt] = groups[..i] {
                    let masked = leave.op == SelOp::Machine(MachineOp::CndLeave)
                        && lt.op == SelOp::Machine(MachineOp::CndLoopEnd);
                    if masked {
                        let emc = plain(leave.srcs[0]).ok_or_else(bad)?;
                        let on_emc =
                            |x: &SelGroup| plain(x.srcs[0]) == Some(emc) && x.dst == Some(SelDst::Virtual(emc));
                        if !on_emc(&leave) || !on_emc(&lt) {
                            return Err(bad());
                        }
                        end = i - 2;
                    }
                }
                let [.., before_cmp, cmp_or_inc, or_or_cmp, test] = groups[..end] else { return Err(bad()) };
                let b = plain(test.srcs[0]).filter(|_| test.op == SelOp::Machine(MachineOp::LoopTest));
                let b = b.ok_or_else(bad)?;
                /* Two shapes: the counter's compare alone, or (a loop counted
                 * at run time, unroll::UNIFORM_LOOP_CAP) that compare or-ed
                 * with the loop's break flag -- a flag whose every write the
                 * unroller traced to uniform values (`uniform_exits`), so
                 * the or is the same in every instance too. */
                let (inc, cmp) = if or_or_cmp.op == SelOp::Alu(ir::AluOp::Ior) {
                    let e = or_or_cmp.dst == Some(SelDst::Virtual(b)) && writes(b) == 1;
                    let from_cmp =
                        plain(or_or_cmp.srcs[0]).zip(cmp_or_inc.dst).is_some_and(|(x, d)| d == SelDst::Virtual(x));
                    if !e || !from_cmp || plain(or_or_cmp.srcs[1]).is_none() {
                        return Err(bad());
                    }
                    (before_cmp, cmp_or_inc)
                } else {
                    (cmp_or_inc, or_or_cmp)
                };
                let Some(SelDst::Virtual(bc)) = cmp.dst else { return Err(bad()) };
                if cmp.op != SelOp::Alu(ir::AluOp::Ile) || writes(bc) != 1 {
                    return Err(bad());
                }
                if or_or_cmp.op != SelOp::Alu(ir::AluOp::Ior) && bc != b {
                    return Err(bad());
                }
                let (Some(SelSrc::Imm(_)), Some(c)) = (cmp.srcs[0], plain(cmp.srcs[1])) else { return Err(bad()) };
                let counts = inc.op == SelOp::Alu(ir::AluOp::Iadd)
                    && inc.dst == Some(SelDst::Virtual(c))
                    && plain(inc.srcs[0]) == Some(c)
                    && inc.srcs[1] == Some(SelSrc::Imm(1));
                /* Its only other write: zero, before the loop. */
                let init = groups[..top].iter().rposition(|x| x.dst == Some(SelDst::Virtual(c)));
                let zeroed =
                    init.is_some_and(|k| groups[k].op == SelOp::Copy && groups[k].srcs[0] == Some(SelSrc::Imm(0)));
                if !counts || !zeroed || writes(c) != 2 {
                    return Err(bad());
                }
            }
            /* A test sets p0 for the back edge, the masked leave, or a
             * predicated `if`'s arms (`if_convert`); the loop's own test
             * right before its back edge sets p0 again after any `if`. */
            SelOp::Machine(MachineOp::LoopTest)
                if !matches!(
                    groups.get(i + 1).map(|x| x.op),
                    Some(SelOp::LoopEnd | SelOp::Machine(MachineOp::CndLeave) | SelOp::Pred(PredArm::Then))
                ) =>
            {
                return Err(bad());
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(bad());
    }
    Ok(())
}

/// Select USC groups for `m`.
/// `n` copies of `v`, in a vector the caller can give back to the arena with
/// [`AVec::release`] (a slice from `alloc_filled` cannot be).
fn filled_vec<'a, T: Copy>(a: &Arena<'a>, n: usize, v: T) -> Result<AVec<'a, T>, ArenaError> {
    let mut x = AVec::with_capacity(a, n)?;
    for _ in 0..n {
        x.push(a, v)?;
    }
    Ok(x)
}

pub fn select<'a>(m: &'a ir::Module<'a>, arena: &'a Arena<'a>) -> Result<Selected<'a>, SelectError> {
    if m.stage != SHC_STAGE_VERTEX && m.stage != SHC_STAGE_FRAGMENT {
        return Err(SelectError::Unsupported("a stage that is neither vertex nor fragment"));
    }
    /* Loops first: expanded into their iterations (`unroll.rs`), so
     * nothing below meets one. */
    let (m, trips) = match super::unroll::unroll(m, arena) {
        Ok(Some(u)) => (u.module, u.trips),
        Ok(None) => (m, &[][..]),
        Err(e) => return Err(SelectError::Unsupported(e.message())),
    };
    let locals_in = arena.alloc_filled(Sel::slots(m) as usize * LANES as usize, VReg(0))?;
    for (i, e) in locals_in.iter_mut().enumerate() {
        *e = Sel::local_home(m, i as u32 / LANES, i as u32 % LANES);
    }
    /* After the values, the slots and the varyings: scratch this file invents
     * for an `if`'s arms starts where those end. */
    let next_vreg = (m.instrs.len() as u32 + Sel::slots(m) + m.inputs.len() as u32) * LANES;
    let loaded = arena.alloc_filled(m.instrs.len() * LANES as usize, UNLOADED)?;
    let mut s = Sel {
        m,
        attribs_read: super::abi::attribs_placed(m).unwrap_or(0),
        arena,
        locals_in,
        loaded,
        next_vreg,
        tex_dst: None,
        groups: AVec::new(),
        inputs: AVec::new(),
        outputs: AVec::new(),
        discard: None,
        alpha_to_one: None,
        trips,
        frag_coord: [None; 2],
        point_coord: [None; 2],
        emc: None,
        front_facing: None,
        frag_zw: [None; 2],
        loop_depth: 0,
        grad_w: None,
        grad_cache: [None; 4],
    };
    s.declare_inputs()?;
    s.frag_coord_prologue()?;
    s.front_face_prologue()?;
    s.nodes(m.body)?;
    s.discard_epilogue()?;
    s.zero_unwritten_locals()?;
    s.fold_constants()?;
    s.forward_copies()?;
    s.merge_nested_selects()?;
    s.eliminate_dead()?;
    s.prune_frag_zw();
    verify_loops(s.groups.as_slice())?;
    /* Unchanged by the deletion: `vregs` is the bound the allocator sizes its
     * map from, not a count of what survives, and a register that no longer
     * has a definition simply never appears. */
    let vregs = s.next_vreg;
    Ok(Selected {
        groups: s.groups.into_slice(),
        vregs,
        inputs: s.inputs.into_slice(),
        outputs: s.outputs.into_slice(),
        discard: s.discard,
        alpha_to_one: s.alpha_to_one,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::store;

    /// A loop that leaves on data that may differ between instances (a
    /// varying) is one body between a `LoopBegin` and a back edge driven by
    /// its counter alone, and `verify_loops` refuses every way the back edge
    /// could come to depend on anything else. With a uniform bound instead,
    /// the back edge also reads the loop's break flag (unroll.rs: a counted
    /// loop leaves early on a uniform break), `leave = (7 <= c) | flag`, and
    /// `verify_loops` takes that.
    /// The groups `src`'s fragment shader selects to.
    fn selected_fs(src: &str) -> std::vec::Vec<SelGroup> {
        use crate::shaderc::online::{lower, sema::Stage};
        let _l = store::test_lock();
        let mut out = std::vec::Vec::new();
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            out = select(&l.module, a).expect("selects").groups.to_vec();
        })
        .unwrap();
        out
    }

    /// `x * 1.0` is x: a matrix times a vector with a 1.0 lane keeps the
    /// products of the other lanes and reads that column as it is
    /// (fill_color's `scissorMat * vec3(p, 1.0)`; gl_test 189), and a
    /// negated column lane reaches its reader as the modifier. A lane that
    /// is not 1.0 keeps its product.
    #[test]
    fn a_product_by_one_is_read_as_its_other_operand() {
        let fs = |v: &str| {
            selected_fs(&format!(
                "precision highp float; uniform vec3 a, b, c; void main() {{ \
                 gl_FragColor = vec4(mat3(a, b, -c) * vec3({v}), 1.0); }}"
            ))
        };
        let count = |g: &[SelGroup], op: ir::AluOp| g.iter().filter(|x| x.op == SelOp::Alu(op)).count();
        let (with_one, without) = (fs("0.5, 0.25, 1.0"), fs("0.5, 0.25, 2.0"));
        /* 2.0: a product per row, then two fused products on it */
        assert_eq!((count(&without, ir::AluOp::Fmul), count(&without, ir::AluOp::Ffma)), (3, 6), "{without:?}");
        /* 1.0: the column is the first fused product's addend, as pco's */
        assert_eq!((count(&with_one, ir::AluOp::Fmul), count(&with_one, ir::AluOp::Ffma)), (0, 6), "{with_one:?}");
        let negated_addend = |x: &&SelGroup| {
            x.op == SelOp::Alu(ir::AluOp::Ffma)
                && matches!(x.srcs[2], Some(SelSrc::Fixed { bank: Bank::Shared, mods, .. }) if mods.neg)
        };
        assert_eq!(with_one.iter().filter(negated_addend).count(), 3, "-c read as the addends: {with_one:?}");
    }

    /// What the builder reads after the last group -- the outputs, the
    /// discard flag and alpha-to-one's alpha and flag -- keeps its
    /// definition. With a constant colour, alpha-to-one's alpha is the copy
    /// of 1.0 whose group readers `forward_copies` sends to the immediate:
    /// nothing but the builder reads it then, and stencil.frag's blend
    /// variants blended an undefined alpha (`tests/usc_limits.rs`, 14 of
    /// them against pco) until `eliminate_dead` counted the pair.
    #[test]
    fn the_values_the_builder_reads_keep_their_definitions() {
        use crate::shaderc::online::{lower, sema::Stage};
        /* through a local, as stencil.frag's `result`: the local's copy */
        let src = "precision mediump float; uniform float u; void main() { \
                   vec4 r; if (u < 0.5) discard; r = vec4(1.0); gl_FragColor = r; }";
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let s = select(&l.module, a).expect("selects");
            let defined = |x: SelSrc| match x {
                SelSrc::Virtual { v, .. } => s.groups.iter().any(|g| g.dst == Some(SelDst::Virtual(v))),
                _ => true,
            };
            let (alpha, flag) = s.alpha_to_one.expect("a shader that discards has alpha-to-one's pair");
            assert!(defined(alpha), "alpha-to-one's alpha {alpha:?} has no group: {:?}", s.groups);
            assert!(defined(flag), "alpha-to-one's flag {flag:?} has no group");
            assert!(defined(s.discard.expect("discards")), "the discard flag has no group");
            for c in s.outputs.iter().flat_map(|(_, c)| c.iter().flatten()) {
                assert!(defined(*c), "an output {c:?} has no group");
            }
        })
        .unwrap();
    }

    /// An `if` on a uniform whose arms are arithmetic runs predicated, as
    /// pco runs it (pco_cf.c lower_if_pred_exec): the condition's tstz.p0,
    /// the then arm, the else arm, the end, and only then the joins: the
    /// selects on the same condition, one per component that differs. The
    /// arms hold arithmetic only. (gl_test 186/187: the table's predicated
    /// arms were the whole of the fill_color gap.)
    #[test]
    fn a_uniform_if_with_arithmetic_arms_runs_them_predicated() {
        let g = selected_fs(
            "precision mediump float; uniform vec4 u; void main() { vec4 c; \
             if (u.x > 0.0) { c = u * 2.0; } else { c = u + 1.0; } gl_FragColor = c; }",
        );
        let at = |m: PredArm| g.iter().position(|x| x.op == SelOp::Pred(m)).expect("the marker");
        let (then_, else_, end) = (at(PredArm::Then), at(PredArm::Else), at(PredArm::End));
        assert!(then_ < else_ && else_ < end, "in order: {g:?}");
        assert_eq!(
            g[then_ - 1].op,
            SelOp::Machine(MachineOp::LoopTest),
            "the condition's test right before the then arm"
        );
        for x in g[then_ + 1..else_].iter().chain(&g[else_ + 1..end]) {
            assert!(matches!(x.op, SelOp::Alu(_) | SelOp::Copy), "only arithmetic in an arm: {x:?}");
        }
        assert!(else_ > then_ + 1 && end > else_ + 1, "both arms compute: {g:?}");
        let joins = g[end + 1..].iter().filter(|x| x.op == SelOp::Alu(ir::AluOp::Bcsel)).count();
        assert_eq!(joins, 4, "a select per component of c after the end: {g:?}");
        assert_eq!(g.iter().filter(|x| matches!(x.op, SelOp::Pred(_))).count(), 3, "one `if`");
    }

    /// Arms that read or write p0 or need every instance -- a look-up, a
    /// sine, a derivative -- are if-converted as before (no marker), and of
    /// two nested `if`s only the inner one, whose arms are arithmetic, runs
    /// predicated (pco_cf.c can_pred_exec skips nesting).
    #[test]
    fn arms_that_cannot_run_predicated_are_if_converted() {
        for (what, body) in [
            ("a look-up", "c = texture2D(t, u.xy);"),
            ("a sine", "c = vec4(sin(u.x));"),
            ("a derivative", "c = vec4(dFdx(v.x));"),
        ] {
            let g = selected_fs(&format!(
                "#extension GL_OES_standard_derivatives : enable\nprecision mediump float; uniform vec4 u; \
                 uniform sampler2D t; varying vec4 v; void main() {{ vec4 c = u; if (u.x > 0.0) {{ {body} }} \
                 gl_FragColor = c; }}"
            ));
            assert!(!g.iter().any(|x| matches!(x.op, SelOp::Pred(_))), "{what}: if-converted");
            assert!(g.iter().any(|x| x.op == SelOp::Alu(ir::AluOp::Bcsel)), "{what}: the `if` is there, as selects");
        }
        let g = selected_fs(
            "precision mediump float; uniform vec4 u; void main() { vec4 c = u; \
             if (u.x > 0.0) { c = u * 3.0; if (u.y > 0.0) { c = u * 2.0; } } gl_FragColor = c; }",
        );
        assert_eq!(g.iter().filter(|x| x.op == SelOp::Pred(PredArm::Then)).count(), 1, "the inner `if` alone: {g:?}");
    }

    #[test]
    fn the_back_edge_is_driven_by_the_loop_counter_and_nothing_else() {
        use crate::shaderc::online::{lower, sema::Stage};
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a
                .alloc_str(
                    "precision mediump float; uniform vec4 u; varying vec4 v; void main(){ float s = 0.0; \
                     for (float i = 1.0; i <= 6.0; i += 1.0) { if (i >= v.x) { break; } s += i * u.y; } \
                     gl_FragColor = vec4(s); }",
                )
                .unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let s = select(&l.module, a).expect("selects");
            let g: std::vec::Vec<SelGroup> = s.groups.to_vec();
            let count = |op: SelOp| g.iter().filter(|x| x.op == op).count();
            assert_eq!((count(SelOp::LoopBegin), count(SelOp::LoopEnd)), (1, 1), "one loop, kept");
            let end = g.iter().position(|x| x.op == SelOp::LoopEnd).unwrap();
            assert_eq!(g[end - 1].op, SelOp::Machine(MachineOp::LoopTest));
            assert_eq!(g[end - 2].srcs[0], Some(SelSrc::Imm(7)), "6 iterations and the entry that breaks");
            assert_eq!(verify_loops(&g), Ok(()));

            let refused = |g: &[SelGroup]| verify_loops(g).is_err();
            /* The test reading a data value (the loop's own break compare). */
            let data = g.iter().find(|x| x.op == SelOp::Alu(ir::AluOp::Fle)).and_then(|x| x.dst).unwrap();
            let SelDst::Virtual(dv) = data else { panic!("a fixed compare") };
            let mut bad = g.clone();
            bad[end - 1].srcs[0] = Some(SelSrc::Virtual { v: dv, mods: Mods::default() });
            assert!(refused(&bad), "a back edge on a data value");
            /* The counter written once more, inside the body. */
            let Some(SelDst::Virtual(c)) = g[end - 3].dst else { panic!("no counter") };
            let mut bad = g.clone();
            let top = g.iter().position(|x| x.op == SelOp::LoopBegin).unwrap();
            bad.insert(
                top + 1,
                SelGroup {
                    op: SelOp::Copy,
                    ty: OpType::U32,
                    dst: Some(SelDst::Virtual(c)),
                    srcs: [Some(SelSrc::Imm(5)), None, None],
                },
            );
            assert!(refused(&bad), "a counter something else writes");
            /* No test before the back edge. */
            let mut bad = g.clone();
            bad.remove(end - 1);
            assert!(refused(&bad), "a back edge with no test before it");
            /* A trip count that is not a constant. */
            let mut bad = g.clone();
            bad[end - 2].srcs[0] = Some(SelSrc::Virtual { v: dv, mods: Mods::default() });
            assert!(refused(&bad), "a bound read from a register");

            /* the uniform bound: the counter's test or'd with the flag */
            let text = a
                .alloc_str(
                    "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
                     for (float i = 1.0; i <= 6.0; i += 1.0) { if (i >= u.x) { break; } s += i * u.y; } \
                     gl_FragColor = vec4(s); }",
                )
                .unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let s = select(&l.module, a).expect("selects");
            let g: std::vec::Vec<SelGroup> = s.groups.to_vec();
            let end = g.iter().position(|x| x.op == SelOp::LoopEnd).unwrap();
            assert_eq!(g[end - 1].op, SelOp::Machine(MachineOp::LoopTest));
            assert_eq!(g[end - 2].op, SelOp::Alu(ir::AluOp::Ior), "the counter's test or the flag");
            assert_eq!(g[end - 3].srcs[0], Some(SelSrc::Imm(7)), "the same count");
            assert_eq!(verify_loops(&g), Ok(()));
        })
        .unwrap();
    }

    /// A 2D shadow look-up computes `ref OP depth` for every compare
    /// function the sampler can hold, read from descriptor dword 12 as the
    /// function less GL_NEVER (bit 0 less, 1 equal, 2 greater), with the
    /// reference clamped to [0, 1] only when bit 3 says the depth is
    /// fixed-point. The selected groups are evaluated at the selection
    /// level: the sample returns `depth` in lane 0, dword 12 comes from the
    /// arguments, and no other descriptor word may be read.
    #[cfg(not(feature = "report-es2"))] // GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_shadow_look_up_compares_the_reference_with_the_depth_by_the_samplers_function() {
        use crate::shaderc::online::{lower, sema::Stage};
        use std::collections::BTreeMap;
        let _l = store::test_lock();
        /* (dword 12, depth) -> the output's lane 0 */
        let run = |refv: f32, cases: &[(u32, f32)]| -> std::vec::Vec<f32> {
            store::with_scratch(|a| {
                let src = std::format!(
                    "#version 300 es\nprecision mediump float; uniform mediump sampler2DShadow s; \
                     layout(location = 0) out vec4 o; \
                     void main(){{ o = vec4(texture(s, vec3(0.25, 0.75, {refv:?})), 0.0, 0.0, 1.0); }}"
                );
                let text = a.alloc_str(&src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let sel = select(&l.module, a).expect("selects");
                let out = sel.outputs.iter().find_map(|(_, c)| c[0]).expect("an output");
                cases
                    .iter()
                    .map(|&(op, depth)| {
                        let mut v: BTreeMap<u32, u32> = BTreeMap::new();
                        let read = |s: &SelSrc, v: &BTreeMap<u32, u32>| match *s {
                            SelSrc::Virtual { v: r, mods } => {
                                assert_eq!(mods, Mods::default(), "no modifier in this program");
                                v[&r.0]
                            }
                            SelSrc::Imm(b) => b,
                            ref f => panic!("an unexpected source {f:?}"),
                        };
                        let fl = |x: u32| f32::from_bits(x);
                        for g in sel.groups {
                            let x: std::vec::Vec<u32> = g.srcs.iter().flatten().map(|s| read(s, &v)).collect();
                            let r = match g.op {
                                SelOp::Sample { .. } => {
                                    let Some(SelDst::Virtual(d)) = g.dst else { panic!("a fixed sample") };
                                    for lane in 0..LANES {
                                        v.insert(d.0 + lane, if lane == 0 { depth.to_bits() } else { 0 });
                                    }
                                    continue;
                                }
                                SelOp::TexWord { word: 12, .. } => op,
                                SelOp::Copy => x[0],
                                SelOp::Alu(ir::AluOp::Flt) => u32::from(fl(x[0]) < fl(x[1])),
                                SelOp::Alu(ir::AluOp::Feq) => u32::from(fl(x[0]) == fl(x[1])),
                                SelOp::Alu(ir::AluOp::Fmax) => fl(x[0]).max(fl(x[1])).to_bits(),
                                SelOp::Alu(ir::AluOp::Fmin) => fl(x[0]).min(fl(x[1])).to_bits(),
                                SelOp::Alu(ir::AluOp::Iand) => x[0] & x[1],
                                SelOp::Alu(ir::AluOp::Ior) => x[0] | x[1],
                                SelOp::Alu(ir::AluOp::Ine) => u32::from(x[0] != x[1]),
                                SelOp::Alu(ir::AluOp::Bcsel) => {
                                    if x[0] != 0 {
                                        x[1]
                                    } else {
                                        x[2]
                                    }
                                }
                                o => panic!("{o:?} is not in this evaluator"),
                            };
                            let Some(SelDst::Virtual(d)) = g.dst else { panic!("a fixed destination") };
                            v.insert(d.0, r);
                        }
                        f32::from_bits(read(&out, &v))
                    })
                    .collect()
            })
            .unwrap()
        };
        const FIXED: u32 = 8; /* a fixed-point depth: the reference is clamped */
        let want = |bits: u32, r: f32, d: f32| -> f32 {
            let hold = (bits & 1 != 0 && r < d) || (bits & 2 != 0 && r == d) || (bits & 4 != 0 && r > d);
            if hold {
                1.0
            } else {
                0.0
            }
        };
        for refv in [0.25f32, 0.5, 0.75] {
            let mut cases = std::vec::Vec::new();
            for bits in 0..8u32 {
                cases.push((bits | FIXED, 0.5f32));
            }
            let got = run(refv, &cases);
            let exp: std::vec::Vec<f32> = (0..8).map(|b| want(b, refv, 0.5)).collect();
            assert_eq!(got, exp, "reference {refv} against depth 0.5, functions NEVER..ALWAYS");
        }
        /* A reference past 1.0 is clamped for a fixed-point depth (1.5 ->
         * 1.0, equal to a depth of 1.0) and not for a float one. */
        let got = run(1.5, &[(2 | FIXED, 1.0), (2, 1.0), (4 | FIXED, 1.0), (4, 1.0)]);
        assert_eq!(got, [1.0, 0.0, 0.0, 1.0], "EQUAL / GREATER with and without the fixed-point clamp");
        /* TEXTURE_COMPARE_MODE NONE (bit 4): the depth itself, "r = Dt"
         * (ES 3.0.6 3.8.15.1), whatever the function bits */
        let got = run(0.25, &[(16, 0.625), (16 | 7, 0.625), (16 | FIXED, 0.375)]);
        assert_eq!(got, [0.625, 0.625, 0.375], "no compare: the sampled depth");
    }

    /// A cube and a 2D array shadow look-up select too: the depth through
    /// the plain look-up's arm, the reference from lane 3.
    #[cfg(not(feature = "report-es2"))] // GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn cube_and_array_shadow_look_ups_select() {
        use crate::shaderc::online::{lower, sema::Stage};
        let _l = store::test_lock();
        for (decl, call) in [
            ("samplerCubeShadow", "texture(s, vec4(0.1, 0.2, 0.9, 0.5))"),
            ("sampler2DArrayShadow", "texture(s, vec4(0.1, 0.2, 1.0, 0.5))"),
            ("sampler2DShadow", "textureLod(s, vec3(0.1, 0.2, 0.5), 1.0)"),
        ] {
            /* a 2D array needs its hidden pvr_layers_ uniform, which the
             * front end declares on a PowerVR build only (reflect.rs) */
            if decl == "sampler2DArrayShadow" && cfg!(not(feature = "gpu-powervr")) {
                continue;
            }
            store::with_scratch(|a| {
                let src = std::format!(
                    "#version 300 es\nprecision mediump float; uniform mediump {decl} s; \
                     layout(location = 0) out vec4 o; void main(){{ o = vec4({call}); }}"
                );
                let text = a.alloc_str(&src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let sel = select(&l.module, a).unwrap_or_else(|e| panic!("{decl}: {e:?}"));
                let words: std::vec::Vec<u8> = sel
                    .groups
                    .iter()
                    .filter_map(|g| if let SelOp::TexWord { word, .. } = g.op { Some(word) } else { None })
                    .collect();
                assert!(
                    words.iter().filter(|w| **w == 12).count() == 1 && !words.contains(&7),
                    "{decl}: the compare word is read once, the pack info not at all: {words:?}"
                );
                assert!(
                    sel.groups.iter().any(|g| matches!(g.op, SelOp::Sample { .. })),
                    "{decl}: the depth is sampled"
                );
            })
            .unwrap();
        }
    }

    /// A value carried past a `break` on data is selected once per
    /// iteration, on the or of the two flags, not twice: no select is left
    /// whose other arm is a select of the same value, and the ors exist.
    #[test]
    fn a_value_kept_past_a_break_is_one_select_on_the_or_of_its_guards() {
        use crate::shaderc::online::{lower, sema::Stage};
        let _l = store::test_lock();
        store::with_scratch(|a| {
            let text = a
                .alloc_str(
                    "precision mediump float; uniform vec4 u; void main(){ float s = 0.0; \
                     for (float i = 1.0; i <= 6.0; i += 1.0) { if (i >= u.x) { break; } s += i * u.y; } \
                     gl_FragColor = vec4(s); }",
                )
                .unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let s = select(&l.module, a).expect("selects");
            let bcsel = SelOp::Alu(ir::AluOp::Bcsel);
            let def = |v: VReg| s.groups.iter().find(|g| g.dst == Some(SelDst::Virtual(v)));
            let nested = s.groups.iter().filter(|g| g.op == bcsel).filter(|g| match g.srcs {
                [_, Some(t), Some(SelSrc::Virtual { v, .. })] => {
                    def(v).is_some_and(|inner| inner.op == bcsel && inner.srcs[1] == Some(t))
                }
                _ => false,
            });
            assert_eq!(nested.count(), 0, "{:?}", s.groups);
            assert!(s.groups.iter().any(|g| g.op == SelOp::Alu(ir::AluOp::Ior)), "no guard was merged");
        })
        .unwrap();
    }

    /// A modifier on a constant is applied to the constant, bit-exactly,
    /// in the order a source modifier applies (floor, abs, negate).
    #[test]
    fn a_modifier_on_a_constant_is_folded_into_it() {
        let f =
            |x: f32, neg: bool, abs: bool, flr: bool| f32::from_bits(fold_mods(x.to_bits(), Mods { neg, abs, flr }));
        assert_eq!(f(1.0, true, false, false), -1.0, "x - 1.0 arrives as x + neg(1.0)");
        assert_eq!(f(-2.5, false, true, false), 2.5);
        assert_eq!(f(2.5, true, true, false), -2.5);
        assert_eq!(f(-0.5, false, false, true), -1.0);
        assert_eq!(f(0.5, false, false, true), 0.0);
        assert_eq!(f(-3.0, false, false, true), -3.0);
        assert_eq!(f(1.0e9, false, false, true), 1.0e9);
        assert_eq!(fold_mods((-0.0f32).to_bits(), Mods { flr: true, ..Mods::default() }), (-0.0f32).to_bits());
        assert_eq!(f(-0.25, true, false, true), 1.0, "floor first, then negate");
        assert!(f(f32::NAN, false, false, true).is_nan());
        for x in [-7.75f32, -1.0, -0.1, 0.0, 0.1, 3.999, 12345.5] {
            assert_eq!(f(x, false, false, true), x.floor(), "{x}");
        }
    }

    fn sel<'r>(m: &ir::Module<'_>, f: impl FnOnce(Result<Selected<'_>, SelectError>) -> &'r str) -> &'r str {
        let _s = store::test_lock();
        store::with_scratch(|a| f(select(m, a))).unwrap()
    }

    /// One declared slot, so that a test about some other refusal does not
    /// trip the malformed-IR one instead.
    const ONE_SLOT: &[ir::VarDecl] = &[ir::VarDecl { name: "c", ty: ir::Type::F32X4, array_len: 1 }];

    fn vs<'a>(instrs: &'a [ir::Op], nodes: &'a [ir::Node]) -> ir::Module<'a> {
        ir::Module {
            instrs,
            nodes,
            locals: ONE_SLOT,
            body: ir::Range { first: 0, len: nodes.len() as u32 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        }
    }

    fn body(n: usize) -> Vec<ir::Node> {
        (0..n).map(|i| ir::Node::Instr(ir::InstrId(i as u32))).collect()
    }

    /// A vector select whose condition is a dot product read at lanes 1..3
    /// without a splat -- the shape faceforward's lowering had -- reads
    /// registers nothing writes, and is refused rather than drawn. The same
    /// program with the condition splatted goes through.
    #[test]
    fn a_lane_of_a_one_lane_value_read_unsplatted_is_refused() {
        let splat = ir::Src { val: ir::ValueId(1), swizzle: [0; 4] };
        for (cond, refused) in [(ir::Src::id(ir::ValueId(1)), true), (splat, false)] {
            let instrs = [
                ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
                ir::Op::Alu(ir::AluOp::Fdot4, [ir::Src::id(ir::ValueId(0)); 3]),
                ir::Op::Alu(ir::AluOp::Bcsel, [cond, ir::Src::id(ir::ValueId(0)), ir::Src::id(ir::ValueId(0))]),
                ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(2)) },
            ];
            let nodes = body(instrs.len());
            let m = vs(&instrs, &nodes);
            let got = sel(&m, |r| match r {
                Err(SelectError::Unsupported(s)) => s,
                Ok(_) => "",
                other => panic!("{:?}", other.map(|_| ())),
            });
            if refused {
                assert_eq!(got, "a register read that nothing writes");
            } else {
                assert_eq!(got, "", "the splatted condition goes through");
            }
        }
    }

    /// One group per component, which is the whole difference from the
    /// GC7000: there a `vec4` add is one instruction with a write mask.
    #[test]
    fn a_vector_operation_becomes_one_group_per_component() {
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes = body(3);
        let m = vs(&instrs, &nodes);
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.groups.len(), 4, "four components, four groups");
            for (lane, g) in s.groups.iter().enumerate() {
                assert_eq!(g.op, SelOp::Alu(ir::AluOp::Fadd));
                assert_eq!(g.dst, Some(SelDst::Virtual(VReg(LANES + lane as u32))));
                /* The attribute is read straight out of the vertex-input
                 * bank: location 0 component `lane`. */
                assert_eq!(
                    g.srcs[0],
                    Some(SelSrc::Fixed { bank: Bank::VtxIn, reg: lane as u16, mods: Mods::default() })
                );
            }
            /* The output is not a group: `uvsw_write` is a backend
             * operation, so the builder emits it from this list. */
            assert_eq!(s.outputs.len(), 1);
            assert_eq!(s.outputs[0].0, ir::IoSlot::Position);
            assert!(s.outputs[0].1.iter().all(|c| c.is_some()), "all four components of a position are written");
        })
        .unwrap();
    }

    /// An arm that writes nothing still needs the slot's value.
    ///
    /// The same defect the GC7000 side had, in the same shape and found the
    /// same way: the join selects from the then arm's register for every slot
    /// EITHER arm writes, and the seed that fills that register only ran when
    /// the then arm could observe the slot. `if (c) { } else { v = b; }` then
    /// selected a register nobody wrote.
    ///
    /// Asserted as a property over the whole program -- every virtual
    /// register a group reads is written by an earlier group, or is a varying
    /// the builder fills before the program runs -- because the defect is "a
    /// register nobody wrote" and naming the select's operand would pin the
    /// symptom instead.
    #[test]
    fn an_arm_that_writes_nothing_still_needs_the_slots_value() {
        let locals = [ir::VarDecl { name: "v", ty: ir::Type::F32X4, array_len: 1 }];
        let instrs = [
            ir::Op::LoadUniform { dword_off: 0, comps: 1 },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: 0b1111,
            },
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(1), comps: 4 },
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(3)),
                write_mask: 0b1111,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(5)) },
        ];
        let nodes = [
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range::EMPTY,
                else_: ir::Range { first: 6, len: 2 },
            },
            ir::Node::Instr(ir::InstrId(5)),
            ir::Node::Instr(ir::InstrId(6)),
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
        ];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            locals: &locals,
            body: ir::Range { first: 0, len: 6 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            let mut written: Vec<VReg> = Vec::new();
            for (_, comps) in s.inputs.iter() {
                for v in comps.iter().flatten() {
                    written.push(*v);
                }
            }
            for (i, g) in s.groups.iter().enumerate() {
                for src in g.srcs.iter().flatten() {
                    if let SelSrc::Virtual { v, .. } = src {
                        assert!(written.contains(v), "group {i} ({:?}) reads {v:?}, which nothing has written", g.op);
                    }
                }
                if let Some(SelDst::Virtual(v)) = g.dst {
                    written.push(v);
                }
            }
        })
        .unwrap();
    }

    /// A component nothing reads costs no group, and one only an OUTPUT
    /// reads is not dead.
    ///
    /// The two halves of `eliminate_dead`, in the order they can go wrong.
    /// The first is what the pass is for: this machine is scalar, selection
    /// gives every value four registers, and a `vec4` sum whose `.y` alone is
    /// wanted computes three numbers nobody asks for.
    ///
    /// The second is the way the pass can be catastrophically wrong, which is
    /// why it is asserted beside it. An output's components are carried in
    /// `Selected::outputs` for the BUILDER to write -- no group reads them --
    /// so a liveness counting only group operands would find every final
    /// value unread and delete the entire shader, leaving a program that
    /// computes nothing and still assembles. Removing the `outputs` check
    /// from the pass makes this test fail rather than the corpus shrink to
    /// nothing quietly.
    #[test]
    fn a_component_nothing_reads_costs_no_group_and_an_output_is_a_reader() {
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            /* A four-component sum ... */
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
            /* ... of which one component is read, and by an output only. */
            ir::Op::StoreOutput {
                slot: ir::IoSlot::PointSize,
                src: ir::Src { val: ir::ValueId(1), swizzle: [1, 1, 1, 1] },
            },
        ];
        let nodes = body(3);
        let m = ir::Module {
            outputs: &[ir::IoDecl {
                name: "gl_PointSize",
                kind: ir::IoKind::PointSize,
                location: 0,
                components: 1,
                interp: ir::INTERP_SMOOTH,
                gl_type: 0,
            }],
            ..vs(&instrs, &nodes)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.groups.len(), 1, "one component is read, so three of the four groups are dead");
            /* And it is the component the output named, not just any one. */
            assert_eq!(s.groups[0].dst, Some(SelDst::Virtual(VReg(LANES + 1))));
            assert_eq!(s.outputs.len(), 1);
            assert_eq!(s.outputs[0].1[0], Some(SelSrc::Virtual { v: VReg(LANES + 1), mods: Mods::default() }));
        })
        .unwrap();
    }

    /// `Fneg`, `Fabs` and `Ffloor` are source modifiers on this machine, so
    /// they cost no group at all -- and getting that wrong costs an `mbyp`
    /// per use, which the corpus does not have.
    #[test]
    fn a_modifier_costs_no_group_and_rides_its_consumer() {
        let instrs = [
            ir::Op::LoadUniform { dword_off: 4, comps: 1 },
            ir::Op::Alu(ir::AluOp::Fneg, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::Alu(
                ir::AluOp::Fmul,
                [ir::Src::id(ir::ValueId(1)), ir::Src::id(ir::ValueId(0)), ir::Src::id(ir::ValueId(0))],
            ),
            ir::Op::StoreOutput { slot: ir::IoSlot::PointSize, src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes = body(4);
        let m = vs(&instrs, &nodes);
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            /* The claim is that the NEGATE costs nothing, not how many
             * components the multiply has.
             *
             * es100-frontend's 0132a89 made `comps_of` ask the instruction
             * before falling back to 4, so a `LoadUniform { comps: 1 }` or a
             * reduction now answers exactly even in hand-built IR. An
             * ordinary `Alu` still answers 4, and SHOULD: its width is the
             * checker's type and only the front end has that. In a real
             * compile the table is filled and the multiply is one lane; here
             * it costs three groups in a test and nothing in a shader, which
             * is the right way round for an over-estimate to fail. */
            assert!(
                s.groups.iter().all(|g| g.op == SelOp::Alu(ir::AluOp::Fmul)),
                "the negate is free; only the multiply is a group"
            );
            let g = &s.groups[0];
            /* The uniform read is folded too: a shared-file register, with
             * the negate on the operand. */
            assert_eq!(
                g.srcs[0],
                Some(SelSrc::Fixed { bank: Bank::Shared, reg: 4, mods: Mods { neg: true, ..Mods::default() } })
            );
            assert_eq!(g.srcs[1], Some(SelSrc::Fixed { bank: Bank::Shared, reg: 4, mods: Mods::default() }));
        })
        .unwrap();
    }

    /// A dot product is `fmul` then one `fmad` per further term, and the last
    /// one writes the result's lane 0.
    #[test]
    fn a_dot_product_is_a_multiply_then_an_accumulate_per_term() {
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            ir::Op::Alu(ir::AluOp::Fdot3, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreOutput { slot: ir::IoSlot::PointSize, src: ir::Src::id(ir::ValueId(1)) },
        ];
        let nodes = body(3);
        let m = vs(&instrs, &nodes);
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.groups.len(), 3, "three terms, three groups");
            assert_eq!(s.groups[0].op, SelOp::Alu(ir::AluOp::Fmul));
            assert_eq!(s.groups[1].op, SelOp::Alu(ir::AluOp::Ffma));
            assert_eq!(s.groups[2].op, SelOp::Alu(ir::AluOp::Ffma));
            /* ONE register: every term writes the result and each accumulate
             * reads it back. */
            let r = VReg(LANES); // lane 0 of value 1's block
            for g in s.groups {
                assert_eq!(g.dst, Some(SelDst::Virtual(r)), "every term writes the result register");
            }
            assert_eq!(s.groups[1].srcs[2], Some(SelSrc::Virtual { v: r, mods: Mods::default() }));
            assert_eq!(s.groups[2].srcs[2], Some(SelSrc::Virtual { v: r, mods: Mods::default() }));
            /* Every accumulate READS the register it writes. This asserted
             * the opposite until pvrgpu-platform measured the corpus
             * (a4a6935): 266,932 groups read a register they also write, and
             * `fmad` into its own destination is the ordinary case. The
             * caution cost a register per dot. */
            for g in s.groups.iter().filter(|g| g.op == SelOp::Alu(ir::AluOp::Ffma)) {
                let Some(SelDst::Virtual(d)) = g.dst else { panic!("an accumulate writes a register") };
                assert!(
                    g.srcs.iter().flatten().any(|x| matches!(x, SelSrc::Virtual { v, .. } if *v == d)),
                    "the accumulate must read its own destination"
                );
            }
        })
        .unwrap();
    }

    /// A varying is iterated once by the builder and read as a register, and
    /// one the shader never reads is not iterated at all.
    #[test]
    fn a_varying_is_one_register_block_however_often_it_is_read() {
        let inputs = [
            ir::IoDecl { name: "v_read", kind: ir::IoKind::Varying, location: 0, components: 2, interp: 0, gl_type: 0 },
            ir::IoDecl {
                name: "v_unread",
                kind: ir::IoKind::Varying,
                location: 1,
                components: 4,
                interp: 0,
                gl_type: 0,
            },
        ];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 2 },
            ir::Op::LoadInput { slot: ir::IoSlot::Varying(0), comps: 2 },
            /* .xyxy: a vec2 read as four lanes names only its two */
            ir::Op::Alu(
                ir::AluOp::Fadd,
                [
                    ir::Src { val: ir::ValueId(0), swizzle: [0, 1, 0, 1] },
                    ir::Src { val: ir::ValueId(1), swizzle: [0, 1, 0, 1] },
                    ir::Src { val: ir::ValueId(0), swizzle: [0, 1, 0, 1] },
                ],
            ),
            ir::Op::StoreOutput { slot: ir::IoSlot::FragColor(0), src: ir::Src::id(ir::ValueId(2)) },
        ];
        let nodes = body(4);
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            inputs: &inputs,
            body: ir::Range { first: 0, len: 4 },
            ..ir::Module::empty(SHC_STAGE_FRAGMENT)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("selects");
            assert_eq!(s.inputs.len(), 1, "the unread varying is not iterated");
            assert_eq!(s.inputs[0].0, ir::IoSlot::Varying(0));
            assert_eq!(s.inputs[0].1[0..2].iter().filter(|r| r.is_some()).count(), 2, "two components");
            assert!(s.inputs[0].1[2].is_none(), "and only two");
            /* Both reads name the same registers: one iterator, not two. */
            let block = s.inputs[0].1[0].expect("component 0 has a register");
            assert!(
                s.groups.iter().any(|g| g
                    .srcs
                    .iter()
                    .flatten()
                    .any(|x| matches!(x, SelSrc::Virtual { v, .. } if *v == block))),
                "the add reads the iterated register"
            );
        })
        .unwrap();
    }

    /// Every refusal names its own open question rather than saying "not
    /// supported", because each of these is a decision somebody has to make
    /// and a shared message hides which one.
    #[test]
    fn each_refusal_names_the_question_behind_it() {
        let cases: &[(&[ir::Op], &str)] = &[
            (
                &[
                    ir::Op::LoadInput { slot: ir::IoSlot::FrontFacing, comps: 1 },
                    ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
                ],
                "built-in fragment input",
            ),
            /* a projective shadow look-up (textureProj on a shadow sampler):
             * still refused. The plain shadow look-up selects since the
             * shadow arm; this is the catch-all's example now. */
            (
                &[ir::Op::Tex {
                    dim: ir::TexDim::D2,
                    shadow: true,
                    proj: true,
                    ret: ir::ScalarType::F32,
                    sampler: 0,
                    coord: ir::Src::id(ir::ValueId(0)),
                    lod: ir::TexLod::Implicit,
                    offset: None,
                }],
                "texture fetch",
            ),
        ];
        for (instrs, want) in cases {
            let nodes = body(instrs.len());
            let m = vs(instrs, &nodes);
            let got = sel(&m, |r| match r {
                Err(SelectError::Unsupported(s)) => s,
                other => panic!("expected a named refusal, got {:?}", other.map(|_| ())),
            });
            assert!(got.contains(want), "refusal for {want:?} reads {got:?}");
        }
    }

    /// A leaf `if` is if-converted into a select per component of every
    /// slot an arm writes, and so is a nested one: each level joins its own
    /// arms, the outer select reading the inner's result as its then-value.
    /// A loop that leaves on data refuses, because this machine has no
    /// branch to fall back to.
    ///
    /// The GC7000 side measured that if-converting a nested `if` is
    /// quadratic and branches instead. The corpus's nested `if`s are one or
    /// two levels deep (a select per written component per level); the
    /// meaning is checked against the IR run with real branches in
    /// tests/usc_limits.rs, with every `if` driven both ways.
    #[test]
    fn a_leaf_if_and_a_nested_one_become_selects_and_an_unbounded_loop_runs_to_the_cap() {
        let locals = [ir::VarDecl { name: "c", ty: ir::Type::F32X4, array_len: 1 }];
        let instrs = [
            ir::Op::LoadInput { slot: ir::IoSlot::Attrib(0), comps: 4 },
            /* Computed, not folded: a store of a uniform read would be a
             * plain copy, which is the refusal this test is not about. */
            ir::Op::Alu(ir::AluOp::Fadd, [ir::Src::id(ir::ValueId(0)); 3]),
            ir::Op::StoreLocal {
                local: ir::LocalId(0),
                index: None,
                src: ir::Src::id(ir::ValueId(1)),
                write_mask: 0b1111,
            },
            ir::Op::LoadLocal { local: ir::LocalId(0), index: None, comps: 4 },
            ir::Op::StoreOutput { slot: ir::IoSlot::Position, src: ir::Src::id(ir::ValueId(3)) },
        ];
        /* 0..1 is the arm (the uniform read and the store); the body is the
         * attribute read, the `if`, the load and the output store. */
        let leaf = [
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 0, len: 2 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
        ];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &leaf,
            locals: &locals,
            body: ir::Range { first: 2, len: 4 },
            uniform_dwords: 8,
            ..ir::Module::empty(SHC_STAGE_VERTEX)
        };
        let _s = store::test_lock();
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a leaf `if` is if-converted");
            let sels = s.groups.iter().filter(|g| g.op == SelOp::Alu(ir::AluOp::Bcsel)).count();
            assert_eq!(sels, 4, "one select per component of the one written slot");
            /* Each select takes the then arm's register (the stored value's
             * own: a store moves the slot, it does not copy) and, as its
             * false operand, the slot's value from before the `if` (the
             * else arm left it), into a fresh register -- nothing is
             * written twice. */
            for (lane, g) in s.groups.iter().filter(|g| g.op == SelOp::Alu(ir::AluOp::Bcsel)).enumerate() {
                let lane = lane as u32;
                let v = |r: u32| Some(SelSrc::Virtual { v: VReg(r), mods: Mods::default() });
                assert_eq!(g.srcs[1], v(LANES + lane), "the then arm's value, value 1 lane {lane}");
                /* nothing stored to the slot before the `if`: its zero
                 * fill, read through the fill's copy (`forward_copies`) */
                assert_eq!(g.srcs[2], Some(SelSrc::Imm(0)), "the slot before the `if`");
                let Some(SelDst::Virtual(d)) = g.dst else { panic!("a select writes a register") };
                assert!(d.0 >= s.vregs - 16, "a fresh register");
            }
        })
        .unwrap();

        /* The same `if`, now inside another one's arm. */
        let nested = [
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 0, len: 2 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 2, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
        ];
        let m = ir::Module { nodes: &nested, body: ir::Range { first: 3, len: 4 }, ..m };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("a nested `if` is if-converted");
            let sels: Vec<&SelGroup> = s.groups.iter().filter(|g| g.op == SelOp::Alu(ir::AluOp::Bcsel)).collect();
            assert_eq!(sels.len(), 8, "four selects for the inner `if`, four for the outer");
            /* The outer join's then-value is the inner join's result: what
             * the slot holds after the inner `if`, not its pre-`if` value. */
            for (inner, outer) in sels[..4].iter().zip(&sels[4..]) {
                let Some(SelDst::Virtual(d)) = inner.dst else { panic!("a select writes a register") };
                assert_eq!(outer.srcs[1], Some(SelSrc::Virtual { v: d, mods: Mods::default() }));
            }
        })
        .unwrap();

        /* A loop in the arm instead. A loop with a known trip count is
         * unrolled before selection (`unroll.rs`), one that also leaves on
         * data under flags; one with no bound at all (the body only breaks on
         * data) is counted at run time: kept as ONE loop, up to
         * UNIFORM_LOOP_CAP passes, inside the arm (both arms run, the join
         * keeps or drops what it computed). It was refused as "too long to
         * unroll" until loops could sit in an arm (the loops design, step 4). */
        let looped = [
            ir::Node::Break,
            ir::Node::Instr(ir::InstrId(1)),
            ir::Node::Instr(ir::InstrId(2)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 0, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Loop { body: ir::Range { first: 1, len: 3 } },
            ir::Node::Instr(ir::InstrId(0)),
            ir::Node::If {
                cond: ir::Src::id(ir::ValueId(0)),
                then_: ir::Range { first: 4, len: 1 },
                else_: ir::Range::EMPTY,
            },
            ir::Node::Instr(ir::InstrId(3)),
            ir::Node::Instr(ir::InstrId(4)),
        ];
        let m = ir::Module { nodes: &looped, body: ir::Range { first: 5, len: 4 }, ..m };
        store::with_scratch(|a| {
            let s = select(&m, a).expect("an unbounded loop in an arm runs to the cap");
            let count = |op: SelOp| s.groups.iter().filter(|g| g.op == op).count();
            assert_eq!((count(SelOp::LoopBegin), count(SelOp::LoopEnd)), (1, 1), "one loop, kept, not expanded");
            let cap = s.groups.iter().any(|g| {
                g.op == SelOp::Alu(ir::AluOp::Ile)
                    && g.srcs[0] == Some(SelSrc::Imm(crate::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP))
            });
            assert!(cap, "its back edge is bounded by the cap");
        })
        .unwrap();
    }
}
