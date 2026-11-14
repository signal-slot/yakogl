// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014, 2016, 2018 Intel Corporation
// SPDX-FileCopyrightText: 2014-2015 Connor Abbott
// SPDX-FileCopyrightText: 2014-2015 Broadcom
// SPDX-FileCopyrightText: 2016 Red Hat
// SPDX-FileCopyrightText: 2019 Valve Corporation
// SPDX-FileCopyrightText: 2018-2022 Alyssa Rosenzweig
// SPDX-FileCopyrightText: 2019-2021, 2026 Collabora, Ltd.
// SPDX-FileCopyrightText: 2020, 2021, 2025 Google LLC
// SPDX-FileCopyrightText: 2025 Arm Ltd.
// Derived from Mesa 26.2.1 src/compiler/nir/ (nir_search.c, nir_opt_vectorize.c,
// nir_opt_copy_propagate.c, nir_opt_dce.c, nir_opt_cse.c, nir_instr_set.c,
// nir_lower_alu_width.c, nir_opt_constant_folding.c, nir_opt_move.c,
// nir_opt_shrink_vectors.c, nir_lower_load_const_to_scalar.c,
// nir_range_analysis.c, nir_range_analysis.h, nir_search_helpers.h, nir.c,
// nir_builder.c, nir_algebraic.py's automaton, nir_lower_blend.c),
// src/panfrost/lib/pan_blend.c, src/panfrost/compiler/pan_nir.h and
// src/panfrost/compiler/bifrost/bifrost_nir.c (MIT; each holder above from
// those files' headers). The upstream part stays under its licence
// (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! The NIR of a blend shader, through Mesa's passes.
//!
//! Mesa builds a blend shader as NIR (`GENX(pan_blend_create_shader)`,
//! pan_blend.c:798-946, `nir_color_blend`, nir_lower_blend.c:909-990) and
//! optimises it with the bifrost back end's NIR passes
//! (`bifrost_preprocess_nir`, `bifrost_postprocess_nir`, `bi_optimize_late`,
//! bifrost_nir.c). The words the back end emits depend on every detail of
//! the NIR those passes leave: which products fused into an `ffma`, which
//! lanes paired into a vec2, the order of the instructions. This module is
//! that NIR on one block: the instructions in program order, each source's
//! use recorded in the order NIR's use lists keep (a source joins its
//! value's list when it is made or rewritten), and the passes, each a
//! transcription of Mesa's on the opcodes a blend shader holds.
//!
//! The algebraic passes run Mesa's own transforms (blend_nir_rules.rs,
//! generated from nir_opt_algebraic.py and bifrost_nir_algebraic.py) with
//! nir_search.c's matching, its automaton's notion of a changed state, and
//! its worklist order. A transform whose replacement this module does not
//! build, or a condition it does not evaluate, refuses the shader when it
//! would apply: the shader is then never built other than as Mesa builds
//! it.
//!
//! What is not modelled: the UBO load of the blend constants is kept as one
//! four-word load. Mesa splits it into four scalar loads
//! (`nir_lower_mem_access_bit_sizes`) and joins them again
//! (`nir_opt_load_store_vectorize`); the back end reads the words as FAU
//! either way (`bi_opt_push_ubo`), and those passes do not touch the 16-bit
//! code (measured once, 2026-11-12, with a harness outside the tree, on
//! the fp16 keys: the final NIR equals Mesa's printed one for the 1403 keys
//! of blend_deqp.words and 600 4x keys of blend_random.sha, and a Python
//! model of the same passes for 1500 more; the gate compares the words of
//! all 4103 keys, fp32 ones included, tools/mali/README-pan_shc.md "Blend
//! shaders").

use super::blend_nir_rules;
use super::select::f16_rtne;
use super::MaliError;

/// The most values a blend shader's NIR makes over all its passes (the
/// largest over the 4103 keys of blend_deqp.words and blend_random.sha is
/// 145); more refuses the key.
pub const MAX: usize = 256;
const NONE: u16 = u16::MAX;
/// The most uses one value has (the largest over the same 4103 keys is
/// 20); a value with more refuses the key.
const MAX_USES: usize = 48;

/// The NIR opcodes of a blend shader. `F2f` and `U2u` are the automaton's
/// unsized conversion opcodes, and `Other` a sized one of another size: a
/// search pattern with it never matches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Mov,
    Vec2,
    Vec3,
    Vec4,
    Fadd,
    Fmul,
    Ffma,
    Fneg,
    Fsat,
    Fmin,
    Fmax,
    Fsub,
    Fcanonicalize,
    F2f16,
    F2f,
    Iand,
    U2u8,
    U2u,
    Pack32_4x8Split,
    Other,
}

impl Op {
    fn num_inputs(self) -> usize {
        match self {
            Op::Mov | Op::Fneg | Op::Fsat | Op::Fcanonicalize | Op::F2f16 | Op::F2f | Op::U2u8 | Op::U2u => 1,
            Op::Vec2 | Op::Fadd | Op::Fmul | Op::Fmin | Op::Fmax | Op::Fsub | Op::Iand => 2,
            Op::Vec3 | Op::Ffma => 3,
            Op::Vec4 | Op::Pack32_4x8Split => 4,
            Op::Other => 0,
        }
    }

    fn is_vec(self) -> bool {
        matches!(self, Op::Vec2 | Op::Vec3 | Op::Vec4)
    }

    fn is_vec_or_mov(self) -> bool {
        self == Op::Mov || self.is_vec()
    }

    /// `nir_op_infos[op].output_size` (the vecs and the pack are sized).
    fn output_size(self) -> u8 {
        match self {
            Op::Vec2 => 2,
            Op::Vec3 => 3,
            Op::Vec4 => 4,
            Op::Pack32_4x8Split => 1,
            _ => 0,
        }
    }

    /// `nir_op_infos[op].input_sizes[i]` (a vec's and the pack's sources
    /// are scalars).
    fn input_size(self) -> u8 {
        if self.is_vec() || self == Op::Pack32_4x8Split {
            1
        } else {
            0
        }
    }

    /// `2src_commutative` (nir_opcodes.py).
    fn commutative(self) -> bool {
        matches!(self, Op::Fadd | Op::Fmul | Op::Ffma | Op::Fmin | Op::Fmax | Op::Iand)
    }

    /// A float opcode (its sources' base type is float).
    fn is_float(self) -> bool {
        matches!(
            self,
            Op::Fadd
                | Op::Fmul
                | Op::Ffma
                | Op::Fneg
                | Op::Fsat
                | Op::Fmin
                | Op::Fmax
                | Op::Fsub
                | Op::Fcanonicalize
                | Op::F2f16
        )
    }

    fn vec(n: u8) -> Op {
        match n {
            1 => Op::Mov,
            2 => Op::Vec2,
            3 => Op::Vec3,
            _ => Op::Vec4,
        }
    }
}

/// The intrinsics of a blend shader.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intr {
    /// `load_blend_input_pan`, the colour (fp16) or the dual source's (fp32).
    BlendInput,
    BlendInputDual,
    Coverage,
    SampleId,
    /// `load_tile_pan (pixel word, coverage, conversion)`.
    LoadTile,
    /// `load_ubo` of the sysval UBO's first four words, the blend constants.
    LoadUbo,
    Blend,
    BlendReturn,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Alu(Op),
    Const,
    Undef,
    Intr(Intr),
}

#[derive(Clone, Copy, Debug)]
pub struct Src {
    pub def: u16,
    pub swz: [u8; 4],
    /// The order of this use in its value's use list.
    seq: u32,
}

const NO_SRC: Src = Src { def: NONE, swz: [0, 1, 2, 3], seq: 0 };

#[derive(Clone, Copy, Debug)]
pub struct Instr {
    pub kind: Kind,
    pub comps: u8,
    pub bits: u8,
    pub n: u8,
    pub srcs: [Src; 4],
    /// A constant's components.
    pub k: [u64; 4],
    pub live: bool,
    pass_flag: u8,
}

const NO_INSTR: Instr =
    Instr { kind: Kind::Undef, comps: 0, bits: 0, n: 0, srcs: [NO_SRC; 4], k: [0; 4], live: false, pass_flag: 0 };

impl Instr {
    pub fn op(&self) -> Option<Op> {
        match self.kind {
            Kind::Alu(op) => Some(op),
            _ => None,
        }
    }
}

/// One use: the user and its source index.
#[derive(Clone, Copy)]
struct Use {
    user: u16,
    src: u8,
}

/// A blend shader's NIR: one block.
pub struct Nir<'a> {
    pub ins: &'a mut [Instr],
    alloc: usize,
    pub order: &'a mut [u16],
    pub len: usize,
    seq: u32,
    /// A value had more than [`MAX_USES`] uses.
    overflow: core::cell::Cell<bool>,
    /// The algebraic passes' automaton states and worklist (kept off the
    /// stack).
    states: &'a mut [State],
    worklist: &'a mut [u16],
}

type R<T> = Result<T, MaliError>;

fn defect() -> MaliError {
    MaliError::Defect(super::Defect::IrIndex)
}

impl<'a> Nir<'a> {
    pub fn new(scratch: &crate::shaderc::online::arena::Arena<'a>) -> R<Nir<'a>> {
        let ins = scratch.alloc_filled(MAX, NO_INSTR).map_err(|_| MaliError::OutOfScratch)?;
        let order = scratch.alloc_filled(MAX, NONE).map_err(|_| MaliError::OutOfScratch)?;
        let states = scratch.alloc_filled(MAX, WILD).map_err(|_| MaliError::OutOfScratch)?;
        let worklist = scratch.alloc_filled(4 * MAX, 0u16).map_err(|_| MaliError::OutOfScratch)?;
        Ok(Nir { ins, alloc: 0, order, len: 0, seq: 0, overflow: core::cell::Cell::new(false), states, worklist })
    }

    pub fn at(&self, k: usize) -> u16 {
        self.order[k]
    }

    fn pos(&self, i: u16) -> R<usize> {
        self.order[..self.len].iter().position(|&x| x == i).ok_or_else(defect)
    }

    /// A new instruction, not yet in the block.
    fn new_instr(&mut self, kind: Kind, comps: u8, bits: u8) -> R<u16> {
        if self.alloc == MAX {
            return Err(MaliError::OutOfScratch);
        }
        let i = self.alloc;
        self.alloc += 1;
        self.ins[i] = Instr { kind, comps, bits, ..NO_INSTR };
        Ok(i as u16)
    }

    /// Set source `k` of `user`, not yet joining a use list (an instruction
    /// out of the block).
    fn set_src(&mut self, user: u16, k: usize, def: u16, swz: [u8; 4]) {
        let u = &mut self.ins[user as usize];
        u.srcs[k] = Src { def, swz, seq: 0 };
        u.n = u.n.max(k as u8 + 1);
    }

    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// Put `i` at block position `at`, its sources joining their use lists.
    fn insert(&mut self, i: u16, at: usize) -> R<()> {
        if self.len == MAX {
            return Err(MaliError::OutOfScratch);
        }
        self.order.copy_within(at..self.len, at + 1);
        self.order[at] = i;
        self.len += 1;
        self.ins[i as usize].live = true;
        for k in 0..self.ins[i as usize].n as usize {
            let s = self.next_seq();
            self.ins[i as usize].srcs[k].seq = s;
        }
        Ok(())
    }

    /// `nir_instr_remove`: out of the block, its sources out of their lists.
    fn remove(&mut self, i: u16) -> R<()> {
        let p = self.pos(i)?;
        self.order.copy_within(p + 1..self.len, p);
        self.len -= 1;
        self.ins[i as usize].live = false;
        Ok(())
    }

    /// `nir_src_rewrite`: the source moves to the end of `def`'s use list.
    fn rewrite_src(&mut self, u: Use, def: u16) {
        let s = self.next_seq();
        let src = &mut self.ins[u.user as usize].srcs[u.src as usize];
        src.def = def;
        src.seq = s;
    }

    /// The uses of `d`, in use-list order.
    fn uses(&self, d: u16, out: &mut [Use; MAX_USES]) -> usize {
        let mut seqs = [0u32; MAX_USES];
        let mut n = 0;
        for k in 0..self.len {
            let i = self.order[k];
            let ins = &self.ins[i as usize];
            for s in 0..ins.n as usize {
                if ins.srcs[s].def == d {
                    if n == MAX_USES {
                        self.overflow.set(true);
                        return n;
                    }
                    /* insertion sort on the sequence */
                    let seq = ins.srcs[s].seq;
                    let mut j = n;
                    while j > 0 && seqs[j - 1] > seq {
                        seqs[j] = seqs[j - 1];
                        out[j] = out[j - 1];
                        j -= 1;
                    }
                    seqs[j] = seq;
                    out[j] = Use { user: i, src: s as u8 };
                    n += 1;
                }
            }
        }
        n
    }

    fn has_uses(&self, d: u16) -> bool {
        (0..self.len).any(|k| {
            let ins = &self.ins[self.order[k] as usize];
            ins.srcs[..ins.n as usize].iter().any(|s| s.def == d)
        })
    }

    fn use_count(&self, d: u16) -> usize {
        let mut n = 0;
        for k in 0..self.len {
            let ins = &self.ins[self.order[k] as usize];
            n += ins.srcs[..ins.n as usize].iter().filter(|s| s.def == d).count();
        }
        n
    }

    /// `nir_def_rewrite_uses`: every use of `old`, in order, to `new`.
    fn rewrite_uses(&mut self, old: u16, new: u16) {
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let n = self.uses(old, &mut u);
        for x in &u[..n] {
            self.rewrite_src(*x, new);
        }
    }

    /// `nir_ssa_alu_instr_src_components`.
    fn src_comps(&self, user: u16, k: usize) -> u8 {
        let u = &self.ins[user as usize];
        match u.kind {
            Kind::Alu(op) if op.input_size() != 0 => op.input_size(),
            Kind::Alu(_) => u.comps,
            _ => self.ins[u.srcs[k].def as usize].comps,
        }
    }

    fn is_alu(&self, i: u16) -> bool {
        matches!(self.ins[i as usize].kind, Kind::Alu(_))
    }

    fn is_const(&self, i: u16) -> bool {
        self.ins[i as usize].kind == Kind::Const
    }

    fn alu_op(&self, i: u16) -> Option<Op> {
        self.ins[i as usize].op()
    }

    // ---- construction helpers --------------------------------------------

    fn push(&mut self, i: u16) -> R<u16> {
        let at = self.len;
        self.insert(i, at)?;
        Ok(i)
    }

    fn konst(&mut self, bits: u8, vals: &[u64]) -> R<u16> {
        let i = self.new_instr(Kind::Const, vals.len() as u8, bits)?;
        self.ins[i as usize].k[..vals.len()].copy_from_slice(vals);
        Ok(i)
    }

    fn alu(&mut self, op: Op, comps: u8, bits: u8, srcs: &[(u16, [u8; 4])]) -> R<u16> {
        let i = self.new_instr(Kind::Alu(op), comps, bits)?;
        for (k, &(d, swz)) in srcs.iter().enumerate() {
            self.set_src(i, k, d, swz);
        }
        Ok(i)
    }

    fn intr(&mut self, op: Intr, comps: u8, bits: u8, srcs: &[u16]) -> R<u16> {
        let i = self.new_instr(Kind::Intr(op), comps, bits)?;
        for (k, &d) in srcs.iter().enumerate() {
            self.set_src(i, k, d, [0, 1, 2, 3]);
        }
        Ok(i)
    }
}

/// A swizzle of one component.
const fn sw(c: u8) -> [u8; 4] {
    [c, 0, 0, 0]
}

const ID: [u8; 4] = [0, 1, 2, 3];

// ---------------------------------------------------------------- building

/// A blend factor as gallium spells it (`pipe_blendfactor` without its
/// invert bit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Base {
    One,
    SrcColor,
    SrcAlpha,
    DstAlpha,
    DstColor,
    SrcAlphaSaturate,
    ConstColor,
    ConstAlpha,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Factor {
    pub base: Base,
    pub invert: bool,
}

/// `pipe_blend_func`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Func {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Channel {
    pub func: Func,
    pub src: Factor,
    pub dst: Factor,
}

