// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Selection of a vertex shader: the program Mesa's NIR has before
//! instruction selection ([`super::nir`]; a fragment shader's is
//! [`super::nir_fs`]) to the BIR Mesa's Valhall compiler has when its
//! optimisation passes are
//! done and `bi_lower_branch` has run (the program
//! `BIFROST_MESA_DEBUG=shaders` prints first, bifrost_compile.c:4538-4539).
//!
//! * the IDVS split (bifrost_compile.c:4374-4399): the position variant
//!   keeps the position stores, the varying variant the varying stores
//!   ([`vs_variant`]);
//! * the output stores (`pan_nir_lower_vs_outputs`,
//!   panfrost/compiler/pan_nir_lower_vs_outputs.c:57-103): per output a
//!   `lea_buf_pan` of the varying buffer and one 32-bit `store_global` per
//!   component at `+4k` (26.2.1 splits the store, tools/mali/README-pan_shc.md,
//!   "VS stores"); `.istream` for the position, `.estream` for a generic
//!   varying;
//! * the uniforms: words of Mesa's packed uniform block ([`UboLayout`]),
//!   pushed as FAU words by `bi_opt_push_ubo` (fau.rs);
//! * the generic varyings' layout, shared with the fragment side
//!   ([`generic_layout`]).
//!
//! The resulting order and operand forms are the reference's, read from
//! the shim driver's dump (Mesa 26.2.1 on drm-shim,
//! `tools/mali/drm_shim_ref.sh run`): the corpus and the selector's
//! populations (tools/mali/testdata/sel_*, tests.rs `POPULATION`,
//! `UNIFORM_POPULATION` and `VARYING_POPULATION`) are checked to the byte
//! by the tests. A shape
//! outside them is refused with its own message ([`super::MaliError`]),
//! never approximated.

use super::bir::{Access, Dest, Idx, Ins, LodMode, Op, Prog, Src, TexDim};
use super::emit;
use super::fau::FauWord;
use super::nir;
use super::MaliError;
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::Module;

/// `f2f16` of a constant (NIR folds it), refused unless fp16 holds the
/// value exactly: then the rounding mode cannot matter.
pub fn f16_exact(bits: u32) -> Option<u16> {
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0 && man == 0 {
        return Some(sign);
    }
    if exp == 0xff || exp == 0 {
        return None; /* inf, NaN, an fp32 subnormal (none is an fp16 value we take) */
    }
    let e = exp - 127;
    if (-14..=15).contains(&e) {
        /* normal: 10 mantissa bits */
        if man & 0x1fff != 0 {
            return None;
        }
        return Some(sign | (((e + 15) as u16) << 10) | (man >> 13) as u16);
    }
    if (-24..-14).contains(&e) {
        /* fp16 subnormal: the implicit one and the mantissa shifted */
        let full = man | 0x80_0000;
        let shift = 13 + (-14 - e) as u32;
        if full & ((1 << shift) - 1) != 0 {
            return None;
        }
        return Some(sign | (full >> shift) as u16);
    }
    None
}

/// `_mesa_float_to_half` (round to nearest even) of a constant whose fp16
/// value is normal, as GLSL's lower_precision converts a mediump
/// operation's constant operand (`lower_constant`, lower_precision.cpp:
/// 993-1010); None for one that is not (an infinity, a NaN, a subnormal or
/// an overflow), unless fp16 holds it exactly ([`f16_exact`]).
pub fn f16_rtne(bits: u32) -> Option<u16> {
    if let Some(h) = f16_exact(bits) {
        return Some(h);
    }
    let sign = ((bits >> 16) & 0x8000) as u16;
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let man = bits & 0x7f_ffff;
    if !(-14..=15).contains(&e) {
        return None;
    }
    let (q, r) = (((e + 15) as u32) << 10 | man >> 13, man & 0x1fff);
    let q = q + (r > 0x1000 || (r == 0x1000 && q & 1 == 1)) as u32;
    /* a carry into the exponent past 15 is an infinity */
    (q < 0x7c00).then_some(sign | q as u16)
}

/// The generic varyings a stage may have (the selector keeps 16 varying
/// locations, nir.rs).
pub const MAX_VARYINGS: usize = 16;

/// One generic varying where Mesa puts it: the front end's location, its
/// 32-bit components and its byte offset in a vertex's generic varyings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VaryingAt {
    pub location: u8,
    pub comps: u8,
    pub offset: u16,
}

/// The generic varying layout Mesa's link gives a program's smooth float
/// varyings, in its order (`varying_matches_assign_locations`,
/// compiler/glsl/gl_nir_link_varyings.c:2820-2960): the varyings sorted by
/// packing order (`compute_packing_order`, :2695-2708: a vec4 first, then a
/// vec2, a float, a vec3), equal ones in declaration order, each from the
/// next free component. Two varyings may share a slot (GLSL's
/// `lower_packed_varyings`: the sort puts them together whenever a vec2, a
/// float or a vec3 follows one that is not a vec4), and Mesa's link then
/// keeps the components where the count puts them (`compact_varyings`), so
/// a varying's byte offset is 4 times its first component (measured:
/// sel_v_color_uv puts the vec2 at byte 16, sel_v_v4v4v2 the vec2 at 32,
/// and the shim's sel_v_packed the second of two vec2 at byte 8, both read
/// by one four-lane load).
///
/// `decls` are the stage's varyings in declaration order (location,
/// components): the producer's order decides (measured on the board's
/// GLSL path: two vec2 declared the other way round swap halves), so a
/// fragment shader that declares them in another order lays them out
/// otherwise, and the link compiles it again with its reads in the vertex
/// shader's order (link.rs `VaryingOrder`).
pub fn generic_layout(
    decls: impl Iterator<Item = (u8, u8)>,
    out: &mut [VaryingAt; MAX_VARYINGS],
) -> Result<usize, MaliError> {
    let rank = |comps: u8| match comps % 4 {
        0 => 0,
        2 => 1,
        1 => 2,
        _ => 3,
    };
    let mut n = 0;
    for (location, comps) in decls {
        if n >= MAX_VARYINGS {
            return Err(MaliError::VaryingLocation);
        }
        if !(1..=4).contains(&comps) {
            /* the front end's varyings are 1 to 4 components */
            return Err(MaliError::Defect(super::Defect::IrIndex));
        }
        /* a stable insertion: after every one of the same or a lower rank */
        let at = out[..n].iter().position(|v| rank(v.comps) > rank(comps)).unwrap_or(n);
        out.copy_within(at..n, at + 1);
        out[at] = VaryingAt { location, comps, offset: 0 };
        n += 1;
    }
    let mut component = 0u16;
    for v in out[..n].iter_mut() {
        v.offset = 4 * component;
        component += v.comps as u16;
    }
    Ok(n)
}

/// Emit `SPLIT v` into `n` scalars; returns them.
pub(super) fn split(p: &mut Prog<'_>, v: u16, n: usize) -> Result<[u16; 4], MaliError> {
    let mut d = [0u16; 4];
    let mut dests = [Dest::NULL; 4];
    for k in 0..n {
        d[k] = p.temp()?;
        dests[k] = Dest::ssa(d[k]);
    }
    p.push(Ins::new(Op::Split, &dests[..n], &[Src::ssa(v)]))?;
    Ok(d)
}

/// `lea_buf_pan` of the varying buffer's position/generic section: resource
/// table 61, buffer 0 (`pan_res_handle(61, 0)`, the `0x3d000000` constant,
/// pan_nir_lower_vs_outputs.c:64-72), index = the IDVS output buffer index
/// in r59 (`load_idvs_output_buf_index_pan`); the point size's at that
/// index plus 4 (`IADD_IMM.i32 r59, #4`, the attribs section).
fn lea(p: &mut Prog<'_>, psiz: bool) -> Result<[u16; 2], MaliError> {
    let index =
        if psiz { Src::ssa(alu(p, Op::IaddImmI32(4), &[Src::of(Idx::Reg(59))])?) } else { Src::of(Idx::Reg(59)) };
    let v = p.temp()?;
    let mut i = Ins::new(Op::LeaBufImm { table: 61, index: 0 }, &[Dest::ssa(v)], &[index]);
    i.dest[0] = Dest::ssa(v);
    p.push(i)?;
    let s = split(p, v, 2)?;
    Ok([s[0], s[1]])
}