/// Mesa's blend shader key without the constants
/// (`pan_blend_shader_key`, pan_blend.h): the colour's and the alpha's
/// equations, the colour mask, the sample count, the fragment colour's type
/// (`src0_type`: fp16, or fp32 with `colour_f32`); an RGBA8 UNORM target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    pub rgb: Channel,
    pub alpha: Channel,
    pub mask: u8,
    pub samples: u8,
    /// The fragment program writes the colour as fp32 (a highp ES 3.00
    /// `out vec4`), read from r0-r3 and converted (pan_blend.c:847-851).
    pub colour_f32: bool,
}

const ONE16: u64 = 0x3c00;

/// The NIR `pan_blend_create_shader` builds for `key`, as it stands after
/// the first `nir_lower_alu_width` (the vec4 conversions and clamps split
/// into pairs): the first NIR Mesa's passes print. The builder's calls are
/// Mesa's, in its order; where a C call takes several built values, they
/// are built right to left, as Mesa's compiler evaluates the arguments.
pub fn build(n: &mut Nir<'_>, key: Key) -> R<()> {
    let undef = n.new_instr(Kind::Undef, 4, 16)?;
    n.push(undef)?;
    let src0 = n.intr(Intr::BlendInput, 4, if key.colour_f32 { 32 } else { 16 }, &[])?;
    n.push(src0)?;
    let src1 = n.intr(Intr::BlendInputDual, 4, 32, &[])?;
    n.push(src1)?;
    /* nir_convert_with_rounding to f16 (pan_blend.c:847-851): none for an
     * fp16 colour; else one f2f16 of four, split */
    let src0 = if key.colour_f32 { pairs(n, Op::F2f16, src0)? } else { src0 };
    let src1h = pairs(n, Op::F2f16, src1)?;
    /* pan_blend.c:880-889: the sample, then load_tile_pan's arguments
     * right to left */
    let sample = if key.samples > 1 { n.intr(Intr::SampleId, 1, 32, &[])? } else { n.konst(32, &[0])? };
    n.push(sample)?;
    let conv = n.konst(32, &[0xed000])?;
    n.push(conv)?;
    let cov = n.intr(Intr::Coverage, 1, 32, &[])?;
    n.push(cov)?;
    let k1f = n.konst(32, &[0x1f])?;
    n.push(k1f)?;
    let cov_mask = n.alu(Op::Iand, 1, 32, &[(cov, ID), (k1f, ID)])?;
    n.push(cov_mask)?;
    /* pan_nir_tile_rt_sample (pan_nir.h:16-23): pack(u2u8(sample), u2u8(rt), 0, 255) */
    let rt = n.konst(32, &[0])?;
    n.push(rt)?;
    let kff = n.konst(8, &[0xff])?;
    n.push(kff)?;
    let k00 = n.konst(8, &[0])?;
    n.push(k00)?;
    let u_rt = n.alu(Op::U2u8, 1, 8, &[(rt, ID)])?;
    n.push(u_rt)?;
    let u_s = n.alu(Op::U2u8, 1, 8, &[(sample, ID)])?;
    n.push(u_s)?;
    let word = n.alu(Op::Pack32_4x8Split, 1, 32, &[(u_s, ID), (u_rt, ID), (k00, ID), (kff, ID)])?;
    n.push(word)?;
    let dest = n.intr(Intr::LoadTile, 4, 16, &[word, cov_mask, conv])?;
    n.push(dest)?;
    /* nir_color_blend: the constants (pan_lower_blend_constants), fp16 */
    let k0 = n.konst(32, &[0])?;
    n.push(k0)?;
    let k1 = n.konst(32, &[1])?;
    n.push(k1)?;
    let ubo = n.intr(Intr::LoadUbo, 4, 32, &[k1, k0])?;
    n.push(ubo)?;
    let bconst = pairs(n, Op::F2f16, ubo)?;
    /* the inputs clamped to the UNORM format */
    let src = pairs(n, Op::Fsat, src0)?;
    let bconst = pairs(n, Op::Fsat, bconst)?;
    let _src1 = pairs(n, Op::Fsat, src1h)?;
    let zero = n.konst(16, &[0])?;
    n.push(zero)?;
    let one = n.konst(16, &[ONE16])?;
    n.push(one)?;
    let _ = (zero, one);
    /* dst = vec4(channel 0..3), its arguments right to left */
    let mut m = [0u16; 4];
    for ch in (0..4).rev() {
        m[ch] = n.alu(Op::Mov, 1, 16, &[(dest, sw(ch as u8))])?;
        n.push(m[ch])?;
    }
    let dst = n.alu(Op::Vec4, 4, 16, &[(m[0], ID), (m[1], ID), (m[2], ID), (m[3], ID)])?;
    n.push(dst)?;
    let v = Vals { src, dst, bconst };
    let mut chans = [0u16; 4];
    for (ch, slot) in chans.iter_mut().enumerate() {
        let c = if ch < 3 { key.rgb } else { key.alpha };
        let mut psrc = channel(n, src, ch as u8)?;
        let mut pdst = channel(n, dst, ch as u8)?;
        if matches!(c.func, Func::Add | Func::Subtract | Func::ReverseSubtract) {
            psrc = blend_factor(n, &v, psrc, ch as u8, c.src)?;
            pdst = blend_factor(n, &v, pdst, ch as u8, c.dst)?;
        }
        let (op, a, b) = match c.func {
            Func::Add => (Op::Fadd, psrc, pdst),
            Func::Subtract => (Op::Fsub, psrc, pdst),
            Func::ReverseSubtract => (Op::Fsub, pdst, psrc),
            Func::Min => (Op::Fmin, psrc, pdst),
            Func::Max => (Op::Fmax, psrc, pdst),
        };
        *slot = n.alu(op, 1, 16, &[(a, ID), (b, ID)])?;
        n.push(*slot)?;
    }
    let mut color = n.alu(Op::Vec4, 4, 16, &[(chans[0], ID), (chans[1], ID), (chans[2], ID), (chans[3], ID)])?;
    n.push(color)?;
    if key.mask & 0xf != 0xf {
        /* nir_color_mask, its arguments right to left */
        let mut mm = [0u16; 4];
        for ch in (0..4).rev() {
            let from = if key.mask >> ch & 1 == 1 { color } else { dest };
            mm[ch] = n.alu(Op::Mov, 1, 16, &[(from, sw(ch as u8))])?;
            n.push(mm[ch])?;
        }
        color = n.alu(Op::Vec4, 4, 16, &[(mm[0], ID), (mm[1], ID), (mm[2], ID), (mm[3], ID)])?;
        n.push(color)?;
    }
    let desc = n.konst(64, &[0x000e_d000_0000_0019])?;
    n.push(desc)?;
    let cov2 = n.intr(Intr::Coverage, 1, 32, &[])?;
    n.push(cov2)?;
    let blend = n.intr(Intr::Blend, 0, 0, &[cov2, desc, color])?;
    n.push(blend)?;
    let ret = n.intr(Intr::BlendReturn, 0, 0, &[])?;
    n.push(ret)?;
    Ok(())
}

struct Vals {
    src: u16,
    dst: u16,
    bconst: u16,
}

/// A vec4 operation as `nir_lower_alu_width` leaves it: the op on .xy and
/// on .zw, then a vec4 of their lanes.
fn pairs(n: &mut Nir<'_>, op: Op, x: u16) -> R<u16> {
    let a = n.alu(op, 2, 16, &[(x, [0, 1, 0, 0])])?;
    n.push(a)?;
    let b = n.alu(op, 2, 16, &[(x, [2, 3, 0, 0])])?;
    n.push(b)?;
    let v = n.alu(Op::Vec4, 4, 16, &[(a, sw(0)), (a, sw(1)), (b, sw(0)), (b, sw(1))])?;
    n.push(v)
}

/// `nir_channel`: a mov of one component.
fn channel(n: &mut Nir<'_>, v: u16, c: u8) -> R<u16> {
    let m = n.alu(Op::Mov, 1, 16, &[(v, sw(c))])?;
    n.push(m)
}

fn one(n: &mut Nir<'_>) -> R<u16> {
    let k = n.konst(16, &[ONE16])?;
    n.push(k)
}

/// `nir_blend_factor_value` (nir_lower_blend.c:100-131).
fn factor_value(n: &mut Nir<'_>, v: &Vals, ch: u8, base: Base) -> R<u16> {
    match base {
        Base::One => one(n),
        Base::SrcColor => channel(n, v.src, ch),
        Base::DstColor => channel(n, v.dst, ch),
        Base::SrcAlpha => channel(n, v.src, 3),
        Base::DstAlpha => channel(n, v.dst, 3),
        Base::ConstColor => channel(n, v.bconst, ch),
        Base::ConstAlpha => channel(n, v.bconst, 3),
        Base::SrcAlphaSaturate => {
            /* nir_alpha_saturate (:84-96) */
            let asrc = channel(n, v.src, 3)?;
            let adst = channel(n, v.dst, 3)?;
            let o = one(n)?;
            let adsti = n.alu(Op::Fsub, 1, 16, &[(o, ID), (adst, ID)])?;
            n.push(adsti)?;
            if ch < 3 {
                let m = n.alu(Op::Fmin, 1, 16, &[(asrc, ID), (adsti, ID)])?;
                n.push(m)
            } else {
                Ok(o)
            }
        }
    }
}

/// `nir_blend_factor` (:179-196): `raw * f`, an inverted factor `1 - f` as
/// `fadd(fneg(f), 1.0)`.
fn blend_factor(n: &mut Nir<'_>, v: &Vals, raw: u16, ch: u8, f: Factor) -> R<u16> {
    let mut x = factor_value(n, v, ch, f.base)?;
    if f.invert {
        let neg = n.alu(Op::Fneg, 1, 16, &[(x, ID)])?;
        n.push(neg)?;
        let k = one(n)?;
        x = n.alu(Op::Fadd, 1, 16, &[(neg, ID), (k, ID)])?;
        n.push(x)?;
    }
    let m = n.alu(Op::Fmul, 1, 16, &[(raw, ID), (x, ID)])?;
    n.push(m)
}

// ---------------------------------------------------------------- copy propagation

impl Nir<'_> {
    /// `is_swizzleless_move` (nir_opt_copy_propagate.c:35-57).
    fn is_swizzleless_move(&self, c: u16) -> bool {
        let i = &self.ins[c as usize];
        let n = i.comps;
        if self.ins[i.srcs[0].def as usize].comps != n {
            return false;
        }
        if i.op() == Some(Op::Mov) {
            (0..n as usize).all(|k| i.srcs[0].swz[k] == k as u8)
        } else {
            (0..n as usize).all(|k| i.srcs[k].swz[0] == k as u8 && i.srcs[k].def == i.srcs[0].def)
        }
    }

    /// `merge_vec_and_mov` (:59-74).
    fn merge_vec_and_mov(&mut self, mov: u16, vec: u16) -> R<()> {
        let n = self.ins[mov as usize].comps;
        let new = self.new_instr(Kind::Alu(Op::vec(n)), n, self.ins[mov as usize].bits)?;
        for k in 0..n as usize {
            let s = self.ins[vec as usize].srcs[self.ins[mov as usize].srcs[0].swz[k] as usize];
            self.set_src(new, k, s.def, s.swz);
        }
        let at = self.pos(mov)? + 1;
        self.insert(new, at)?;
        self.rewrite_uses(mov, new);
        Ok(())
    }

    /// `copy_propagate_alu` (:76-138).
    fn copy_propagate_alu(&mut self, u: Use, copy: u16) -> R<bool> {
        let n = self.src_comps(u.user, u.src as usize) as usize;
        let c = self.ins[copy as usize];
        let mut swz = self.ins[u.user as usize].srcs[u.src as usize].swz;
        let new = if c.op() == Some(Op::Mov) {
            for k in 0..n {
                swz[k] = c.srcs[0].swz[swz[k] as usize];
            }
            c.srcs[0].def
        } else {
            let new = c.srcs[swz[0] as usize].def;
            for k in 1..n {
                if c.srcs[swz[k] as usize].def != new {
                    if self.alu_op(u.user) == Some(Op::Mov) {
                        self.merge_vec_and_mov(u.user, copy)?;
                        return Ok(true);
                    }
                    return Ok(false);
                }
            }
            for k in 0..n {
                swz[k] = c.srcs[swz[k] as usize].swz[0];
            }
            new
        };
        self.ins[u.user as usize].srcs[u.src as usize].swz = swz;
        self.rewrite_src(u, new);
        Ok(true)
    }

    /// `nir_opt_copy_prop` (:140-181).
    pub fn copy_prop(&mut self) -> R<bool> {
        let mut progress = false;
        let mut k = 0;
        while k < self.len {
            let c = self.order[k];
            let next = self.order.get(k + 1).copied().filter(|_| k + 1 < self.len);
            if self.ins[c as usize].op().is_some_and(Op::is_vec_or_mov) {
                let mut p = false;
                let mut u = [Use { user: 0, src: 0 }; MAX_USES];
                let n = self.uses(c, &mut u);
                for x in &u[..n] {
                    if self.is_alu(x.user) {
                        p |= self.copy_propagate_alu(*x, c)?;
                    } else if self.is_swizzleless_move(c) {
                        let d = self.ins[c as usize].srcs[0].def;
                        self.rewrite_src(*x, d);
                        p = true;
                    }
                }
                if p && !self.has_uses(c) {
                    self.remove(c)?;
                }
                progress |= p;
            }
            k = match next {
                Some(nx) => self.pos(nx)?,
                None => self.len,
            };
        }
        Ok(progress)
    }

    // ------------------------------------------------------------ dce

    fn side_effect(&self, i: u16) -> bool {
        matches!(self.ins[i as usize].kind, Kind::Intr(Intr::Blend | Intr::BlendReturn))
    }

    /// `nir_opt_dce` on one block (nir_opt_dce.c:96-131).
    pub fn dce(&mut self) -> R<bool> {
        let mut live = [false; MAX];
        let mut progress = false;
        let mut k = self.len;
        while k > 0 {
            k -= 1;
            let i = self.order[k];
            if self.side_effect(i) || live[i as usize] {
                let ins = self.ins[i as usize];
                for s in &ins.srcs[..ins.n as usize] {
                    live[s.def as usize] = true;
                }
            } else {
                self.remove(i)?;
                progress = true;
            }
        }
        Ok(progress)
    }

    // ------------------------------------------------------------ cse

    /// `nir_alu_srcs_equal` (nir_instr_set.c:570-586).
    fn alu_srcs_equal(&self, a: u16, b: u16, s1: usize, s2: usize) -> bool {
        let n = self.src_comps(a, s1) as usize;
        let x = &self.ins[a as usize].srcs[s1];
        let y = &self.ins[b as usize].srcs[s2];
        x.swz[..n] == y.swz[..n] && x.def == y.def
    }

    /// `nir_instrs_equal` (:588-640) for the blend shader's instructions.
    fn instrs_equal(&self, a: u16, b: u16) -> bool {
        let (x, y) = (&self.ins[a as usize], &self.ins[b as usize]);
        if x.kind != y.kind || x.comps != y.comps || x.bits != y.bits {
            return false;
        }
        match x.kind {
            Kind::Const => x.k[..x.comps as usize] == y.k[..y.comps as usize],
            Kind::Intr(_) => x.n == y.n && (0..x.n as usize).all(|k| x.srcs[k].def == y.srcs[k].def),
            Kind::Alu(op) => {
                let n = op.num_inputs();
                if op.commutative() {
                    if !((self.alu_srcs_equal(a, b, 0, 0) && self.alu_srcs_equal(a, b, 1, 1))
                        || (self.alu_srcs_equal(a, b, 0, 1) && self.alu_srcs_equal(a, b, 1, 0)))
                    {
                        return false;
                    }
                    (2..n).all(|k| self.alu_srcs_equal(a, b, k, k))
                } else {
                    (0..n).all(|k| self.alu_srcs_equal(a, b, k, k))
                }
            }
            Kind::Undef => false,
        }
    }

    /// `nir_opt_cse` (nir_opt_cse.c:41-70): the coverage and UBO loads are
    /// reorderable intrinsics; the tile and input loads appear once.
    pub fn cse(&mut self) -> R<bool> {
        let mut seen = [0u16; MAX];
        let mut ns = 0;
        let mut progress = false;
        let mut k = 0;
        while k < self.len {
            let i = self.order[k];
            let can = match self.ins[i as usize].kind {
                Kind::Alu(_) | Kind::Const => true,
                Kind::Intr(t) => matches!(t, Intr::Coverage | Intr::LoadUbo | Intr::SampleId),
                Kind::Undef => false,
            };
            if !can {
                k += 1;
                continue;
            }
            match seen[..ns].iter().copied().find(|&m| self.instrs_equal(m, i)) {
                Some(m) => {
                    self.rewrite_uses(i, m);
                    self.remove(i)?;
                    progress = true;
                }
                None => {
                    seen[ns] = i;
                    ns += 1;
                    k += 1;
                }
            }
        }
        Ok(progress)
    }

    // ------------------------------------------------------------ vectorize

    /// `bi_vectorize_filter` (bifrost_nir.c:107-178) for the blend shader's
    /// opcodes.
    fn vectorize_filter(&self, i: u16) -> u8 {
        let ins = &self.ins[i as usize];
        match ins.kind {
            Kind::Alu(Op::F2f16) => 2,
            Kind::Alu(_) => {
                let bits = ins.bits.max(self.ins[ins.srcs[0].def as usize].bits);
                (32 / bits).max(1)
            }
            _ => 0,
        }
    }

    /// `instr_can_rewrite` (nir_opt_vectorize.c:268-307).
    fn vec_can_rewrite(&self, i: u16, width: u8) -> bool {
        let ins = &self.ins[i as usize];
        let Kind::Alu(op) = ins.kind else { return false };
        if op == Op::Mov || ins.comps >= width || op.output_size() != 0 || op.input_size() != 0 {
            return false;
        }
        let mask = !(width - 1);
        ins.srcs[..ins.n as usize].iter().all(|s| (1..ins.comps as usize).all(|j| s.swz[0] & mask == s.swz[j] & mask))
    }

    /// `instrs_equal` of nir_opt_vectorize.c (:237-266).
    fn vec_equal(&self, a: u16, b: u16, width: u8) -> bool {
        let (x, y) = (&self.ins[a as usize], &self.ins[b as usize]);
        if x.kind != y.kind || x.bits != y.bits {
            return false;
        }
        let mask = !(width - 1);
        (0..x.n as usize).all(|k| {
            let (s, t) = (&x.srcs[k], &y.srcs[k]);
            s.swz[0] & mask == t.swz[0] & mask && (s.def == t.def || (self.is_const(s.def) && self.is_const(t.def)))
        })
    }

    /// `instr_try_combine_alu` (:545-617): one instruction of both after
    /// `a1`, their uses rewritten (`rewrite_uses`, :309-368).
    fn vec_try_combine(&mut self, a1: u16, a2: u16, width: u8) -> R<Option<u16>> {
        let (x, y) = (self.ins[a1 as usize], self.ins[a2 as usize]);
        let (n1, n2) = (x.comps, y.comps);
        let tot = n1 + n2;
        if tot > width {
            return Ok(None);
        }
        let new = self.new_instr(x.kind, tot, x.bits)?;
        let mut at = self.pos(a1)? + 1;
        for k in 0..x.n as usize {
            let (s1, s2) = (x.srcs[k], y.srcs[k]);
            let mut swz = [0u8; 4];
            if s1.def != s2.def {
                let (c1, c2) = (self.ins[s1.def as usize], self.ins[s2.def as usize]);
                let mut vals = [0u64; 4];
                for j in 0..tot as usize {
                    vals[j] =
                        if j < n1 as usize { c1.k[s1.swz[j] as usize] } else { c2.k[s2.swz[j - n1 as usize] as usize] };
                }
                let kk = self.konst(c1.bits, &vals[..tot as usize])?;
                self.insert(kk, at)?;
                at += 1;
                self.set_src(new, k, kk, ID);
                continue;
            }
            let (n1, n2) = (n1 as usize, n2 as usize);
            swz[..n1].copy_from_slice(&s1.swz[..n1]);
            swz[n1..n1 + n2].copy_from_slice(&s2.swz[..n2]);
            self.set_src(new, k, s1.def, swz);
        }
        self.insert(new, at)?;
        at += 1;
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let n = self.uses(a1, &mut u);
        for x in &u[..n] {
            if self.is_alu(x.user) {
                self.rewrite_src(*x, new);
            }
        }
        let n = self.uses(a2, &mut u);
        for x in &u[..n] {
            if self.is_alu(x.user) {
                self.rewrite_src(*x, new);
                let c = self.src_comps(x.user, x.src as usize) as usize;
                for s in &mut self.ins[x.user as usize].srcs[x.src as usize].swz[..c] {
                    *s += n1;
                }
            }
        }
        for (d, base, nd) in [(a1, 0u8, n1), (a2, n1, n2)] {
            if self.has_uses(d) {
                /* nir_swizzle: the value itself, or a mov of its lanes */
                let mut swz = [0u8; 4];
                for j in 0..nd {
                    swz[j as usize] = base + j;
                }
                let mv = if nd == tot && base == 0 {
                    new
                } else {
                    let m = self.alu(Op::Mov, nd, x.bits, &[(new, swz)])?;
                    self.insert(m, at)?;
                    at += 1;
                    m
                };
                self.rewrite_uses(d, mv);
            }
        }
        self.remove(a1)?;
        self.remove(a2)?;
        Ok(Some(new))
    }

    /// `nir_opt_vectorize` (:672-707) with `bi_vectorize_filter`.
    pub fn vectorize(&mut self) -> R<bool> {
        let mut seen = [0u16; MAX];
        let mut ns = 0;
        let mut progress = false;
        let mut k = 0;
        while k < self.len {
            let i = self.order[k];
            let next = (k + 1 < self.len).then(|| self.order[k + 1]);
            let w = self.vectorize_filter(i);
            self.ins[i as usize].pass_flag = w;
            if self.vec_can_rewrite(i, w) {
                let found =
                    seen[..ns].iter().position(|&m| self.ins[m as usize].pass_flag == w && self.vec_equal(m, i, w));
                let mut combined = false;
                if let Some(f) = found {
                    let m = seen[f];
                    seen.copy_within(f + 1..ns, f);
                    ns -= 1;
                    if let Some(new) = self.vec_try_combine(m, i, w)? {
                        self.ins[new as usize].pass_flag = w;
                        if self.vec_can_rewrite(new, w) {
                            seen[ns] = new;
                            ns += 1;
                        }
                        progress = true;
                        combined = true;
                    }
                }
                if !combined {
                    seen[ns] = i;
                    ns += 1;
                }
            }
            k = match next {
                Some(nx) => self.pos(nx)?,
                None => self.len,
            };
        }
        Ok(progress)
    }
}

// ---------------------------------------------------------------- constants

/// A constant component as a float (`nir_const_value_as_float`).
fn const_float(bits: u8, v: u64) -> Option<f64> {
    match bits {
        16 => Some(f32::from_bits(super::fau::half_to_float(v as u16)) as f64),
        32 => Some(f32::from_bits(v as u32) as f64),
        64 => Some(f64::from_bits(v)),
        _ => None,
    }
}

/// A float as a constant of `bits` (`nir_imm_floatN_t`, `_mesa_float_to_half`).
fn float_bits(f: f64, bits: u8) -> R<u64> {
    match bits {
        16 => {
            let b = (f as f32).to_bits();
            /* a zero keeps its sign; any other value fp16 cannot hold
             * exactly or round to a normal is not a blend constant */
            if b & 0x7fff_ffff == 0 {
                return Ok(((b >> 16) & 0x8000) as u64);
            }
            f16_rtne(b).map(u64::from).ok_or(MaliError::BlendVariantNotLowered)
        }
        32 => Ok((f as f32).to_bits() as u64),
        64 => Ok(f.to_bits()),
        _ => Err(MaliError::BlendVariantNotLowered),
    }
}

/// `nir_eval_const_opcode` for one component of the blend shader's opcodes
/// (float ones computed in fp32 and rounded to the destination, as Mesa's
/// generated constant expressions do for fp16).
fn eval(op: Op, bits: u8, src_bits: u8, a: &[u64]) -> R<u64> {
    let f = |k: usize| const_float(src_bits, a[k]).ok_or(MaliError::BlendVariantNotLowered);
    let r = |x: f32| float_bits(x as f64, bits);
    match op {
        Op::Fadd => r(f(0)? as f32 + f(1)? as f32),
        Op::Fsub => r(f(0)? as f32 - f(1)? as f32),
        Op::Fmul => r(f(0)? as f32 * f(1)? as f32),
        /* the product of two fp16 values is exact in f64: one rounding */
        Op::Ffma => r((f(0)? * f(1)? + f(2)?) as f32),
        Op::Fneg => r(-(f(0)? as f32)),
        Op::Fcanonicalize => r(f(0)? as f32),
        Op::Fsat => r((f(0)? as f32).clamp(0.0, 1.0)),
        Op::Fmin => r((f(0)? as f32).min(f(1)? as f32)),
        Op::Fmax => r((f(0)? as f32).max(f(1)? as f32)),
        Op::F2f16 => r(f(0)? as f32),
        Op::Iand => Ok(a[0] & a[1]),
        Op::U2u8 => Ok(a[0] & 0xff),
        Op::Pack32_4x8Split => Ok((a[0] & 0xff) | (a[1] & 0xff) << 8 | (a[2] & 0xff) << 16 | (a[3] & 0xff) << 24),
        Op::Mov | Op::Vec2 | Op::Vec3 | Op::Vec4 => Ok(a[0]),
        _ => Err(MaliError::BlendVariantNotLowered),
    }
}