/// `LD_ATTR_IMM` of attribute record `loc` (the GL location: tri_varying's
/// `col` at location 1 is record 1, pan_shc's `attributes`), vertex and
/// instance id in r60/r61, and its SPLIT.
fn ld_attr(p: &mut Prog<'_>, loc: u8, n: u8) -> Result<[u16; 4], MaliError> {
    let v = p.temp()?;
    p.push(Ins::new(
        Op::LdAttrImm { index: loc, vecsize: n },
        &[Dest::ssa(v)],
        &[Src::of(Idx::Reg(60)), Src::of(Idx::Reg(61))],
    ))?;
    split(p, v, n as usize)
}

pub(super) fn alu(p: &mut Prog<'_>, op: Op, srcs: &[Src]) -> Result<u16, MaliError> {
    let v = p.temp()?;
    p.push(Ins::new(op, &[Dest::ssa(v)], srcs))?;
    Ok(v)
}

/// One uniform, or one element of a uniform array, in Mesa's packed block
/// ([`UboLayout`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UboRow {
    /// The front end's first dword.
    pub dword: u16,
    /// Mesa's first 32-bit word.
    pub word: u16,
    /// Its components, one front-end dword each.
    pub comps: u16,
    /// Two fp16 components a word (a mediump fragment shader's float
    /// uniform, `LowerPrecisionFloat16Uniforms`; `glsl_count_dword_slots`
    /// gives a 16-bit vector `DIV_ROUND_UP(components, 2)` dwords); else
    /// one fp32 or int32 component a word.
    pub halves: bool,
}

impl UboRow {
    pub const NONE: UboRow = UboRow { dword: 0, word: 0, comps: 0, halves: false };

    /// The 32-bit words the row takes.
    pub const fn words(&self) -> u16 {
        if self.halves {
            self.comps.div_ceil(2)
        } else {
            self.comps
        }
    }
}

/// Where the uniforms are in Mesa's block: the packed layout of the GL
/// driver's uniform storage (`PackedDriverUniformStorage`: one dword per
/// component, the stage's used uniforms in declaration order, nothing
/// aligned; measured: sel_u_float_vec4 puts the vec4 at word 1,
/// sel_u_vec3_float the float at word 3, sel_u_unused gives an unused
/// uniform no word). A fragment shader's mediump float uniform is fp16
/// there (`LowerPrecisionFloat16Uniforms`: two components a word,
/// sel_u_color), an int uniform one 32-bit word (uniidx_i: `fidx` at byte
/// 32 after `vec4 fcols[4]`'s 8 words, the block's `Size: 36`).
#[derive(Clone, Copy, Debug)]
pub struct UboLayout {
    rows: [UboRow; MAX_UBO_ROWS],
    n: usize,
    /// The block's words through its last row: the exact bytes Mesa gives
    /// the UBO descriptor (`cfg.size`, pan_cmdstream.c:1438-1452) are
    /// `4 * words`.
    words: u16,
    /// Bit k: row k is a uniform array's ([`UboLayout::vs`]).
    arrays: u32,
}

/// The uniforms one stage's layout holds.
pub const MAX_UBO_ROWS: usize = 32;

impl UboLayout {
    pub const NONE: UboLayout = UboLayout { rows: [UboRow::NONE; MAX_UBO_ROWS], n: 0, words: 0, arrays: 0 };

    /// The uniforms the layout holds (its use of [`MAX_UBO_ROWS`]).
    pub fn rows(&self) -> usize {
        self.n
    }

    /// The rows, in block order.
    pub fn as_slice(&self) -> &[UboRow] {
        &self.rows[..self.n]
    }

    /// The block's bytes.
    pub fn bytes(&self) -> u32 {
        4 * u32::from(self.words)
    }

    fn push(&mut self, dword: u16, comps: u16, halves: bool) -> Result<(), MaliError> {
        if self.n >= self.rows.len() {
            return Err(MaliError::UboFallback);
        }
        let row = UboRow { dword, word: self.words, comps, halves };
        self.rows[self.n] = row;
        self.n += 1;
        self.words += row.words();
        Ok(())
    }

    /// A vertex shader's: the uniforms its program reads.
    pub fn vs(m: &Module<'_>, vs: &nir::Nir<'_>) -> Result<UboLayout, MaliError> {
        let mut l = UboLayout::NONE;
        for u in m.uniforms.iter().filter(|u| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
            let used = vs.order[..vs.len].iter().any(|&id| match vs.nodes[id as usize].kind {
                nir::Kind::Uniform { dword, .. } => dword >= u.dword_off && dword < u.dword_off + u.dwords.max(1),
                nir::Kind::UniformIdx { dword, .. } => dword == u.dword_off,
                _ => false,
            });
            if !used {
                continue;
            }
            /* a float array read at a run-time index: one fp32 word an
             * element (uniarray_vs: `ui_two` at word 0, `u_arr` from word 1,
             * the varying program's `IADD_IMM.i32 #0x4`) */
            let (_, comps) = super::nir_fs::uniform_extent(m, u.dword_off)?;
            l.push(u.dword_off, comps, false)?;
            if u.array_len > 1 {
                l.arrays |= 1 << (l.n - 1);
            }
        }
        Ok(l)
    }

    /// Front-end dword `dword` is an element's of a uniform array (a
    /// vertex shader's layout, [`UboLayout::vs`]).
    pub fn in_array(&self, dword: u16) -> bool {
        let k = self.rows[..self.n].iter().position(|r| dword >= r.dword && dword < r.dword + r.comps);
        k.is_some_and(|k| self.arrays & (1 << k) != 0)
    }

    /// The program reads the block from memory (a [`nir::Kind::UniformIdx`]
    /// node, Mesa's `LD_PKA`): the frame uploads it (`ubo_mask` bit 0).
    pub fn loaded(g: &nir::Nir<'_>) -> bool {
        g.order[..g.len].iter().any(|&id| matches!(g.nodes[id as usize].kind, nir::Kind::UniformIdx { .. }))
    }

    /// A highp fragment shader's: the uniforms its graph reads, fp32 words
    /// as the vertex shader's ([`UboLayout::vs`]), a vec4 array as its
    /// elements in a row (nir_fs.rs `uniform_extent`).
    pub fn fs32(m: &Module<'_>, fs: &nir::Nir<'_>) -> Result<UboLayout, MaliError> {
        let mut l = UboLayout::NONE;
        for u in m.uniforms.iter().filter(|u| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
            let used = fs.order[..fs.len].iter().any(|&id| match fs.nodes[id as usize].kind {
                nir::Kind::Uniform { dword, .. } => dword >= u.dword_off && dword < u.dword_off + u.dwords.max(1),
                nir::Kind::UniformIdx { dword, .. } => dword == u.dword_off,
                _ => false,
            });
            if !used {
                continue;
            }
            /* a float array read at a run-time index: one fp32 word an
             * element (uniarray_highp: the four ints at words 0-3, `u_arr`
             * from word 4, `IADD_IMM.i32 #0x10`) */
            let (_, comps) = super::nir_fs::uniform_extent(m, u.dword_off)?;
            l.push(u.dword_off, comps, false)?;
        }
        /* gl_FbWposYTransform, a vec4 state slot after the declared
         * uniforms (`_mesa_add_state_reference` appends it to the
         * parameters st_nir_assign_uniform_locations lays out) */
        let wpos = super::nir_fs::FB_WPOS_Y_TRANSFORM;
        let reads = |id: &u16| matches!(fs.nodes[*id as usize].kind, nir::Kind::Uniform { dword, .. } if dword == wpos);
        if fs.order[..fs.len].iter().any(reads) {
            l.push(wpos, 4, false)?;
        }
        Ok(l)
    }