impl Nir<'_> {
    /// `nir_try_constant_fold_alu` (nir_opt_constant_folding.c:79-162):
    /// the components of `i` when every source is a constant.
    fn try_fold(&self, i: u16) -> R<Option<[u64; 4]>> {
        let ins = &self.ins[i as usize];
        let Kind::Alu(op) = ins.kind else { return Ok(None) };
        if !ins.srcs[..ins.n as usize].iter().all(|s| self.is_const(s.def)) {
            return Ok(None);
        }
        let mut out = [0u64; 4];
        if op.is_vec() {
            for k in 0..ins.n as usize {
                let s = ins.srcs[k];
                out[k] = self.ins[s.def as usize].k[s.swz[0] as usize];
            }
            return Ok(Some(out));
        }
        let src_bits = self.ins[ins.srcs[0].def as usize].bits;
        let per_lane = op.input_size() == 0;
        for c in 0..ins.comps as usize {
            let mut a = [0u64; 4];
            for k in 0..ins.n as usize {
                let s = ins.srcs[k];
                a[k] = self.ins[s.def as usize].k[s.swz[if per_lane { c } else { 0 }] as usize];
            }
            out[c] = eval(op, ins.bits, src_bits, &a[..ins.n as usize])?;
        }
        Ok(Some(out))
    }

    /// `nir_instr_free_and_dce` (nir.c:1464-1501): `i` out, then every
    /// value it read that nothing reads any more, in that order.
    fn free_and_dce(&mut self, i: u16) -> R<()> {
        let mut wl = [0u16; MAX];
        let (mut head, mut tail) = (0, 0);
        let mut x = i;
        loop {
            let ins = self.ins[x as usize];
            /* the sources leave their lists first */
            self.ins[x as usize].n = 0;
            if self.ins[x as usize].live {
                self.remove(x)?;
            }
            for s in &ins.srcs[..ins.n as usize] {
                let d = s.def;
                if !self.side_effect(d)
                    && !self.has_uses(d)
                    && self.ins[d as usize].live
                    && !wl[head..tail].contains(&d)
                {
                    wl[tail] = d;
                    tail += 1;
                }
            }
            if head == tail {
                return Ok(());
            }
            x = wl[head];
            head += 1;
        }
    }

    /// `nir_function_impl_lower_instructions` (nir.c:2309-2383): each
    /// instruction `lower` replaces gets the new value's uses (in their
    /// order) and is freed with the values only it read; the walk goes on
    /// at the instructions `lower` put after it.
    fn lower_instructions(&mut self, lower: fn(&mut Nir<'_>, u16, &mut usize) -> R<Option<u16>>) -> R<bool> {
        let mut progress = false;
        let mut k = 0;
        while k < self.len {
            let i = self.order[k];
            let mut u = [Use { user: 0, src: 0 }; MAX_USES];
            let nu = self.uses(i, &mut u);
            let mut at = k + 1;
            match lower(self, i, &mut at)? {
                Some(new) => {
                    for x in &u[..nu] {
                        self.rewrite_src(*x, new);
                    }
                    let p = self.pos(i)?;
                    let next = (p + 1 < self.len).then(|| self.order[p + 1]);
                    if !self.has_uses(i) {
                        self.free_and_dce(i)?;
                        k = match next {
                            Some(nx) => self.pos(nx)?,
                            None => self.len,
                        };
                    } else {
                        k = p + 1;
                    }
                    progress = true;
                }
                None => k = self.pos(i)? + 1,
            }
        }
        Ok(progress)
    }

    /// `nir_lower_alu_width` (nir_lower_alu_width.c:203-416) with
    /// `bi_vectorize_filter`.
    pub fn lower_alu_width(&mut self) -> R<bool> {
        fn lower(n: &mut Nir<'_>, i: u16, at: &mut usize) -> R<Option<u16>> {
            let ins = n.ins[i as usize];
            let Kind::Alu(op) = ins.kind else { return Ok(None) };
            if ins.comps <= 1 || op.is_vec() {
                return Ok(None);
            }
            let mut w = n.vectorize_filter(i);
            if w == 0 {
                return Ok(None);
            }
            let c = ins.comps;
            if c <= w {
                let mask = !(w - 1);
                let ok = ins.srcs[..ins.n as usize]
                    .iter()
                    .all(|s| (1..c as usize).all(|j| s.swz[0] & mask == s.swz[j] & mask));
                if ok {
                    return Ok(None);
                }
                w = c.div_ceil(2);
            }
            let vec = n.new_instr(Kind::Alu(Op::vec(c)), c, ins.bits)?;
            let mut ch = 0;
            let mut vk = 0;
            while ch < c {
                let m = w.min(c - ch);
                let lo = n.new_instr(ins.kind, m, ins.bits)?;
                for k in 0..ins.n as usize {
                    let mut swz = [0u8; 4];
                    for j in 0..m as usize {
                        swz[j] = ins.srcs[k].swz[ch as usize + j];
                    }
                    n.set_src(lo, k, ins.srcs[k].def, swz);
                }
                n.insert(lo, *at)?;
                *at += 1;
                for j in 0..m {
                    n.set_src(vec, vk, lo, sw(j));
                    vk += 1;
                }
                ch += m;
            }
            n.insert(vec, *at)?;
            *at += 1;
            Ok(Some(vec))
        }
        self.lower_instructions(lower)
    }

    /// `nir_opt_constant_folding` (nir_opt_constant_folding.c:612-633): a
    /// constant after each instruction whose sources all are.
    pub fn constant_folding(&mut self) -> R<bool> {
        fn lower(n: &mut Nir<'_>, i: u16, at: &mut usize) -> R<Option<u16>> {
            let Some(vals) = n.try_fold(i)? else { return Ok(None) };
            let ins = n.ins[i as usize];
            let k = n.konst(ins.bits, &vals[..ins.comps as usize])?;
            n.insert(k, *at)?;
            *at += 1;
            Ok(Some(k))
        }
        self.lower_instructions(lower)
    }

    // ------------------------------------------------------------ move

    /// `nir_opt_move` (nir_opt_move.c:28-120) with the move options Mesa's
    /// bifrost passes give it: constants, undefs, movs and vecs and the UBO
    /// load move down to their first user.
    pub fn opt_move(&mut self) -> R<bool> {
        /* an instruction the backward walk has not reached matches no index */
        let mut index = [u32::MAX; MAX];
        let mut progress = false;
        let mut last: Option<u16> = None;
        /* the walk is backwards over the instructions as they were */
        let mut walk = [0u16; MAX];
        let n = self.len;
        walk[..n].copy_from_slice(&self.order[..n]);
        for (next_index, w) in (1u32..).zip((0..n).rev()) {
            let i = walk[w];
            index[i as usize] = next_index;
            let movable = match self.ins[i as usize].kind {
                Kind::Const | Kind::Undef => true,
                Kind::Alu(op) => op.is_vec_or_mov(),
                Kind::Intr(t) => t == Intr::LoadUbo,
            };
            if !movable {
                continue;
            }
            let mut first = last;
            let mut u = [Use { user: 0, src: 0 }; MAX_USES];
            let nu = self.uses(i, &mut u);
            for x in &u[..nu] {
                if first.is_none_or(|f| index[x.user as usize] > index[f as usize]) {
                    first = Some(x.user);
                }
            }
            if let Some(mut f) = first {
                let mut p = self.pos(f)?;
                while p > 0 && index[self.order[p - 1] as usize] == index[f as usize] {
                    p -= 1;
                    f = self.order[p];
                }
                if p > 0 && self.order[p - 1] == i {
                    continue;
                }
                let q = self.pos(i)?;
                self.order.copy_within(q + 1..self.len, q);
                self.len -= 1;
                index[i as usize] = index[f as usize];
                let p = self.pos(f)?;
                self.order.copy_within(p..self.len, p + 1);
                self.order[p] = i;
                self.len += 1;
                progress = true;
                continue;
            }
            if self.order[self.len - 1] == i {
                continue;
            }
            let q = self.pos(i)?;
            self.order.copy_within(q + 1..self.len, q);
            self.order[self.len - 1] = i;
            index[i as usize] = 0;
            last = Some(i);
            progress = true;
        }
        Ok(progress)
    }

    // ------------------------------------------------------------ shrink

    /// `nir_src_components_read` of one use.
    fn read_mask(&self, x: Use) -> u8 {
        let user = &self.ins[x.user as usize];
        let s = &user.srcs[x.src as usize];
        if !matches!(user.kind, Kind::Alu(_)) {
            return ((1u16 << self.ins[s.def as usize].comps) - 1) as u8;
        }
        let n = self.src_comps(x.user, x.src as usize) as usize;
        s.swz[..n].iter().fold(0, |m, &c| m | 1 << c)
    }

    fn reswizzle_uses(&mut self, d: u16, re: &[u8; 4]) {
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let n = self.uses(d, &mut u);
        for x in &u[..n] {
            let s = &mut self.ins[x.user as usize].srcs[x.src as usize];
            for c in s.swz.iter_mut() {
                *c = re[*c as usize & 3];
            }
        }
    }

    /// `nir_scalar_resolved`: through movs and vecs.
    fn resolved(&self, mut d: u16, mut c: u8) -> (u16, u8) {
        loop {
            let ins = &self.ins[d as usize];
            match ins.kind {
                Kind::Alu(Op::Mov) => {
                    let s = ins.srcs[0];
                    (d, c) = (s.def, s.swz[c as usize]);
                }
                Kind::Alu(op) if op.is_vec() => {
                    let s = ins.srcs[c as usize];
                    (d, c) = (s.def, s.swz[0]);
                }
                _ => return (d, c),
            }
        }
    }

    /// `create_smaller_vec` (nir_opt_shrink_vectors.c:154-209).
    fn create_smaller_vec(&mut self, vec: u16, mask: u8) -> R<bool> {
        if mask & mask.wrapping_sub(1) == 0 {
            return Ok(false);
        }
        let v = self.ins[vec as usize];
        let mut re = [0u8; 4];
        let mut srcs = [(0u16, 0u8); 4];
        let mut n = 0usize;
        for i in 0..v.comps as usize {
            if mask >> i & 1 == 0 {
                continue;
            }
            let sc = self.resolved(vec, i as u8);
            match srcs[..n].iter().position(|&x| x == sc) {
                Some(j) => re[i] = j as u8,
                None => {
                    srcs[n] = sc;
                    re[i] = n as u8;
                    n += 1;
                }
            }
        }
        if n == v.comps as usize {
            return Ok(false);
        }
        let new = self.new_instr(Kind::Alu(Op::vec(n as u8)), n as u8, v.bits)?;
        if n == 1 {
            self.set_src(new, 0, srcs[0].0, sw(srcs[0].1));
        } else {
            for (k, &(d, c)) in srcs[..n].iter().enumerate() {
                self.set_src(new, k, d, sw(c));
            }
        }
        let at = self.pos(vec)?;
        self.insert(new, at)?;
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let nu = self.uses(vec, &mut u);
        for x in &u[..nu] {
            if self.read_mask(*x) & mask != 0 {
                self.rewrite_src(*x, new);
            }
        }
        self.reswizzle_uses(new, &re);
        Ok(true)
    }

    /// `opt_shrink_or_split_vector` (:211-256).
    fn shrink_vec(&mut self, vec: u16) -> R<bool> {
        let mut masks = [0u8; 4];
        let mut nm = 0usize;
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let nu = self.uses(vec, &mut u);
        for x in &u[..nu] {
            if !self.is_alu(x.user) {
                return Ok(false);
            }
            let read = self.read_mask(*x);
            match masks[..nm].iter().position(|&m| m & read != 0) {
                Some(i) => {
                    /* merge the masks it overlaps (:224-236) */
                    masks[i] |= read;
                    let mut k = i + 1;
                    for j in i + 1..nm {
                        if masks[i] & masks[j] != 0 {
                            masks[i] |= masks[j];
                        } else {
                            masks[k] = masks[j];
                            k += 1;
                        }
                    }
                    nm = k;
                }
                None => {
                    masks[nm] = read;
                    nm += 1;
                }
            }
        }
        let mut p = false;
        for &m in &masks[..nm] {
            p |= self.create_smaller_vec(vec, m)?;
        }
        Ok(p)
    }

    /// `opt_shrink_vectors_alu` (:258-326) and `_load_const` (:390-438).
    fn shrink_value(&mut self, i: u16) -> R<bool> {
        let ins = self.ins[i as usize];
        if ins.comps == 1 {
            return Ok(false);
        }
        if let Kind::Alu(op) = ins.kind {
            if op.is_vec() {
                return self.shrink_vec(i);
            }
            if op.output_size() != 0 {
                return Ok(false);
            }
        }
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let nu = self.uses(i, &mut u);
        if u[..nu].iter().any(|x| !self.is_alu(x.user)) {
            return Ok(false);
        }
        let mask = u[..nu].iter().fold(0u8, |m, x| m | self.read_mask(*x));
        if mask == 0 {
            return Ok(false);
        }
        let mut re = [0u8; 4];
        let mut n = 0usize;
        let mut progress = false;
        let is_const = ins.kind == Kind::Const;
        for c in 0..ins.comps as usize {
            if mask >> c & 1 == 0 {
                continue;
            }
            let x = &self.ins[i as usize];
            let dup = (0..n).find(|&j| {
                if is_const {
                    x.k[c] == x.k[j]
                } else {
                    let op = x.op().unwrap_or(Op::Other);
                    (0..x.n as usize).all(|s| op.input_size() == 0 && x.srcs[s].swz[c] == x.srcs[s].swz[j])
                }
            });
            match dup {
                Some(j) => {
                    re[c] = j as u8;
                    progress = true;
                }
                None => {
                    let x = &mut self.ins[i as usize];
                    if is_const {
                        x.k[n] = x.k[c];
                    } else {
                        for s in 0..x.n as usize {
                            x.srcs[s].swz[n] = x.srcs[s].swz[c];
                        }
                    }
                    if c != n {
                        progress = true;
                    }
                    re[c] = n as u8;
                    n += 1;
                }
            }
        }
        if progress {
            self.reswizzle_uses(i, &re);
        }
        if (n as u8) < ins.comps {
            progress = true;
        }
        self.ins[i as usize].comps = n as u8;
        Ok(progress)
    }

    /// `nir_opt_shrink_vectors` (:575-593): the instructions backwards.
    pub fn shrink_vectors(&mut self) -> R<bool> {
        let mut progress = false;
        let mut walk = [0u16; MAX];
        let n = self.len;
        walk[..n].copy_from_slice(&self.order[..n]);
        for &i in walk[..n].iter().rev() {
            if !self.ins[i as usize].live {
                continue;
            }
            if matches!(self.ins[i as usize].kind, Kind::Alu(_) | Kind::Const) {
                progress |= self.shrink_value(i)?;
            }
        }
        Ok(progress)
    }

    /// `nir_lower_load_const_to_scalar` (nir_lower_load_const_to_scalar.c).
    pub fn load_const_to_scalar(&mut self) -> R<bool> {
        let mut progress = false;
        let mut k = 0;
        while k < self.len {
            let i = self.order[k];
            let ins = self.ins[i as usize];
            if ins.kind != Kind::Const || ins.comps == 1 {
                k += 1;
                continue;
            }
            let mut at = k;
            let vec = self.new_instr(Kind::Alu(Op::vec(ins.comps)), ins.comps, ins.bits)?;
            for c in 0..ins.comps as usize {
                let kk = self.konst(ins.bits, &[ins.k[c]])?;
                self.insert(kk, at)?;
                at += 1;
                self.set_src(vec, c, kk, ID);
            }
            self.insert(vec, at)?;
            self.rewrite_uses(i, vec);
            self.remove(i)?;
            progress = true;
            k = at + 1;
        }
        Ok(progress)
    }
}