    /// A mediump fragment shader's: the uniforms its graph reads, each
    /// float, vec2, vec3 or vec4 as fp16 halves (two a word), an int as one
    /// word, a vec4 array as one row per element (two words each, the
    /// stride of Mesa's `load_ubo(0, index * 8)`, uniidx_i), a float array
    /// as one row per element, each its own word (the fp16 element in the
    /// low half of a dword slot, the stride of Mesa's `load_ubo(0, base +
    /// index * 4)`, case108_uniidx's `vec4[2] uniform_0`), in declaration
    /// order. `highp` names a uniform declared at the other precision, which
    /// Mesa keeps fp32: refused ([`MaliError::FsUniform`]), as is any other
    /// type (no reference draws it).
    pub fn fs16(m: &Module<'_>, fs: &nir::Nir<'_>, highp: &dyn Fn(&str) -> bool) -> Result<UboLayout, MaliError> {
        use crate::gl::consts::{GL_BOOL, GL_FLOAT, GL_FLOAT_VEC2, GL_FLOAT_VEC3, GL_FLOAT_VEC4, GL_INT};
        let mut l = UboLayout::NONE;
        for u in m.uniforms.iter().filter(|u| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
            let end = u.dword_off + u.dwords.max(1);
            let used = fs.order[..fs.len].iter().any(|&id| match fs.nodes[id as usize].kind {
                nir::Kind::Uniform { dword, .. } => dword >= u.dword_off && dword < end,
                nir::Kind::UniformIdx { dword, .. } => dword == u.dword_off,
                _ => false,
            });
            if !used {
                continue;
            }
            let (comps, halves) = match u.gl_type as u32 {
                GL_INT | GL_BOOL if u.array_len <= 1 => (1, false),
                GL_FLOAT => (1, true),
                GL_FLOAT_VEC2 if u.array_len <= 1 => (2, true),
                GL_FLOAT_VEC3 if u.array_len <= 1 => (3, true),
                GL_FLOAT_VEC4 => (4, true),
                _ => return Err(MaliError::FsUniform),
            };
            if halves && highp(u.name) {
                return Err(MaliError::FsUniform);
            }
            for e in 0..u16::from(u.array_len.max(1)) {
                l.push(u.dword_off + e * comps, comps, halves)?;
            }
        }
        /* gl_FbWposYTransform, as [`UboLayout::fs32`] places it: a state
         * slot, no declared mediump uniform, so its words stay fp32
         * (prec_84.expect.json: `FMA.f32 r1, r1^, u2.w0, u2.w1`, the words
         * 1.0 and 0.0, before the fp16 constant pair `u3.w0`) */
        let wpos = super::nir_fs::FB_WPOS_Y_TRANSFORM;
        let reads = |id: &u16| matches!(fs.nodes[*id as usize].kind, nir::Kind::Uniform { dword, .. } if dword == wpos);
        if fs.order[..fs.len].iter().any(reads) {
            l.push(wpos, 4, false)?;
        }
        Ok(l)
    }

    /// The row that holds front-end dword `dword`.
    fn row(&self, dword: u16) -> Option<&UboRow> {
        self.rows[..self.n].iter().find(|r| dword >= r.dword && dword < r.dword + r.comps)
    }

    /// Mesa's word of a front-end dword (a 32-bit row; a halves row's word
    /// holds two dwords, [`UboLayout::half`]).
    pub fn word(&self, dword: u16) -> Option<u16> {
        self.row(dword).map(|r| r.word + if r.halves { (dword - r.dword) / 2 } else { dword - r.dword })
    }

    /// The word and half (0: low, 1: high) of a front-end dword in a halves
    /// row, and the row's first word (the `load_ubo` the push counts from).
    pub fn half(&self, dword: u16) -> Option<(u16, u8, u16)> {
        let r = self.row(dword).filter(|r| r.halves)?;
        let k = dword - r.dword;
        Some((r.word + k / 2, (k % 2) as u8, r.word))
    }

    /// What Mesa's word `w` holds, as a push word.
    pub fn fau(&self, w: u16) -> Option<FauWord> {
        let r = self.rows[..self.n].iter().find(|r| w >= r.word && w < r.word + r.words())?;
        if r.halves {
            let k = 2 * (w - r.word);
            return Some(FauWord::UniformHalves { lo: r.dword + k, hi: (k + 1 < r.comps).then_some(r.dword + k + 1) });
        }
        if r.dword == super::nir_fs::FB_WPOS_Y_TRANSFORM {
            return Some(FauWord::FbWposYTransform((w - r.word) as u8));
        }
        Some(FauWord::Uniform { dword: r.dword + (w - r.word) })
    }
}

/// One section's stores: `lea_buf`, then per word `addr + 4k` and a
/// 32-bit store (pan_nir_lower_vs_outputs.c:57-103, with
/// `nir_lower_mem_access_bit_sizes` splitting the store,
/// bifrost_nir.c:1009-1017). The address of word k > 0 is `SHADDX.s64
/// (4k, 0), lea` (`lea, (4k, 0)` for an output's first word,
/// [`nir::Kind::Addr`]) with the `lea` pair collected once (CSE).
#[derive(Clone, Copy)]
struct Out {
    lea: [u16; 2],
    la: Option<u16>,
    access: Access,
    /// A 16-bit store (the fp16 point size).
    half: bool,
}

/// `addr + 4k` of word `k`: its two halves.
fn address(p: &mut Prog<'_>, o: &mut Out, k: u8, swapped: bool) -> Result<[u16; 2], MaliError> {
    let c = p.temp()?;
    p.push(Ins::new(Op::Collect, &[Dest::ssa(c)], &[Src::of(Idx::Const(4 * k as u32)), Src::of(Idx::Const(0))]))?;
    let la = match o.la {
        Some(v) => v,
        None => {
            let v = p.temp()?;
            p.push(Ins::new(Op::Collect, &[Dest::ssa(v)], &[Src::ssa(o.lea[0]), Src::ssa(o.lea[1])]))?;
            o.la = Some(v);
            v
        }
    };
    let a = p.temp()?;
    let srcs = if swapped { [Src::ssa(la), Src::ssa(c)] } else { [Src::ssa(c), Src::ssa(la)] };
    p.push(Ins::new(Op::ShaddxS64, &[Dest::ssa(a)], &srcs))?;
    let s = split(p, a, 2)?;
    Ok([s[0], s[1]])
}

fn store(p: &mut Prog<'_>, o: &Out, at: [u16; 2], data: Src) -> Result<(), MaliError> {
    /* a FAU word read by a staging source is copied by a move first */
    let data = match data.idx {
        Idx::Ubo { .. } | Idx::Sysval(..) => Src::ssa(alu(p, Op::MovI32, &[data])?),
        _ => data,
    };
    let op = if o.half { Op::StoreI16 { access: o.access } } else { Op::StoreI32 { access: o.access } };
    p.push(Ins::new(op, &[], &[data, Src::ssa(at[0]), Src::ssa(at[1])]))?;
    Ok(())
}