// ---------------------------------------------------------------- fp classes

/// `fp_class_bit` (nir_range_analysis.h:44-62).
mod fc {
    pub const NEG_INF: u16 = 1 << 0;
    pub const LT_NEG_ONE: u16 = 1 << 1;
    pub const NEG_ONE: u16 = 1 << 2;
    pub const LT_ZERO_GT_NEG_ONE: u16 = 1 << 3;
    pub const NEG_ZERO: u16 = 1 << 4;
    pub const POS_ZERO: u16 = 1 << 5;
    pub const GT_ZERO_LT_POS_ONE: u16 = 1 << 6;
    pub const POS_ONE: u16 = 1 << 7;
    pub const GT_POS_ONE: u16 = 1 << 8;
    pub const POS_INF: u16 = 1 << 9;
    pub const NAN: u16 = 1 << 10;
    pub const NON_INTEGRAL: u16 = 1 << 11;
    pub const UNKNOWN: u16 = (1 << 12) - 1;
    pub const ANY_ZERO: u16 = NEG_ZERO | POS_ZERO;
    pub const ANY_INF: u16 = NEG_INF | POS_INF;
    pub const ANY_NEG_FINITE: u16 = LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE;
    pub const ANY_NEG: u16 = ANY_NEG_FINITE | NEG_INF;
    pub const ANY_POS_FINITE: u16 = GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE;
    pub const ANY_POS: u16 = ANY_POS_FINITE | POS_INF;
    pub const ANY_FINITE: u16 = ANY_NEG_FINITE | ANY_ZERO | ANY_POS_FINITE;
    pub const FRACTIONS: u16 = LT_NEG_ONE | LT_ZERO_GT_NEG_ONE | GT_ZERO_LT_POS_ONE | GT_POS_ONE;
}

/// No class yet (a class is 12 bits).
const NO_CLASS: u16 = u16::MAX;
/// The deepest [`Nir::fp_class`] recursion before the key is refused
/// (`BlendVariantNotLowered`). Measured 2026-11-15 over the 4203 keys of
/// blend_deqp.words, blend_random.sha and blend_u32.words: depth 3 at
/// most (a query and three values below it); 16 leaves room for other
/// keys and bounds the stack at 17 frames, not the block's [`MAX`].
const FP_CLASS_DEPTH: u32 = 16;

/// `analyze_fp_constant` (nir_range_analysis.c:117-172).
fn const_class(bits: u8, vals: &[u64]) -> u16 {
    let mut r = 0;
    for &v in vals {
        let Some(f) = const_float(bits, v) else { return fc::UNKNOWN };
        if !f.is_nan() && !f.is_infinite() && libm_floor(f) != f {
            r |= fc::NON_INTEGRAL;
        }
        r |= if f.is_nan() {
            fc::NAN
        } else if f == f64::NEG_INFINITY {
            fc::NEG_INF
        } else if f < -1.0 {
            fc::LT_NEG_ONE
        } else if f == -1.0 {
            fc::NEG_ONE
        } else if f < 0.0 {
            fc::LT_ZERO_GT_NEG_ONE
        } else if f.to_bits() == 0 {
            fc::POS_ZERO
        } else if f == 0.0 {
            fc::NEG_ZERO
        } else if f < 1.0 {
            fc::GT_ZERO_LT_POS_ONE
        } else if f == 1.0 {
            fc::POS_ONE
        } else if f < f64::INFINITY {
            fc::GT_POS_ONE
        } else {
            fc::POS_INF
        };
        if f != 0.0 && !f.is_nan() {
            let min = match bits {
                16 => 1.0 / 16384.0,
                32 => f32::MIN_POSITIVE as f64,
                _ => f64::MIN_POSITIVE,
            };
            if f.abs() < min {
                r |= if f < 0.0 { fc::NEG_ZERO } else { fc::POS_ZERO };
            }
        }
    }
    r
}

/// `floor` without libm, for the integral test of a constant.
fn libm_floor(f: f64) -> f64 {
    if f.abs() >= 4503599627370496.0 {
        return f;
    }
    let t = f as i64 as f64;
    if t > f {
        t - 1.0
    } else {
        t
    }
}

/// `fneg_fp_class` (:213-235).
fn fneg_class(src: u16) -> u16 {
    use fc::*;
    let mut r = src & (NAN | NON_INTEGRAL);
    for (n, p) in [
        (NEG_INF, POS_INF),
        (LT_NEG_ONE, GT_POS_ONE),
        (NEG_ONE, POS_ONE),
        (LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE),
        (NEG_ZERO, POS_ZERO),
    ] {
        if src & p != 0 {
            r |= n;
        }
        if src & n != 0 {
            r |= p;
        }
    }
    r
}

/// Sets `r` from a table of (left class, right class, result) rows.
fn table(left: u16, right: u16, rows: &[(u16, u16, u16)]) -> u16 {
    rows.iter().filter(|&&(l, rr, _)| left & l != 0 && right & rr != 0).fold(0, |a, &(_, _, x)| a | x)
}

/// `fmul_fp_class` (:237-433), `mulz` false (the blend shader has no fmulz).
fn fmul_class(left: u16, right: u16, src_eq: bool, src_neg_eq: bool) -> u16 {
    use fc::*;
    if left == UNKNOWN && right == UNKNOWN && !src_eq && !src_neg_eq {
        return UNKNOWN;
    }
    let mut r = 0;
    if left & NAN != 0 {
        if right & ANY_ZERO != 0 {
            r |= NAN;
        }
        if right & (ANY_NEG | ANY_POS | NAN) != 0 {
            r |= NAN;
        }
    }
    for (inf, same, other) in [(NEG_INF, POS_INF, NEG_INF), (POS_INF, POS_INF, NEG_INF)] {
        if left & inf != 0 {
            if right & ANY_ZERO != 0 {
                r |= NAN;
            }
            /* -inf * neg = +inf, -inf * pos = -inf; +inf * pos = +inf, +inf * neg = -inf */
            if inf == NEG_INF {
                if right & ANY_NEG != 0 {
                    r |= same;
                }
                if right & ANY_POS != 0 {
                    r |= other;
                }
            } else {
                if right & ANY_POS != 0 {
                    r |= same;
                }
                if right & ANY_NEG != 0 {
                    r |= other;
                }
            }
            if right & NAN != 0 {
                r |= NAN;
            }
        }
    }
    if left & ANY_NEG_FINITE != 0 {
        if right & POS_ZERO != 0 {
            r |= NEG_ZERO;
        }
        r |= fneg_class(right & (NEG_ZERO | POS_INF | NEG_INF | NAN));
    }
    if left & ANY_POS_FINITE != 0 {
        if right & NEG_ZERO != 0 {
            r |= NEG_ZERO;
        }
        r |= right & (POS_ZERO | POS_INF | NEG_INF | NAN);
    }
    r |= table(
        left,
        right,
        &[
            (LT_NEG_ONE, LT_NEG_ONE, POS_INF | GT_POS_ONE),
            (LT_NEG_ONE, NEG_ONE, GT_POS_ONE),
            (LT_NEG_ONE, LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (LT_NEG_ONE, GT_POS_ONE, NEG_INF | LT_NEG_ONE),
            (LT_NEG_ONE, POS_ONE, LT_NEG_ONE),
            (LT_NEG_ONE, GT_ZERO_LT_POS_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
            (GT_POS_ONE, GT_POS_ONE, POS_INF | GT_POS_ONE),
            (GT_POS_ONE, POS_ONE, GT_POS_ONE),
            (GT_POS_ONE, GT_ZERO_LT_POS_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (GT_POS_ONE, LT_NEG_ONE, NEG_INF | LT_NEG_ONE),
            (GT_POS_ONE, NEG_ONE, LT_NEG_ONE),
            (GT_POS_ONE, LT_ZERO_GT_NEG_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, LT_NEG_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (LT_ZERO_GT_NEG_ONE, NEG_ONE, GT_ZERO_LT_POS_ONE),
            (LT_ZERO_GT_NEG_ONE, LT_ZERO_GT_NEG_ONE, POS_ZERO | GT_ZERO_LT_POS_ONE),
            (LT_ZERO_GT_NEG_ONE, GT_POS_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, POS_ONE, LT_ZERO_GT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE, NEG_ZERO | LT_ZERO_GT_NEG_ONE),
            (GT_ZERO_LT_POS_ONE, GT_POS_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, POS_ONE, GT_ZERO_LT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, GT_ZERO_LT_POS_ONE, POS_ZERO | GT_ZERO_LT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, LT_NEG_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
            (GT_ZERO_LT_POS_ONE, NEG_ONE, LT_ZERO_GT_NEG_ONE),
            (GT_ZERO_LT_POS_ONE, LT_ZERO_GT_NEG_ONE, NEG_ZERO | LT_ZERO_GT_NEG_ONE),
        ],
    );
    if left & NEG_ONE != 0 {
        r |= fneg_class(right & !ANY_ZERO);
    }
    if left & POS_ONE != 0 {
        r |= right & !ANY_ZERO;
    }
    for (z, pos_res, neg_res) in [(NEG_ZERO, NEG_ZERO, POS_ZERO), (POS_ZERO, POS_ZERO, NEG_ZERO)] {
        if left & z != 0 {
            if right & (ANY_INF | NAN) != 0 {
                r |= NAN;
            }
            /* -0 * neg = +0, -0 * pos = -0; +0 * pos = +0, +0 * neg = -0 */
            if right & (ANY_POS_FINITE | POS_ZERO) != 0 {
                r |= pos_res;
            }
            if right & (ANY_NEG_FINITE | NEG_ZERO) != 0 {
                r |= neg_res;
            }
        }
    }
    if src_eq || src_neg_eq {
        if left & (POS_ONE | NEG_ONE) == 0 {
            r &= !(POS_ONE | NEG_ONE);
        }
        if src_eq {
            r &= !(ANY_NEG | NEG_ZERO);
        } else {
            r &= !(ANY_POS | POS_ZERO);
        }
    }
    if (left | right) & NON_INTEGRAL != 0 {
        if r & FRACTIONS != 0 {
            r |= NON_INTEGRAL;
        }
    } else {
        r &= !(LT_ZERO_GT_NEG_ONE | GT_ZERO_LT_POS_ONE);
    }
    r
}

/// `fadd_fp_class` (:436-600).
fn fadd_class(left: u16, right: u16) -> u16 {
    use fc::*;
    if left == UNKNOWN && right == UNKNOWN {
        return UNKNOWN;
    }
    let mut r = (left | right) & NAN;
    if left & NEG_INF != 0 {
        if right & POS_INF != 0 {
            r |= NAN;
        }
        if right & (ANY_FINITE | NEG_INF) != 0 {
            r |= NEG_INF;
        }
    }
    if left & POS_INF != 0 {
        if right & NEG_INF != 0 {
            r |= NAN;
        }
        if right & (ANY_FINITE | POS_INF) != 0 {
            r |= POS_INF;
        }
    }
    for l in [LT_NEG_ONE, GT_POS_ONE, NEG_ONE, POS_ONE, LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE] {
        if left & l != 0 {
            r |= right & ANY_INF;
        }
    }
    r |= table(
        left,
        right,
        &[
            (LT_NEG_ONE, LT_NEG_ONE, NEG_INF | LT_NEG_ONE),
            (LT_NEG_ONE, NEG_ONE | LT_ZERO_GT_NEG_ONE | ANY_ZERO, LT_NEG_ONE),
            (LT_NEG_ONE, GT_ZERO_LT_POS_ONE | POS_ONE, LT_NEG_ONE | NEG_ONE | LT_ZERO_GT_NEG_ONE),
            (LT_NEG_ONE, GT_POS_ONE, ANY_FINITE),
            (GT_POS_ONE, GT_POS_ONE, POS_INF | GT_POS_ONE),
            (GT_POS_ONE, POS_ONE | GT_ZERO_LT_POS_ONE | ANY_ZERO, GT_POS_ONE),
            (GT_POS_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE, GT_POS_ONE | POS_ONE | GT_ZERO_LT_POS_ONE),
            (GT_POS_ONE, LT_NEG_ONE, ANY_FINITE),
            (NEG_ONE, LT_NEG_ONE | NEG_ONE | LT_ZERO_GT_NEG_ONE, LT_NEG_ONE),
            (NEG_ONE, LT_ZERO_GT_NEG_ONE | ANY_ZERO | GT_ZERO_LT_POS_ONE, NEG_ONE),
            (NEG_ONE, GT_ZERO_LT_POS_ONE, LT_ZERO_GT_NEG_ONE),
            (NEG_ONE, POS_ONE, POS_ZERO),
            (NEG_ONE, GT_POS_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (POS_ONE, GT_POS_ONE | POS_ONE | GT_ZERO_LT_POS_ONE, GT_POS_ONE),
            (POS_ONE, GT_ZERO_LT_POS_ONE | ANY_ZERO | LT_ZERO_GT_NEG_ONE, POS_ONE),
            (POS_ONE, LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE),
            (POS_ONE, NEG_ONE, POS_ZERO),
            (POS_ONE, LT_NEG_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, LT_NEG_ONE, LT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, NEG_ONE, LT_NEG_ONE | NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, LT_ZERO_GT_NEG_ONE, LT_NEG_ONE | NEG_ONE | LT_ZERO_GT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, ANY_ZERO, LT_ZERO_GT_NEG_ONE),
            (LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE, LT_ZERO_GT_NEG_ONE | ANY_ZERO | GT_ZERO_LT_POS_ONE),
            (LT_ZERO_GT_NEG_ONE, POS_ONE, GT_ZERO_LT_POS_ONE | POS_ONE),
            (LT_ZERO_GT_NEG_ONE, GT_POS_ONE, GT_ZERO_LT_POS_ONE | POS_ONE | GT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, GT_POS_ONE, GT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, POS_ONE, GT_POS_ONE | POS_ONE),
            (GT_ZERO_LT_POS_ONE, GT_ZERO_LT_POS_ONE, GT_POS_ONE | POS_ONE | GT_ZERO_LT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, ANY_ZERO, GT_ZERO_LT_POS_ONE),
            (GT_ZERO_LT_POS_ONE, LT_ZERO_GT_NEG_ONE, GT_ZERO_LT_POS_ONE | ANY_ZERO | LT_ZERO_GT_NEG_ONE),
            (GT_ZERO_LT_POS_ONE, NEG_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE),
            (GT_ZERO_LT_POS_ONE, LT_NEG_ONE, LT_ZERO_GT_NEG_ONE | NEG_ONE | LT_NEG_ONE),
        ],
    );
    if left & NEG_ZERO != 0 {
        r |= right;
    }
    if left & POS_ZERO != 0 {
        r |= right & !NEG_ZERO;
        if right & NEG_ZERO != 0 {
            r |= POS_ZERO;
        }
    }
    if (left | right) & NON_INTEGRAL != 0 {
        if r & FRACTIONS != 0 {
            r |= NON_INTEGRAL;
        }
    } else {
        r &= !(LT_ZERO_GT_NEG_ONE | GT_ZERO_LT_POS_ONE);
    }
    r
}

/// `fmin_fp_class` (:617-650).
fn fmin_class(left: u16, right: u16) -> u16 {
    use fc::*;
    let part = |upper: u16, value: u16| {
        let m = ANY_NEG | ANY_ZERO | ANY_POS;
        let (upper, value) = (upper & m, value & m);
        let last = 16 - upper.leading_zeros();
        value & ((1u32 << last) - 1) as u16
    };
    let mut r = 0;
    if left & NAN != 0 {
        r |= right;
    }
    if right & NAN != 0 {
        r |= left;
    }
    r |= part(left, right) | part(right, left);
    if (left | right) & NON_INTEGRAL != 0 && r & FRACTIONS != 0 {
        r |= NON_INTEGRAL;
    }
    r
}

/// `handle_sz` (:652-659), no instruction preserving the sign of zero.
fn handle_sz(src: u16) -> u16 {
    if src & fc::ANY_ZERO == 0 {
        src
    } else {
        src | fc::ANY_ZERO
    }
}

impl Nir<'_> {
    /// `nir_alu_srcs_negative_equal` (nir_instr_set.c:446-568) of two
    /// sources of one instruction.
    fn srcs_negative_equal(&self, a: u16, s1: usize, s2: usize) -> bool {
        let ins = &self.ins[a as usize];
        let (x, y) = (ins.srcs[s1], ins.srcs[s2]);
        let n = ins.comps as usize;
        if self.is_const(x.def) {
            if !self.is_const(y.def) || self.ins[x.def as usize].bits != self.ins[y.def as usize].bits {
                return false;
            }
            let bits = self.ins[x.def as usize].bits;
            return (0..n).all(|c| {
                let v1 = const_float(bits, self.ins[x.def as usize].k[x.swz[c] as usize]);
                let v2 = const_float(bits, self.ins[y.def as usize].k[y.swz[c] as usize]);
                matches!((v1, v2), (Some(p), Some(q)) if p == -q)
            });
        }
        let neg = |s: Src| match self.alu_op(s.def) {
            Some(Op::Fneg) => {
                let t = self.ins[s.def as usize].srcs[0];
                (true, t.def, t.swz)
            }
            _ => (false, s.def, ID),
        };
        let (p1, d1, w1) = neg(x);
        let (p2, d2, w2) = neg(y);
        if p1 == p2 || d1 != d2 {
            return false;
        }
        (0..n).all(|c| w1[x.swz[c] as usize & 3] == w2[y.swz[c] as usize & 3])
    }

    /// `nir_analyze_fp_class` (nir_range_analysis.c:760-1377) of a value.
    /// Each value is analysed once a query (`memo`, as Mesa's `range_ht`
    /// caches it); the recursion follows the value's sources, `depth`
    /// values below the query's. Mesa has no cut-off; this refuses the key
    /// past [`FP_CLASS_DEPTH`] (a frame a value, on a Zephyr thread's
    /// stack), never answering UNKNOWN where Mesa would answer a class.
    fn fp_class(&self, d: u16, memo: &[core::cell::Cell<u16>; MAX], depth: u32) -> R<u16> {
        let m = memo[d as usize].get();
        if m != NO_CLASS {
            return Ok(m);
        }
        if depth > FP_CLASS_DEPTH {
            return Err(MaliError::BlendVariantNotLowered);
        }
        let r = self.fp_class_of(d, memo, depth)?;
        memo[d as usize].set(r);
        Ok(r)
    }

    fn fp_class_of(&self, d: u16, memo: &[core::cell::Cell<u16>; MAX], depth: u32) -> R<u16> {
        use fc::*;
        let ins = &self.ins[d as usize];
        let op = match ins.kind {
            Kind::Const => return Ok(const_class(ins.bits, &ins.k[..ins.comps as usize])),
            Kind::Alu(op) => op,
            _ => return Ok(UNKNOWN),
        };
        let s = |k: usize| self.fp_class(ins.srcs[k].def, memo, depth + 1);
        let r = match op {
            Op::Vec2 => s(0)? | s(1)?,
            Op::F2f16 => {
                let mut r = handle_sz(s(0)?);
                if self.ins[ins.srcs[0].def as usize].bits > ins.bits {
                    if r & LT_NEG_ONE != 0 {
                        r |= NEG_INF | NEG_ONE;
                    }
                    if r & GT_POS_ONE != 0 {
                        r |= POS_INF | POS_ONE;
                    }
                    if r & LT_ZERO_GT_NEG_ONE != 0 {
                        r |= NEG_ONE | NEG_ZERO;
                    }
                    if r & GT_ZERO_LT_POS_ONE != 0 {
                        r |= POS_ONE | POS_ZERO;
                    }
                }
                r
            }
            Op::Fneg => fneg_class(s(0)?),
            Op::Fadd => fadd_class(s(0)?, s(1)?),
            Op::Fsub => fadd_class(s(0)?, fneg_class(s(1)?)),
            Op::Fmax => fneg_class(fmin_class(fneg_class(s(0)?), fneg_class(s(1)?))),
            Op::Fmin => fmin_class(s(0)?, s(1)?),
            Op::Fmul => {
                let eq = self.alu_srcs_equal(d, d, 0, 1);
                let neq = !self.is_const(ins.srcs[0].def) && self.srcs_negative_equal(d, 0, 1);
                fmul_class(s(0)?, s(1)?, eq, neq)
            }
            Op::Ffma => {
                let (a, b, c) = (s(0)?, s(1)?, s(2)?);
                let eq = self.alu_srcs_equal(d, d, 0, 1);
                let neq = !self.is_const(ins.srcs[0].def) && self.srcs_negative_equal(d, 0, 1);
                let mut r = fadd_class(fmul_class(a, b, eq, neq), c);
                let under = (a & LT_ZERO_GT_NEG_ONE != 0 && b & GT_ZERO_LT_POS_ONE != 0)
                    || (b & LT_ZERO_GT_NEG_ONE != 0 && a & GT_ZERO_LT_POS_ONE != 0);
                if !eq && under && c & POS_ZERO != 0 {
                    r |= NEG_ZERO;
                }
                r
            }
            Op::Mov => s(0)?,
            Op::Fsat => {
                let mut r = s(0)?;
                if r & (ANY_NEG | NEG_ZERO | NAN) != 0 {
                    r &= !(ANY_NEG | NEG_ZERO | NAN);
                    r |= POS_ZERO;
                }
                if r & (GT_POS_ONE | POS_INF) != 0 {
                    r &= !(GT_POS_ONE | POS_INF);
                    r |= POS_ONE;
                }
                if r & GT_ZERO_LT_POS_ONE == 0 {
                    r &= !NON_INTEGRAL;
                }
                r
            }
            _ => UNKNOWN,
        };
        Ok(if op.is_float() { handle_sz(r) } else { r })
    }
}

/// A RELATION condition of nir_search_helpers.h (:890-932).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FpRel {
    LtZero,
    NotPositive,
    GtZero,
    NotNegative,
    NotNegativeOrNegativeZero,
    NotZero,
    ZeroToOne,
    LePosOne,
    Gt0AndLt1,
    GePosOne,
    Number,
    Finite,
    FiniteNotZero,
    Integral,
    IntegralNotNegative,
}

impl FpRel {
    /// The classes the relation excludes; `number` adds NaN.
    fn exclude(self, number: bool) -> u16 {
        use fc::*;
        let e = match self {
            FpRel::LtZero => ANY_POS | ANY_ZERO,
            FpRel::NotPositive => ANY_POS,
            FpRel::GtZero => ANY_NEG | ANY_ZERO,
            FpRel::NotNegative => ANY_NEG,
            FpRel::NotNegativeOrNegativeZero => ANY_NEG | NEG_ZERO,
            FpRel::NotZero => ANY_ZERO,
            FpRel::ZeroToOne => ANY_NEG | GT_POS_ONE | POS_INF,
            FpRel::LePosOne => GT_POS_ONE | POS_INF,
            FpRel::Gt0AndLt1 => ANY_NEG | ANY_ZERO | POS_ONE | GT_POS_ONE | POS_INF,
            FpRel::GePosOne => ANY_NEG | ANY_ZERO | GT_ZERO_LT_POS_ONE,
            FpRel::Number => 0,
            FpRel::Finite => ANY_INF | NAN,
            FpRel::FiniteNotZero => ANY_INF | NAN | ANY_ZERO,
            FpRel::Integral => NON_INTEGRAL,
            FpRel::IntegralNotNegative => ANY_NEG | NON_INTEGRAL,
        };
        if number {
            e | NAN
        } else {
            e
        }
    }
}

// ---------------------------------------------------------------- algebraic

/// An automaton item (nir_algebraic.py `TreeAutomaton.Item`): an opcode and
/// the items its sources match; a commutative item is listed in both source
/// orders with one `id`.
pub struct Item {
    pub op: Op,
    pub n: u8,
    pub children: [u8; 4],
    pub id: u8,
}

/// An expression condition of a search pattern (nir_search_helpers.h).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cond {
    None,
    UsedOnce,
    OnlyUsedByFadd,
    NotOnlyUsedByFadd,
    OnlyUsedAsFloat,
    OnlyUsedAsFloatNsz,
    UsedByNonFsat,
    /// One this module does not evaluate: the shader is refused if the
    /// match reaches it.
    Unknown,
}

/// A variable condition of a search pattern.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VarCond {
    None,
    NotConst,
    NotFmul,
    Fmul,
    Nan,
    AnyCompNan,
    /// A fp class relation; `true` for its `is_a_number_` form.
    Fp(FpRel, bool),
    Unknown,
}

/// A search or replace value (`nir_search_value`).
pub enum Node {
    Expr { op: Op, bits: i8, swizzle: i8, comm: i8, src1_const: bool, cond: Cond, n: u8, srcs: [u16; 4] },
    Var { var: u8, is_const: bool, cond: VarCond, bits: i8, swizzle: [u8; 4], swizzled: bool },
    Float { bits: i8, value: f64 },
    Int { bits: i8, value: i64 },
}

/// A transform: its search and replace nodes; `refuse` when its
/// replacement builds an opcode this module does not.
pub struct Rule {
    pub search: u16,
    pub replace: u16,
    pub comm_exprs: u8,
    pub refuse: bool,
}

pub struct Pass {
    pub n_items: usize,
    pub items: &'static [Item],
    pub item_rules: &'static [&'static [u16]],
    pub rules: &'static [Rule],
    pub nodes: &'static [Node],
}

/// An automaton state: the set of items a value matches.
type State = [u64; 4];
const WILD: State = [1, 0, 0, 0];
const CONST_STATE: State = [3, 0, 0, 0];

fn has(s: &State, it: u8) -> bool {
    s[it as usize / 64] >> (it % 64) & 1 == 1
}

/// The matching state of nir_search.c's `match_state`.
struct Match {
    comb: u32,
    seen: u32,
    vars: [(u16, [u8; 4]); 32],
}

impl Nir<'_> {
    /// `nir_algebraic_automaton`: the items `i` matches.
    fn state_of(&self, p: &Pass, states: &[State], i: u16) -> State {
        let ins = &self.ins[i as usize];
        let op = match ins.kind {
            Kind::Const => return CONST_STATE,
            Kind::Alu(op) => op,
            _ => return WILD,
        };
        let sop = match op {
            Op::F2f16 => Op::F2f,
            Op::U2u8 => Op::U2u,
            o => o,
        };
        let mut s = WILD;
        for it in p.items {
            if it.op != sop || it.n != ins.n {
                continue;
            }
            if (0..it.n as usize).all(|k| has(&states[ins.srcs[k].def as usize], it.children[k])) {
                s[it.id as usize / 64] |= 1 << (it.id % 64);
            }
        }
        s
    }

    fn only_used_by_fadd(&self, i: u16) -> bool {
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let n = self.uses(i, &mut u);
        u[..n].iter().all(|x| match self.alu_op(x.user) {
            Some(Op::Fneg) => self.only_used_by_fadd(x.user),
            Some(Op::Fadd) => true,
            _ => false,
        })
    }

    fn only_used_as_float(&self, i: u16, depth: u32) -> bool {
        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
        let n = self.uses(i, &mut u);
        u[..n].iter().all(|x| match self.alu_op(x.user) {
            Some(op) if op.is_vec_or_mov() && depth < 8 && self.only_used_as_float(x.user, depth + 1) => true,
            Some(op) => op.is_float(),
            None => false,
        })
    }

    fn expr_cond(&self, c: Cond, i: u16) -> R<bool> {
        Ok(match c {
            Cond::None => true,
            Cond::UsedOnce => self.use_count(i) == 1,
            Cond::OnlyUsedByFadd => self.only_used_by_fadd(i),
            Cond::NotOnlyUsedByFadd => !self.only_used_by_fadd(i),
            Cond::OnlyUsedAsFloat | Cond::OnlyUsedAsFloatNsz => self.only_used_as_float(i, 0),
            Cond::UsedByNonFsat => {
                let mut u = [Use { user: 0, src: 0 }; MAX_USES];
                let n = self.uses(i, &mut u);
                u[..n].iter().any(|x| self.alu_op(x.user) != Some(Op::Fsat))
            }
            Cond::Unknown => return Err(MaliError::BlendVariantNotLowered),
        })
    }

    fn var_cond(&self, c: VarCond, instr: u16, src: usize, n: usize, swz: &[u8; 4]) -> R<bool> {
        let d = self.ins[instr as usize].srcs[src].def;
        let nan = |all: bool| {
            if !self.is_const(d) {
                return false;
            }
            let k = &self.ins[d as usize];
            let is = |c: usize| const_float(k.bits, k.k[swz[c] as usize]).is_some_and(f64::is_nan);
            if all {
                (0..n).all(is)
            } else {
                (0..n).any(is)
            }
        };
        Ok(match c {
            VarCond::None => true,
            VarCond::NotConst => !self.is_const(d),
            VarCond::NotFmul => {
                let mut x = d;
                loop {
                    match self.alu_op(x) {
                        Some(Op::Fneg) => x = self.ins[x as usize].srcs[0].def,
                        Some(Op::Fmul) => break false,
                        _ => break true,
                    }
                }
            }
            VarCond::Fmul => {
                let mut x = d;
                loop {
                    match self.alu_op(x) {
                        Some(Op::Fneg) => x = self.ins[x as usize].srcs[0].def,
                        Some(Op::Fmul) => break true,
                        _ => break false,
                    }
                }
            }
            VarCond::Nan => nan(true),
            VarCond::AnyCompNan => nan(false),
            VarCond::Fp(rel, number) => {
                let mut ex = rel.exclude(number);
                if ex & (fc::ANY_INF | fc::NAN) != 0 && self.alu_op(instr).is_some_and(Op::is_float) {
                    ex &= !(fc::ANY_INF | fc::NAN);
                    if ex == 0 {
                        return Ok(true);
                    }
                }
                let memo: [core::cell::Cell<u16>; MAX] = core::array::from_fn(|_| core::cell::Cell::new(NO_CLASS));
                self.fp_class(d, &memo, 0)? & ex == 0
            }
            VarCond::Unknown => return Err(MaliError::BlendVariantNotLowered),
        })
    }

    /// `match_value` (nir_search.c:197-339).
    fn match_value(
        &self,
        p: &Pass,
        v: u16,
        instr: u16,
        src: usize,
        mut n: usize,
        mut swz: [u8; 4],
        m: &mut Match,
    ) -> R<bool> {
        let ins = &self.ins[instr as usize];
        if let Some(op) = ins.op() {
            if op.input_size() != 0 {
                n = op.input_size() as usize;
                swz = ID;
            }
        }
        let s = ins.srcs[src];
        let mut ns = [0u8; 4];
        for c in 0..n {
            ns[c] = s.swz[swz[c] as usize & 3];
        }
        let d = &self.ins[s.def as usize];
        match p.nodes[v as usize] {
            Node::Expr { bits, .. } if bits > 0 && d.bits as i8 != bits => Ok(false),
            Node::Expr { .. } => {
                if !self.is_alu(s.def) {
                    return Ok(false);
                }
                self.match_expression(p, v, s.def, n, ns, m)
            }
            Node::Var { var, is_const, cond, bits, .. } => {
                if bits > 0 && d.bits as i8 != bits {
                    return Ok(false);
                }
                if is_const && !self.is_const(s.def) {
                    return Ok(false);
                }
                if cond != VarCond::None && !self.var_cond(cond, instr, src, n, &ns)? {
                    return Ok(false);
                }
                if m.seen >> var & 1 == 1 {
                    let (vd, vs) = m.vars[var as usize];
                    return Ok(vd == s.def && vs[..n] == ns[..n]);
                }
                m.seen |= 1 << var;
                let mut full = [0u8; 4];
                full[..n].copy_from_slice(&ns[..n]);
                m.vars[var as usize] = (s.def, full);
                Ok(true)
            }
            Node::Float { bits, value } => {
                if bits > 0 && d.bits as i8 != bits {
                    return Ok(false);
                }
                if !self.is_const(s.def) || d.bits < 16 {
                    return Ok(false);
                }
                Ok((0..n).all(|c| match const_float(d.bits, d.k[ns[c] as usize]) {
                    Some(f) => f == value && (f.to_bits() == 0) == (value.to_bits() == 0),
                    None => false,
                }))
            }
            Node::Int { bits, value } => {
                if bits > 0 && d.bits as i8 != bits {
                    return Ok(false);
                }
                if !self.is_const(s.def) {
                    return Ok(false);
                }
                let mask = if d.bits >= 64 { u64::MAX } else { (1u64 << d.bits) - 1 };
                Ok((0..n).all(|c| d.k[ns[c] as usize] & mask == value as u64 & mask))
            }
        }
    }

    /// `match_expression` (:341-418).
    fn match_expression(&self, p: &Pass, e: u16, instr: u16, n: usize, swz: [u8; 4], m: &mut Match) -> R<bool> {
        let Node::Expr { op, bits, swizzle, comm, src1_const, cond, n: nsrc, srcs } = p.nodes[e as usize] else {
            return Err(defect());
        };
        let ins = &self.ins[instr as usize];
        if cond != Cond::None && !self.expr_cond(cond, instr)? {
            return Ok(false);
        }
        let iop = ins.op().unwrap_or(Op::Other);
        let op_ok = op == iop || (op == Op::F2f && iop == Op::F2f16) || (op == Op::U2u && iop == Op::U2u8);
        if !op_ok || op == Op::Other {
            return Ok(false);
        }
        if bits > 0 && ins.bits as i8 != bits {
            return Ok(false);
        }
        if swizzle >= 0 {
            if n != 1 || swz[0] as i8 != swizzle {
                return Ok(false);
            }
        } else if iop.output_size() != 0 && (0..n).any(|c| swz[c] != c as u8) {
            return Ok(false);
        }
        let flip = if (0..8).contains(&comm) { (m.comb >> comm) as usize & 1 } else { 0 };
        let s1c = src1_const as usize;
        for i in 0..iop.num_inputs().min(nsrc as usize) {
            let si = if i < 2 { i ^ s1c } else { i };
            let isrc = if i < 2 { si ^ flip } else { si };
            if !self.match_value(p, srcs[si], instr, isrc, n, swz, m)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `construct_value` (:420-505): the replacement's instructions at the
    /// cursor, each folded to a constant when its sources are.
    fn construct(
        &mut self,
        p: &Pass,
        v: u16,
        mut n: u8,
        search_bits: u8,
        m: &Match,
        cur: &mut Cursor,
        states: &mut [State],
    ) -> R<(u16, [u8; 4])> {
        let bitsize = |bits: i8, m: &Match, nir: &Nir<'_>| -> u8 {
            if bits > 0 {
                bits as u8
            } else if bits < 0 {
                nir.ins[m.vars[(-bits - 1) as usize].0 as usize].bits
            } else {
                search_bits
            }
        };
        match p.nodes[v as usize] {
            Node::Expr { op, bits, swizzle, n: nsrc, srcs, .. } => {
                let b = bitsize(bits, m, self);
                let op = match op {
                    Op::F2f if b == 16 => Op::F2f16,
                    Op::U2u if b == 8 => Op::U2u8,
                    Op::F2f | Op::U2u | Op::Other => return Err(MaliError::BlendVariantNotLowered),
                    o => o,
                };
                if op.output_size() != 0 {
                    n = op.output_size();
                }
                let alu = self.new_instr(Kind::Alu(op), n, b)?;
                for k in 0..nsrc as usize {
                    if op.input_size() != 0 {
                        n = op.input_size();
                    }
                    let (d, s) = self.construct(p, srcs[k], n, search_bits, m, cur, states)?;
                    self.set_src(alu, k, d, s);
                }
                let d = match self.try_fold(alu)? {
                    Some(vals) => {
                        let k = self.konst(b, &vals[..self.ins[alu as usize].comps as usize])?;
                        self.cursor_insert(cur, k)?;
                        k
                    }
                    None => {
                        self.cursor_insert(cur, alu)?;
                        alu
                    }
                };
                states[d as usize] = self.state_of(p, states, d);
                let s = if swizzle < 0 { ID } else { [swizzle as u8; 4] };
                Ok((d, s))
            }
            Node::Var { var, swizzle, swizzled, .. } => {
                let (d, s) = m.vars[var as usize];
                let mut out = s;
                if swizzled {
                    for c in 0..4 {
                        out[c] = s[swizzle[c] as usize & 3];
                    }
                }
                Ok((d, out))
            }
            Node::Float { bits, value } => {
                let b = bitsize(bits, m, self);
                let k = self.konst(b, &[float_bits(value, b)?])?;
                self.cursor_insert(cur, k)?;
                states[k as usize] = CONST_STATE;
                Ok((k, [0; 4]))
            }
            Node::Int { bits, value } => {
                let b = bitsize(bits, m, self);
                let mask = if b >= 64 { u64::MAX } else { (1u64 << b) - 1 };
                let k = self.konst(b, &[value as u64 & mask])?;
                self.cursor_insert(cur, k)?;
                states[k as usize] = CONST_STATE;
                Ok((k, [0; 4]))
            }
        }
    }

    fn cursor_insert(&mut self, cur: &mut Cursor, i: u16) -> R<()> {
        match *cur {
            Cursor::Before(r) => {
                let at = self.pos(r)?;
                self.insert(i, at)
            }
            Cursor::After(r) => {
                let at = self.pos(r)? + 1;
                self.insert(i, at)?;
                *cur = Cursor::After(i);
                Ok(())
            }
        }
    }

    /// `nir_algebraic_impl` (nir_search.c:847-916) running `p`.
    pub fn algebraic(&mut self, p: &Pass) -> R<bool> {
        let states = core::mem::take(&mut self.states);
        let wl = core::mem::take(&mut self.worklist);
        let r = self.algebraic_with(p, states, wl);
        self.states = states;
        self.worklist = wl;
        r
    }

    fn algebraic_with(&mut self, p: &Pass, states: &mut [State], wl: &mut [u16]) -> R<bool> {
        states.fill(WILD);
        for k in 0..self.len {
            let i = self.order[k];
            states[i as usize] = self.state_of(p, states, i);
        }
        /* the worklist pops the last instruction first */
        let (mut head, mut tail) = (0usize, 0usize);
        for k in (0..self.len).rev() {
            let i = self.order[k];
            self.ins[i as usize].pass_flag = 0;
            if self.is_alu(i) {
                wl[tail] = i;
                tail += 1;
            }
        }
        let mut progress = false;
        while head < tail {
            let i = wl[head];
            head += 1;
            if self.ins[i as usize].pass_flag != 0 {
                continue;
            }
            let st = states[i as usize];
            let mut cands = [0u16; 64];
            let mut nc = 0;
            for it in 0..p.n_items {
                if has(&st, it as u8) {
                    for &r in p.item_rules[it] {
                        if nc == cands.len() {
                            return Err(MaliError::BlendVariantNotLowered);
                        }
                        cands[nc] = r;
                        nc += 1;
                    }
                }
            }
            cands[..nc].sort_unstable();
            for &ri in &cands[..nc] {
                let rule = &p.rules[ri as usize];
                let mut found = None;
                for comb in 0..1u32 << rule.comm_exprs.min(8) {
                    let mut m = Match { comb, seen: 0, vars: [(0, [0; 4]); 32] };
                    let comps = self.ins[i as usize].comps as usize;
                    if self.match_expression(p, rule.search, i, comps, ID, &mut m)? {
                        found = Some(m);
                        break;
                    }
                }
                let Some(m) = found else { continue };
                if rule.refuse {
                    return Err(MaliError::BlendVariantNotLowered);
                }
                /* the replacement goes before the root, or after the
                 * source of a unary negation (nir_search.c:600-629) */
                let src0 = self.ins[i as usize].srcs[0].def;
                let mut cur = if self.alu_op(i) == Some(Op::Fneg) && self.is_alu(src0) {
                    Cursor::After(src0)
                } else {
                    Cursor::Before(i)
                };
                let (comps, bits) = (self.ins[i as usize].comps, self.ins[i as usize].bits);
                let (d, swz) = self.construct(p, rule.replace, comps, bits, &m, &mut cur, states)?;
                /* nir_def_rewrite_uses_with_alu_src (nir_builder.c:428-456) */
                let trivial = self.ins[d as usize].comps == comps && (0..comps as usize).all(|c| swz[c] == c as u8);
                let mut u = [Use { user: 0, src: 0 }; MAX_USES];
                let nu = self.uses(i, &mut u);
                if trivial {
                    for x in &u[..nu] {
                        self.rewrite_src(*x, d);
                    }
                } else {
                    let mut mov = None;
                    for x in &u[..nu] {
                        if self.is_alu(x.user) {
                            let s = &mut self.ins[x.user as usize].srcs[x.src as usize];
                            for c in s.swz.iter_mut() {
                                *c = swz[*c as usize & 3];
                            }
                            self.rewrite_src(*x, d);
                        } else {
                            let mv = match mov {
                                Some(mv) => mv,
                                None => {
                                    let mv = self.alu(Op::Mov, comps, bits, &[(d, swz)])?;
                                    self.cursor_insert(&mut cur, mv)?;
                                    states[mv as usize] = self.state_of(p, states, mv);
                                    mov = Some(mv);
                                    mv
                                }
                            };
                            self.rewrite_src(*x, mv);
                        }
                    }
                }
                /* nir_algebraic_update_automaton: the uses whose state
                 * changes, then theirs, onto the worklist */
                let mut aw = [0u16; MAX];
                let (mut ah, mut at) = (0usize, 0usize);
                let push_uses =
                    |nir: &Nir<'_>, d: u16, states: &mut [State], aw: &mut [u16; MAX], at: &mut usize| -> R<()> {
                        let mut u = [Use { user: 0, src: 0 }; MAX_USES];
                        let nu = nir.uses(d, &mut u);
                        for x in &u[..nu] {
                            let ns = nir.state_of(p, states, x.user);
                            if states[x.user as usize] != ns {
                                states[x.user as usize] = ns;
                                if *at == MAX {
                                    return Err(MaliError::OutOfScratch);
                                }
                                aw[*at] = x.user;
                                *at += 1;
                            }
                        }
                        Ok(())
                    };
                push_uses(self, d, states, &mut aw, &mut at)?;
                while ah < at {
                    let x = aw[ah];
                    ah += 1;
                    if tail == wl.len() {
                        return Err(MaliError::OutOfScratch);
                    }
                    wl[tail] = x;
                    tail += 1;
                    push_uses(self, x, states, &mut aw, &mut at)?;
                }
                self.ins[i as usize].pass_flag = 1;
                self.remove(i)?;
                progress = true;
                break;
            }
        }
        Ok(progress)
    }
}