/// One IDVS variant of a vertex shader (bifrost_compile.c:4374-4399): the
/// program's nodes the stores of output `out` need (0: the position
/// variant, 1: the varying variant), in the program's order, each as the
/// BIR Mesa selects for it (bifrost_compile.c `bi_emit_alu`,
/// `bi_emit_intrinsic`):
///
/// * `load_input`: `LD_ATTR_IMM` of the lanes the program reads, and its
///   SPLIT; `load_ubo` and the sysval loads: FAU words (`bi_opt_push_ubo`
///   pushes them);
/// * an ALU operation: the row of `bi_emit_alu` it is (emit.rs); the
///   passes after selection fold its modifiers and immediates (opt.rs).
pub fn vs_variant(
    p: &mut Prog<'_>,
    vs: &mut nir::Nir<'_>,
    out: u8,
    ubo: &UboLayout,
    scratch: &Arena<'_>,
) -> Result<(), MaliError> {
    nir::dce(vs, Some(out));
    let lanes = scratch.alloc_filled(nir::MAX_NODES, [Src::NULL; 4]).map_err(|_| MaliError::OutOfScratch)?;
    let mut outs = [None::<Out>; 3];
    let src = |lanes: &[[Src; 4]], v: nir::Val| -> Result<Src, MaliError> {
        match v {
            nir::Val::Const(c) => Ok(Src::of(Idx::Const(c))),
            nir::Val::Lane(n, c) => {
                let s = lanes[n as usize][c as usize & 3];
                if s.idx == Idx::Null {
                    return Err(MaliError::Defect(super::Defect::IrIndex));
                }
                Ok(s)
            }
            nir::Val::Undef | nir::Val::Pair(..) => Err(MaliError::Defect(super::Defect::IrIndex)),
        }
    };
    for k in 0..vs.len {
        let id = vs.order[k] as usize;
        if !vs.live[id] {
            continue;
        }
        let nd = vs.nodes[id];
        match nd.kind {
            nir::Kind::Input { loc, comps } => {
                let s = ld_attr(p, loc, comps)?;
                for c in 0..comps as usize {
                    lanes[id][c] = Src::ssa(s[c]);
                }
            }
            nir::Kind::Uniform { dword, comps } => {
                let base = ubo.word(dword).ok_or(MaliError::Defect(super::Defect::IrIndex))?;
                /* an array element's lanes are pushed alone, each its own
                 * load (measured: varyconst_150_a.expect.json pushes
                 * `u_p[58].y` without `u_p[58].x`, 32 FAU words; a vec4
                 * that is no array pushes from its x, sel_u_yz) */
                let alone = ubo.in_array(dword);
                for c in 0..comps {
                    let at = if alone {
                        Idx::Ubo { base: base + u16::from(c), comp: 0 }
                    } else {
                        Idx::Ubo { base, comp: c }
                    };
                    lanes[id][c as usize] = Src::of(at);
                }
            }
            nir::Kind::Sysval(sv) => {
                for c in 0..3u8 {
                    lanes[id][c as usize] = Src::of(Idx::Sysval(sv, c));
                }
            }
            /* the register itself; [`preloads`] makes its `MOV.i32` */
            nir::Kind::Preload { reg } => lanes[id][0] = Src::of(Idx::Reg(reg)),
            nir::Kind::Alu(op) => {
                let mut s = [Src::NULL; 3];
                for (k, x) in s.iter_mut().enumerate().take(op.arity()) {
                    *x = src(lanes, nd.src[k])?;
                }
                lanes[id][0] = Src::ssa(emit::emit(p, emit::Row { op, bits: nd.bits, width: nd.width }, &s)?);
            }
            nir::Kind::Lea { out: o } => {
                let access = if o == 1 { Access::Estream } else { Access::Istream };
                let st = outs.get_mut(o as usize).ok_or(MaliError::Defect(super::Defect::IrIndex))?;
                *st = Some(Out { lea: lea(p, o == 2)?, la: None, access, half: o == 2 });
            }
            nir::Kind::Addr { out: o, word, swapped } => {
                let st = outs
                    .get_mut(o as usize)
                    .and_then(|x| x.as_mut())
                    .ok_or(MaliError::Defect(super::Defect::IrIndex))?;
                let [lo, hi] = address(p, st, word, swapped)?;
                lanes[id][0] = Src::ssa(lo);
                lanes[id][1] = Src::ssa(hi);
            }
            nir::Kind::VsTex { explicit, cube } => {
                /* `bi_emit_tex_valhall` (bifrost_compile.c:3488-3574): the
                 * staging words collected (the coordinate, or a cube's
                 * descriptor pair, then the level), TEX_SINGLE of the
                 * handle at the texture's dimension, its fp32 result split;
                 * a staging word not in a register is moved there first */
                let n = if explicit { 3 } else { 2 };
                let mut sr = [Src::NULL; 3];
                for (k, x) in sr.iter_mut().enumerate().take(n) {
                    let s = src(lanes, nd.src[k])?;
                    *x = if s.ssa_value().is_some() { s } else { Src::ssa(alu(p, Op::MovI32, &[s])?) };
                }
                let c = alu(p, Op::Collect, &sr[..n])?;
                let lod = if explicit { LodMode::Explicit } else { LodMode::Zero };
                let dim = if cube { TexDim::Cube } else { TexDim::D2 };
                let op = Op::TexSingle { sr_count: n as u8, mask: 0xf, f16: false, skip: false, lod, dim };
                let handle = Src::of(Idx::Const(super::nir_fs::TEX_HANDLE));
                let r = alu(p, op, &[Src::ssa(c), handle, Src::of(Idx::Const(0))])?;
                let w = split(p, r, 4)?;
                for (k, x) in lanes[id].iter_mut().enumerate() {
                    *x = Src::ssa(w[k]);
                }
            }
            nir::Kind::UniformIdx { dword, .. } => {
                let off = ubo_offset(p, ubo, dword, src(lanes, nd.src[0])?, &mut 0)?;
                lanes[id][0] = Src::ssa(ubo_fetch(p, off)?);
            }
            nir::Kind::Tex | nir::Kind::PixelCoord | nir::Kind::FragCoordZw | nir::Kind::Phi | nir::Kind::Discard => {
                return Err(MaliError::Defect(super::Defect::IrIndex))
            }
            nir::Kind::Store { out: o, word } => {
                let data = src(lanes, nd.src[0])?;
                let st =
                    outs.get(o as usize).and_then(|x| x.as_ref()).ok_or(MaliError::Defect(super::Defect::IrIndex))?;
                let at = match nd.src[1] {
                    nir::Val::Lane(a, 0) if word > 0 => {
                        let (lo, hi) = (lanes[a as usize][0], lanes[a as usize][1]);
                        match (lo.ssa_value(), hi.ssa_value()) {
                            (Some(lo), Some(hi)) => [lo, hi],
                            _ => return Err(MaliError::Defect(super::Defect::IrIndex)),
                        }
                    }
                    nir::Val::Undef if word == 0 => st.lea,
                    _ => return Err(MaliError::Defect(super::Defect::IrIndex)),
                };
                store(p, st, at, data)?;
            }
        }
    }
    preloads(p)
}

/// The byte offset of an fp32 float array element at a run-time index
/// ([`nir::uniform_idx`], array at front-end dword `dword`, `idx` the int
/// uniform's FAU word): NIR's `ishl idx, 2` (`LSHIFT_OR.i32 r0, u5.w1,
/// 0x3020100.b22, 0x0`), then `iadd base, that` for an array past the
/// block's first word (`IADD_IMM.i32 r0, r0^, #0x4` after
/// `va_fuse_add_imm`; uniarray_vs.expect.json, uniarray_highp.expect.json),
/// each row noted in `rows` (the fragment record's rows drawn).
pub(super) fn ubo_offset(
    p: &mut Prog<'_>,
    ubo: &UboLayout,
    dword: u16,
    idx: Src,
    rows: &mut u64,
) -> Result<u16, MaliError> {
    let base = ubo.word(dword).ok_or(MaliError::Defect(super::Defect::IrIndex))?;
    let mut row = |p: &mut Prog<'_>, op, s: &[Src]| {
        let r = emit::Row { op, bits: 32, width: 1 };
        *rows |= emit::bit(r);
        emit::emit(p, r, s)
    };
    let mut off = row(p, nir::Alu::Ishl, &[idx, Src::of(Idx::Const(2))])?;
    if base > 0 {
        off = row(p, nir::Alu::Iadd, &[Src::of(Idx::Const(4 * u32::from(base))), Src::ssa(off)])?;
    }
    Ok(off)
}

/// The element's load at its byte offset ([`ubo_offset`]): `LD_PKA.i32` of
/// UBO 0 (the mode descriptor the constant 0), one fp32 word.
pub(super) fn ubo_fetch(p: &mut Prog<'_>, off: u16) -> Result<u16, MaliError> {
    alu(p, Op::LdPka { words: 1, half: false }, &[Src::ssa(off), Src::of(Idx::Const(0))])
}

/// `bi_preload` (bifrost_compile.c): each preloaded register is read once,
/// through a `MOV.i32` inserted at the start of the program when it is
/// first used, so the moves stand in the reverse order of first use.
pub(super) fn preloads(p: &mut Prog<'_>) -> Result<(), MaliError> {
    let mut made: [(u8, u16); 8] = [(0, 0); 8];
    let mut n = 0usize;
    for i in 0..p.n {
        for s in 0..p.ins[i].ns as usize {
            let Idx::Reg(r) = p.ins[i].src[s].idx else { continue };
            let v = match made[..n].iter().find(|(reg, _)| *reg == r) {
                Some(&(_, v)) => v,
                None => {
                    let v = p.temp()?;
                    made[n] = (r, v);
                    n += 1;
                    v
                }
            };
            p.ins[i].src[s].idx = Idx::Ssa(v, 0);
        }
    }
    for &(r, v) in &made[..n] {
        p.insert(0, Ins::new(Op::MovI32, &[Dest::ssa(v)], &[Src::of(Idx::Reg(r))]))?;
    }
    Ok(())
}