#[derive(Clone, Copy)]
enum Cursor {
    Before(u16),
    After(u16),
}

// ---------------------------------------------------------------- the passes

impl Nir<'_> {
    /// `bi_optimize_loop` (bifrost_nir.c:201-276): the passes that change
    /// a blend shader, until none does. `first`: the build already is
    /// after its first `nir_lower_alu_width`.
    fn optimize_loop(&mut self, mut first: bool) -> R<()> {
        loop {
            let mut progress = false;
            if !first {
                progress |= self.lower_alu_width()?;
            }
            first = false;
            progress |= self.vectorize()?;
            progress |= self.copy_prop()?;
            progress |= self.dce()?;
            progress |= self.cse()?;
            progress |= self.algebraic(&blend_nir_rules::OPT)?;
            progress |= self.constant_folding()?;
            if !progress {
                return Ok(());
            }
        }
    }

    fn late_loop(&mut self) -> R<()> {
        loop {
            let late = self.algebraic(&blend_nir_rules::LATE)?;
            self.lower_alu_width()?;
            self.constant_folding()?;
            self.copy_prop()?;
            self.dce()?;
            self.cse()?;
            if !late {
                return Ok(());
            }
        }
    }

    /// The NIR passes Mesa runs on a blend shader, in its order:
    /// `bifrost_preprocess_nir`, `bifrost_postprocess_nir`,
    /// `bi_optimize_late` and the start of `bifrost_compile_shader_nir`
    /// (bifrost_nir.c), the ones that leave a blend shader as it was left
    /// out.
    pub fn optimize(&mut self) -> R<()> {
        self.optimize_loop(true)?;
        /* bifrost_postprocess_nir */
        self.opt_move()?;
        self.lower_alu_width()?;
        self.load_const_to_scalar()?;
        self.optimize_loop(false)?;
        /* bi_optimize_late */
        self.optimize_loop(false)?;
        self.shrink_vectors()?;
        while self.algebraic(&blend_nir_rules::OPT)? {}
        self.late_loop()?;
        self.lower_alu_width()?;
        self.vectorize()?;
        let mut late = false;
        while self.algebraic(&blend_nir_rules::BIFROST)? {
            late = true;
        }
        if late {
            self.late_loop()?;
        }
        self.opt_move()?;
        /* bifrost_compile_shader_nir */
        self.load_const_to_scalar()?;
        self.constant_folding()?;
        self.copy_prop()?;
        self.dce()?;
        if self.overflow.get() {
            return Err(MaliError::BlendVariantNotLowered);
        }
        Ok(())
    }
}

#[cfg(test)]
impl Nir<'_> {
    /// The block as Mesa's `nir_print_shader` lists it, normalised as
    /// the references are compared (values numbered in order, constants
    /// by their bits, no intrinsic indices).
    pub fn dump(&self) -> std::vec::Vec<std::string::String> {
        use std::format;
        let mut idx = [usize::MAX; MAX];
        let mut n = 0;
        for k in 0..self.len {
            let i = self.order[k];
            if !matches!(self.ins[i as usize].kind, Kind::Intr(Intr::Blend | Intr::BlendReturn)) {
                idx[i as usize] = n;
                n += 1;
            }
        }
        let mut out = std::vec::Vec::new();
        for k in 0..self.len {
            let i = self.order[k];
            let ins = &self.ins[i as usize];
            let size = if ins.comps == 1 { format!("{}", ins.bits) } else { format!("{}x{}", ins.bits, ins.comps) };
            let src = |s: usize| {
                let x = ins.srcs[s];
                let d = &self.ins[x.def as usize];
                let mut t = format!("%{}", idx[x.def as usize]);
                if matches!(ins.kind, Kind::Alu(_)) {
                    let used = self.src_comps(i, s) as usize;
                    if (0..used).any(|c| x.swz[c] != c as u8) || used != d.comps as usize {
                        t.push('.');
                        for c in 0..used {
                            t.push(b"xyzw"[x.swz[c] as usize] as char);
                        }
                    }
                }
                t
            };
            let args = (0..ins.n as usize).map(src).collect::<std::vec::Vec<_>>().join(", ");
            let line = match ins.kind {
                Kind::Const => format!(
                    "{size} %{} = load_const ({})",
                    idx[i as usize],
                    ins.k[..ins.comps as usize]
                        .iter()
                        .map(|v| format!("0x{:0w$x}", v, w = ins.bits as usize / 4))
                        .collect::<std::vec::Vec<_>>()
                        .join(", ")
                ),
                Kind::Undef => format!("{size} %{} = undefined", idx[i as usize]),
                Kind::Alu(op) => {
                    let name = match op {
                        Op::Pack32_4x8Split => "pack_32_4x8_split".into(),
                        o => format!("{o:?}").to_lowercase(),
                    };
                    format!("{size} %{} = {name} {args}", idx[i as usize])
                }
                Kind::Intr(t) => {
                    let name = match t {
                        Intr::BlendInput | Intr::BlendInputDual => "load_blend_input_pan",
                        Intr::Coverage => "load_cumulative_coverage_pan",
                        Intr::SampleId => "load_sample_id",
                        Intr::LoadTile => "load_tile_pan",
                        Intr::LoadUbo => "load_ubo",
                        Intr::Blend => "blend_pan",
                        Intr::BlendReturn => "blend_return_pan",
                    };
                    let body = if args.is_empty() { format!("@{name}") } else { format!("@{name} ({args})") };
                    if matches!(t, Intr::Blend | Intr::BlendReturn) {
                        body
                    } else {
                        format!("{size} %{} = {body}", idx[i as usize])
                    }
                }
            };
            out.push(line);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The NIR Mesa 26.2.1's passes leave for one key, as `pan_shc
    /// --blend` with `NIR_DEBUG=print_fs,print_internal,print_no_inline_consts`
    /// prints it after its last pass (normalised: values numbered in order,
    /// a constant by its bits, no intrinsic indices), is ours: a reverse
    /// subtract with a constant's inverse (one blend constant word read,
    /// its pair shrunk), SRC_ALPHA_SATURATE on alpha and a colour mask
    /// keeping the target's blue. tools/mali/testdata/blend_deqp.words holds
    /// the words of 1403 more (shaderc/online/mali/tests.rs).
    #[test]
    fn the_nir_mesa_s_passes_leave_is_ours() {
        let one = Factor { base: Base::One, invert: false };
        let key = Key {
            rgb: Channel {
                func: Func::ReverseSubtract,
                src: Factor { base: Base::ConstAlpha, invert: true },
                dst: Factor { base: Base::One, invert: true },
            },
            alpha: Channel { func: Func::Add, src: Factor { base: Base::SrcAlphaSaturate, invert: false }, dst: one },
            mask: 11,
            samples: 1,
            colour_f32: false,
        };
        let mesa = "16x4 %0 = @load_blend_input_pan
32 %1 = @load_cumulative_coverage_pan
32 %2 = load_const (0x0000001f)
32 %3 = iand %1, %2
32 %4 = load_const (0x000ed000)
32 %5 = load_const (0xff000000)
16x4 %6 = @load_tile_pan (%5, %3, %4)
32 %7 = load_const (0x00000000)
32 %8 = load_const (0x00000001)
32x4 %9 = @load_ubo (%8, %7)
16 %10 = f2f16 %9.w
16x2 %11 = fsat %0.xy
16 %12 = fsat %0.w
16 %13 = fneg %10
16 %14 = load_const (0x3c00)
16 %15 = fadd %14, %13
16 %16 = fsat %15
16x2 %17 = fmul %11, %16.xx
16x2 %18 = fneg %17
16 %19 = fadd %12, %6.w
16x4 %20 = vec4 %18.x, %18.y, %6.z, %19
64 %21 = load_const (0x000ed00000000019)
@blend_pan (%1, %21, %20)
@blend_return_pan";
        let ours = crate::shaderc::online::store::with_scratch(|sc| {
            let mut n = Nir::new(sc).expect("the scratch arena holds a blend shader's NIR");
            build(&mut n, key).expect("the build");
            n.optimize().expect("Mesa's passes");
            n.dump().join("\n")
        })
        .expect("the scratch arena is free");
        assert_eq!(ours, mesa);
    }

    /// `fp_class` refuses a value whose sources run deeper than
    /// [`FP_CLASS_DEPTH`] (a key refused, never UNKNOWN for a class Mesa
    /// would find), and answers one at the bound: a chain of `fneg` on a
    /// constant 1.0.
    #[test]
    fn fp_class_refuses_past_its_depth() {
        crate::shaderc::online::store::with_scratch(|sc| {
            let mut n = Nir::new(sc).expect("the scratch arena");
            let mut v = n.konst(16, &[ONE16]).expect("a constant");
            let mut chain = vec![v];
            for _ in 0..FP_CLASS_DEPTH + 1 {
                v = n.alu(Op::Fneg, 1, 16, &[(v, sw(0))]).expect("an fneg");
                chain.push(v);
            }
            let memo = || -> [core::cell::Cell<u16>; MAX] { core::array::from_fn(|_| core::cell::Cell::new(NO_CLASS)) };
            /* FP_CLASS_DEPTH fnegs over the constant: the constant at the
             * bound, a class (-1.0 after an odd count, 1.0 after an even) */
            let at = chain[FP_CLASS_DEPTH as usize];
            assert!(n.fp_class(at, &memo(), 0).is_ok(), "at the bound");
            /* one more: the constant past it */
            let past = chain[FP_CLASS_DEPTH as usize + 1];
            assert_eq!(n.fp_class(past, &memo(), 0), Err(MaliError::BlendVariantNotLowered));
        })
        .expect("the scratch arena is free");
    }
}
