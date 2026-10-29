// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2015-2025 Intel Corporation, Connor Abbott, Valve Corporation
// Derived from Mesa 26.2.1 src/compiler/nir/nir_opt_vectorize.c,
// nir_opt_copy_propagate.c and nir_opt_move.c (MIT). The upstream part
// stays under its licence (LICENSES/MIT.txt); our changes are Apache-2.0.
// See NOTICE.
//! The 16-bit lane pairs of a mediump shader with blocks (a loop Mesa
//! keeps, an `if` it keeps or flattens), as Mesa's NIR has them when
//! instruction selection starts.
//!
//! Mesa's GL path keeps a mediump shader's values scalar until the back
//! end: `f2fmp` of each varying lane, scalar fp16 arithmetic, one 16-bit
//! phi per lane (`nir_lower_phis_to_scalar`), and the swizzles of a carried
//! vector are plain phi sources (`res = res.yzwx` makes the lane phis read
//! each other). The pairs appear only in `bi_optimize_late`
//! (bifrost_nir.c:350-352): after `pan_nir_lower_bool_to_bitsize`, one
//! `nir_opt_vectorize` with `bi_vectorize_filter` (width 2 at 16 bits)
//! pairs the conversions, the arithmetic and the phis, block by block in
//! layout order, each pair where the first of its two stood; a vectorised
//! phi's back-edge source is a `vec2` of the two lanes it was made of,
//! appended to the latch (nir_opt_vectorize.c `instr_try_combine_phi`,
//! `nir_vec_scalars` at `nir_after_block_before_jump`), which
//! `nir_opt_copy_prop` folds away when it is the identity swizzle of one
//! value, and which `nir_opt_move` (`nir_move_copies`, bifrost_nir.c:
//! 375-381) then sinks to the end of its block in its own order
//! (nir_opt_move.c:46-118: the copies are walked backwards, one with no
//! user in its block goes to the end, before the one moved there last).
//! Measured on the per-pass NIR (`NIR_DEBUG=print`) of intloop_loop_nested
//! and intloop_loop_nested_vs: the final NIR's `vec2 %13.y, %12.x` before
//! `vec2 %12.y, %13.x` in a latch, the duplicated `vec2`s of two phis with
//! equal back-edge values (no CSE runs after), and the pairs' lane order
//! sorted by the forward source's components (`dest_swizzle`).
//!
//! The passes here work on the shared graph ([`Nir`]), after the fragment
//! walk and the NIR-level passes of nir_fs.rs `select_highp` (which end
//! where Mesa's `nir_opt_algebraic_late` does), in Mesa's order:
//!
//! 1. [`narrow_consts`]: the fp16 constants of the 16-bit nodes (the
//!    mediump walk keeps them as fp32 bits, which the pair path converts
//!    at its own selection);
//! 2. [`distribute`]: nir_opt_algebraic.py:246 on the 16-bit sums of two
//!    products (the optimisation loop's rule, before the late fusion);
//! 3. [`convert_inputs`]: the `f2fmp` of each varying lane a 16-bit node
//!    reads, right after the load, lane by lane;
//! 4. [`vectorise`]: `nir_opt_vectorize` at width 2 over the 16-bit
//!    operations and phis;
//! 5. [`copy_prop`]: a `vec2` of one value's two lanes in order is that
//!    value;
//! 6. [`move_copies`]: `nir_opt_move` of the `vec2`s and the 16-bit
//!    `mov`s within their blocks.
//!
//! A two-lane node's source is [`Val::Pair`] (one value's two lanes
//! through a swizzle: `a.xy`, `a.yx`, `a.xx`, `a.yy`), a two-lane
//! constant (both halves in one word), or, for `vec2` and the `f2f16` of
//! two 32-bit lanes, one [`Val::Lane`] per lane.

use super::bir::Swz;
use super::nir::{Alu, Kind, Nir, NirNode, Val, MAX_NODES};
use super::select::f16_rtne;
use super::MaliError;

/// The lanes one vectorised 16-bit node holds (`bi_vectorize_filter`: 32 /
/// 16 at 16 bits, bifrost_nir.c:177).
const WIDTH: u8 = 2;

/// The origin [`vectorise`] gives a node it makes at a block's end (a
/// back-edge `vec2`): no IR instruction; nir_fs.rs `select_highp` orders it
/// after every node of its block that has one.
pub const ORIGIN_BLOCK_END: u16 = u16::MAX;

/// Node `n` is a 16-bit float value: an fp16 operation or phi, or a `vec2`
/// of halves.
fn is_f16(nir: &Nir<'_>, n: u16) -> bool {
    let nd = nir.nodes[n as usize];
    nd.bits == 16 && matches!(nd.kind, Kind::Phi | Kind::Alu(_)) && !matches!(nd.kind, Kind::Alu(op) if op.is_int())
}

/// The fp16 constants of the 16-bit float nodes (`_mesa_float_to_half`,
/// round to nearest even, as GLSL's lower_precision converts a mediump
/// operation's constant, lower_precision.cpp:993-1010): each fp32 constant
/// source of an fp16 operation or phi becomes its half. A `bcsel`'s
/// condition (a 32-bit mask) is left; a constant fp16 does not hold is
/// refused ([`MaliError::Fp16Inexact`]).
pub fn narrow_consts(nir: &mut Nir<'_>) -> Result<(), MaliError> {
    for k in 0..nir.len {
        let id = nir.order[k] as usize;
        if !is_f16(nir, id as u16) {
            continue;
        }
        let nd = nir.nodes[id];
        let first = usize::from(matches!(nd.kind, Kind::Alu(Alu::Bcsel)));
        for s in first..nd.arity() {
            if let Val::Const(c) = nd.src[s] {
                let h = f16_rtne(c).ok_or(MaliError::Fp16Inexact)?;
                nir.nodes[id].src[s] = Val::Const(u32::from(h));
            }
        }
    }
    Ok(())
}

/// The product node lane `v` is, if it is a 16-bit `fmul`: its two sources.
fn product(nir: &Nir<'_>, v: Val) -> Option<(u16, [Val; 2])> {
    match v {
        Val::Lane(n, 0) => match nir.nodes[n as usize] {
            NirNode { kind: Kind::Alu(Alu::Fmul), src, bits: 16, width: 1 } => Some((n, [src[0], src[1]])),
            _ => None,
        },
        _ => None,
    }
}

/// `fadd(fmul(a, b), fmul(a, c))` is `fmul(a, fadd(b, c))`
/// (nir_opt_algebraic.py:246) on the 16-bit sums, in nir_search's order of
/// the commutative combinations (nir_search.c `match_expression`: the
/// pattern's commutative expressions are numbered in pre-order, the `fadd`
/// 0 and its two `fmul`s 1 and 2, and combination `k` swaps the sources of
/// expression `e` when bit `e` of `k` is set; the first combination that
/// matches wins), the new `fadd` before the old one, which becomes the
/// `fmul`. Measured: intloop_loop_nested's `a.x * 0.25 + b.y * 0.25 + c.z *
/// 0.25 + res.w * 0.25` is `fmul(0.25, fadd(res.w, fadd(c.z, fadd(a.x,
/// b.y))))`: the first sum keeps its order (combination 6, both products
/// swapped to put the constant first), the later ones take the new term
/// first (combination 3, the `fadd` and the first `fmul` swapped).
pub fn distribute(nir: &mut Nir<'_>, origin: &mut [u16]) -> Result<(), MaliError> {
    let mut progress = true;
    while progress {
        progress = false;
        let mut k = 0;
        while k < nir.len {
            let id = nir.order[k] as usize;
            let nd = nir.nodes[id];
            k += 1;
            if nd.kind != Kind::Alu(Alu::Fadd) || nd.bits != 16 || nd.width != 1 {
                continue;
            }
            let (Some((_, px)), Some((_, py))) = (product(nir, nd.src[0]), product(nir, nd.src[1])) else { continue };
            let flip = |p: [Val; 2], on: bool| if on { [p[1], p[0]] } else { p };
            let found = (0..8u8).find_map(|comb| {
                let (x, y) = if comb & 1 != 0 { (py, px) } else { (px, py) };
                let [a, b] = flip(x, comb & 2 != 0);
                let [a2, c] = flip(y, comb & 4 != 0);
                (a == a2).then_some((a, b, c))
            });
            let Some((a, b, c)) = found else { continue };
            let sum = NirNode { kind: Kind::Alu(Alu::Fadd), src: [b, c, Val::Undef], bits: 16, width: 1 };
            let s = nir.insert_before(k - 1, sum)?;
            origin[s as usize] = origin[id];
            nir.nodes[id] = NirNode { kind: Kind::Alu(Alu::Fmul), src: [a, Val::Lane(s, 0), Val::Undef], ..nd };
            k += 1;
            progress = true;
        }
    }
    Ok(())
}

/// The `f2fmp` of each 32-bit input lane a 16-bit node reads (GLSL's
/// lower_precision narrows a mediump varying where it is read, the
/// conversion emitted where the read stood; measured: intloop_loop_nested's
/// `%5..%8 = f2fmp %4.x..w` right after the load): one scalar `f2f16` node
/// per lane, in lane order, right after the input node; the readers read
/// it. A 32-bit reader (an integer conversion) keeps the lane.
pub fn convert_inputs(nir: &mut Nir<'_>, origin: &mut [u16]) -> Result<(), MaliError> {
    let mut k = 0;
    while k < nir.len {
        let input = nir.order[k];
        k += 1;
        let NirNode { kind: Kind::Input { .. }, bits: 32, .. } = nir.nodes[input as usize] else { continue };
        let mut at = k;
        for c in 0..4u8 {
            let read = (0..nir.len).any(|j| {
                let u = nir.order[j];
                is_f16(nir, u) && nir.nodes[u as usize].src[..nir.nodes[u as usize].arity()].contains(&Val::Lane(input, c))
            });
            if !read {
                continue;
            }
            let conv = NirNode {
                kind: Kind::Alu(Alu::F2f16),
                src: [Val::Lane(input, c), Val::Undef, Val::Undef],
                bits: 16,
                width: 1,
            };
            let n = nir.insert_at(at, conv, nir.block[input as usize])?;
            origin[n as usize] = origin[input as usize];
            at += 1;
            for j in 0..nir.len {
                let u = nir.order[j] as usize;
                if u == n as usize || !is_f16(nir, u as u16) {
                    continue;
                }
                let arity = nir.nodes[u].arity();
                for s in nir.nodes[u].src.iter_mut().take(arity) {
                    if *s == Val::Lane(input, c) {
                        *s = Val::Lane(n, 0);
                    }
                }
            }
        }
        k = at;
    }
    Ok(())
}

/// `bi_vectorize_filter` (bifrost_nir.c:107-179) for a node: the lanes it
/// may be vectorised to. A phi is its bit size's; an operation whose
/// sources are wider than its result (an `i2i16` of a 32-bit mask) stays
/// scalar (`MAX2(def.bit_size, src_bit_size)`), but the `f2f16` of 32-bit
/// lanes is two on v10 (:148-154).
fn filter(nir: &Nir<'_>, nd: &NirNode) -> u8 {
    match nd.kind {
        Kind::Phi => {
            if nd.bits == 16 {
                WIDTH
            } else {
                1
            }
        }
        Kind::Alu(Alu::F2f16) => WIDTH,
        Kind::Alu(op) => {
            if op.is_int() || nd.bits != 16 {
                return 1;
            }
            /* a 32-bit source widens the operation's size: a `bcsel`'s
             * condition does not count here, it is `i2i16`'d to 16 bits in
             * Mesa's NIR by then (pan_nir_lower_bool_to_bitsize) */
            let first = usize::from(op == Alu::Bcsel);
            let wide = nd.src[first..nd.arity()].iter().any(|v| match *v {
                Val::Lane(n, _) => nir.nodes[n as usize].bits == 32,
                _ => false,
            });
            if wide {
                1
            } else {
                WIDTH
            }
        }
        _ => 0,
    }
}

/// `instr_can_rewrite` (nir_opt_vectorize.c:274-330): a scalar 16-bit
/// operation other than a `vec` or a `mov`, or a scalar 16-bit phi.
fn can_rewrite(nir: &Nir<'_>, id: u16) -> bool {
    let nd = nir.nodes[id as usize];
    if nd.width >= WIDTH || filter(nir, &nd) < WIDTH {
        return false;
    }
    match nd.kind {
        Kind::Alu(Alu::Vec2 | Alu::Mov16) => false,
        Kind::Alu(_) | Kind::Phi => true,
        _ => false,
    }
}

/// A scalar source as (value, lane) for the hash's swizzle mask, or a
/// constant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Const,
    Lane(u16, u8),
}

fn scalar(v: Val) -> Option<Scalar> {
    match v {
        Val::Const(_) => Some(Scalar::Const),
        Val::Lane(n, c) => Some(Scalar::Lane(n, c)),
        /* a one-lane node reads no pair */
        Val::Pair(..) | Val::Undef => None,
    }
}

/// `alu_srcs_equal` (:166-175): the same value with the lanes in one pair
/// (`swizzle & ~(max_vec - 1)`), or two constants.
fn alu_srcs_equal(a: Val, b: Val) -> bool {
    match (scalar(a), scalar(b)) {
        (Some(Scalar::Const), Some(Scalar::Const)) => true,
        (Some(Scalar::Lane(n, c)), Some(Scalar::Lane(m, d))) => n == m && (c & !(WIDTH - 1)) == (d & !(WIDTH - 1)),
        _ => false,
    }
}

/// `phi_srcs_equal` (:177-223) for the sources from one predecessor: two
/// constants; a forward edge's values must be one value's lanes in one
/// pair; a back edge's (not vectorised yet) the same kind of instruction,
/// the same operation when ALU.
fn phi_srcs_equal(nir: &Nir<'_>, a: Val, b: Val, back: bool) -> bool {
    match (scalar(a), scalar(b)) {
        (Some(Scalar::Const), Some(Scalar::Const)) => true,
        (Some(Scalar::Lane(n, c)), Some(Scalar::Lane(m, d))) => {
            if (c & !(WIDTH - 1)) != (d & !(WIDTH - 1)) {
                return false;
            }
            if !back {
                return n == m;
            }
            match (nir.nodes[n as usize].kind, nir.nodes[m as usize].kind) {
                (Kind::Alu(x), Kind::Alu(y)) => x == y,
                (Kind::Phi, Kind::Phi) => true,
                (x, y) => core::mem::discriminant(&x) == core::mem::discriminant(&y) && !matches!(x, Kind::Alu(_)),
            }
        }
        _ => false,
    }
}

/// `instrs_equal` (:225-272).
fn instrs_equal(nir: &Nir<'_>, a: u16, b: u16, back: &dyn Fn(u8, usize) -> bool) -> bool {
    let (x, y) = (nir.nodes[a as usize], nir.nodes[b as usize]);
    if x.bits != y.bits {
        return false;
    }
    match (x.kind, y.kind) {
        (Kind::Phi, Kind::Phi) => {
            let block = nir.block[a as usize];
            if block != nir.block[b as usize] {
                return false;
            }
            (0..2).all(|i| phi_srcs_equal(nir, x.src[i], y.src[i], back(block, i)))
        }
        (Kind::Alu(p), Kind::Alu(q)) => p == q && (0..x.arity()).all(|s| alu_srcs_equal(x.src[s], y.src[s])),
        _ => false,
    }
}

/// The swizzle reading lanes `a` then `b` of one value.
fn swz(a: u8, b: u8) -> Swz {
    match (a & 1, b & 1) {
        (0, 0) => Swz::H00,
        (0, _) => Swz::H01,
        (_, 0) => Swz::H10,
        _ => Swz::H11,
    }
}

/// Every read of lane 0 of scalar node `from` becomes lane `to_lane` of
/// node `to` (`nir_def_rewrite_uses_with_alu_src`, `rewrite_uses`: an ALU
/// user's swizzle moves; a phi's source is the lane, which a later
/// vectorisation of the phi chases, or a 16-bit `mov` if it stays scalar,
/// made by [`vectorise`] after the pass). A two-lane user reading `from`
/// in both lanes reads `to`'s lane in both.
fn redirect(nir: &mut Nir<'_>, from: u16, to: u16, to_lane: u8) {
    for k in 0..nir.len {
        let u = nir.order[k] as usize;
        let arity = nir.nodes[u].arity();
        for s in nir.nodes[u].src.iter_mut().take(arity) {
            match *s {
                Val::Lane(n, 0) if n == from => *s = Val::Lane(to, to_lane),
                Val::Pair(n, _) if n == from => *s = Val::Pair(to, swz(to_lane, to_lane)),
                _ => {}
            }
        }
    }
    for r in nir.ifs[..nir.nifs].iter_mut() {
        if r.cond == Val::Lane(from, 0) {
            r.cond = Val::Lane(to, to_lane);
        }
    }
}

/// `instr_try_combine_alu` (:548-612) of `old` and `new` (scalar, equal):
/// the two-lane node after `old`, each source the pair of the two (a
/// constant pair, or one value's two lanes; the `f2f16` of 32-bit lanes
/// keeps one source a lane), `old` lane 0 and `new` lane 1.
fn combine_alu(nir: &mut Nir<'_>, origin: &mut [u16], old: u16, new: u16) -> Result<u16, MaliError> {
    let (x, y) = (nir.nodes[old as usize], nir.nodes[new as usize]);
    let Kind::Alu(op) = x.kind else { return Err(MaliError::Defect(super::Defect::IrIndex)) };
    let mut src = [Val::Undef; 3];
    for s in 0..x.arity() {
        src[s] = match (x.src[s], y.src[s]) {
            (Val::Const(a), Val::Const(b)) => Val::Const((a & 0xffff) | (b & 0xffff) << 16),
            (Val::Lane(n, a), Val::Lane(m, b)) if n == m => {
                if nir.nodes[n as usize].bits == 32 && op == Alu::F2f16 {
                    /* the two 32-bit lanes, one source each */
                    src[1] = Val::Lane(m, b);
                    Val::Lane(n, a)
                } else {
                    Val::Pair(n, swz(a, b))
                }
            }
            _ => return Err(MaliError::Defect(super::Defect::IrIndex)),
        };
        if op == Alu::F2f16 {
            break;
        }
    }
    let node = NirNode { kind: x.kind, src, bits: 16, width: WIDTH };
    let at = nir.position(old).ok_or(MaliError::Defect(super::Defect::IrIndex))? + 1;
    let id = nir.insert_at(at, node, nir.block[old as usize])?;
    origin[id as usize] = origin[old as usize];
    redirect(nir, old, id, 0);
    redirect(nir, new, id, 1);
    nir.remove_from_order(old);
    nir.remove_from_order(new);
    Ok(id)
}

/// `instr_try_combine_phi` (:430-546) of two scalar phis of one block: the
/// two-lane phi after `old`, its lanes in the order of the components its
/// first forward, non-constant source reads (`dest_swizzle`, sorted by
/// `compare_scalar`: the swizzle of that source stays in order), each
/// source a constant pair, one value's two lanes (a forward edge) or a
/// `vec2` of the two lanes appended to the predecessor (a back edge, whose
/// values are not vectorised yet).
fn combine_phi(
    nir: &mut Nir<'_>,
    origin: &mut [u16],
    old: u16,
    new: u16,
    back: &dyn Fn(u8, usize) -> bool,
) -> Result<u16, MaliError> {
    let (x, y) = (nir.nodes[old as usize], nir.nodes[new as usize]);
    let block = nir.block[old as usize];
    /* dest_swizzle: lane 0 is `old`'s, lane 1 `new`'s, unless the first
     * forward non-constant source has them the other way round */
    let mut dest = [0u8, 1u8];
    for i in 0..2 {
        if back(block, i) {
            continue;
        }
        let (Val::Lane(_, a), Val::Lane(_, b)) = (x.src[i], y.src[i]) else { continue };
        if b < a {
            dest = [1, 0];
        }
        break;
    }
    let srcs_in_order = |i: usize| -> [Val; 2] {
        let s = [x.src[i], y.src[i]];
        [s[dest[0] as usize], s[dest[1] as usize]]
    };
    let mut src = [Val::Undef; 3];
    for i in 0..2 {
        let [a, b] = srcs_in_order(i);
        src[i] = match (a, b) {
            (Val::Const(p), Val::Const(q)) => Val::Const((p & 0xffff) | (q & 0xffff) << 16),
            (Val::Lane(n, p), Val::Lane(m, q)) if !back(block, i) => {
                if n != m {
                    return Err(MaliError::Defect(super::Defect::IrIndex));
                }
                Val::Pair(n, swz(p, q))
            }
            (Val::Lane(..), Val::Lane(..)) => {
                /* the loop's latch: the block before this one in the
                 * header's predecessors is its `pre`, the other the latch */
                let latch = nir.loops[..nir.nloops]
                    .iter()
                    .find(|l| l.header == block)
                    .map(|l| l.latch)
                    .ok_or(MaliError::Defect(super::Defect::IrIndex))?;
                let vec = NirNode { kind: Kind::Alu(Alu::Vec2), src: [a, b, Val::Undef], bits: 16, width: WIDTH };
                let v = nir.insert_at_block_end(latch, vec)?;
                origin[v as usize] = ORIGIN_BLOCK_END;
                Val::Pair(v, Swz::H01)
            }
            _ => return Err(MaliError::Defect(super::Defect::IrIndex)),
        };
    }
    let node = NirNode { kind: Kind::Phi, src, bits: 16, width: WIDTH };
    let at = nir.position(old).ok_or(MaliError::Defect(super::Defect::IrIndex))? + 1;
    let id = nir.insert_at(at, node, block)?;
    origin[id as usize] = origin[old as usize];
    /* `replace_phi1.swizzle[comp] = i`: `old` is the lane `dest` puts
     * component 0 at */
    let lane_of = |comp: u8| dest.iter().position(|&d| d == comp).unwrap_or(0) as u8;
    redirect(nir, old, id, lane_of(0));
    redirect(nir, new, id, lane_of(1));
    nir.remove_from_order(old);
    nir.remove_from_order(new);
    Ok(id)
}

/// `nir_opt_vectorize` (nir_opt_vectorize.c:688-710) at width 2 over the
/// 16-bit scalars: the blocks in layout order, each node that may be
/// vectorised against the set of the ones seen (`vec_instr_set_add_or_rewrite`,
/// :650-686): an equal one that dominates it is combined with it (the
/// result replaces neither in the set: it is already two lanes), an equal
/// one that does not dominate is replaced by it. `back(block, i)` says
/// whether source `i` of a phi of `block` comes over a back edge.
pub fn vectorise(nir: &mut Nir<'_>, origin: &mut [u16], back: &dyn Fn(u8, usize) -> bool) -> Result<(), MaliError> {
    let mut set = [0u16; MAX_NODES];
    let mut nset = 0usize;
    let mut k = 0;
    while k < nir.len {
        let id = nir.order[k];
        k += 1;
        if !can_rewrite(nir, id) {
            continue;
        }
        let found = set[..nset].iter().position(|&e| instrs_equal(nir, e, id, back));
        let Some(e) = found else {
            if nset == set.len() {
                return Err(MaliError::TooManyValues);
            }
            set[nset] = id;
            nset += 1;
            continue;
        };
        let old = set[e];
        if !nir.dominates(nir.block[old as usize], nir.block[id as usize]) {
            set[e] = id;
            continue;
        }
        set.copy_within(e + 1..nset, e);
        nset -= 1;
        /* the combined node stands after `old`, before this position, and
         * the two scalars leave the order (`nir_foreach_instr_safe`
         * continues at the node after `id`); a back edge's `vec2` stands in
         * a later block */
        let next = (k < nir.len).then(|| nir.order[k]);
        if nir.nodes[id as usize].kind == Kind::Phi {
            combine_phi(nir, origin, old, id, back)?;
        } else {
            combine_alu(nir, origin, old, id)?;
        }
        k = next.and_then(|n| nir.position(n)).unwrap_or(nir.len);
    }
    Ok(())
}

/// `nir_opt_copy_prop` for the `vec2`s the vectorisation made: a `vec2`
/// of lanes 0 and 1 of one two-lane value is that value
/// (`is_swizzleless_move`, nir_opt_copy_propagate.c:35-57); its readers
/// read the value and it leaves the order.
pub fn copy_prop(nir: &mut Nir<'_>) {
    let mut k = 0;
    while k < nir.len {
        let id = nir.order[k];
        k += 1;
        let nd = nir.nodes[id as usize];
        let (Kind::Alu(Alu::Vec2), [Val::Lane(n, 0), Val::Lane(m, 1), _]) = (nd.kind, nd.src) else { continue };
        if n != m || nir.nodes[n as usize].width != WIDTH {
            continue;
        }
        for j in 0..nir.len {
            let u = nir.order[j] as usize;
            let arity = nir.nodes[u].arity();
            for s in nir.nodes[u].src.iter_mut().take(arity) {
                match *s {
                    Val::Lane(x, c) if x == id => *s = Val::Lane(n, c),
                    Val::Pair(x, w) if x == id => *s = Val::Pair(n, w),
                    _ => {}
                }
            }
        }
        nir.remove_from_order(id);
        k -= 1;
    }
}

/// A copy `nir_opt_move` moves (`nir_can_move_instr` with
/// `nir_move_copies`, nir_opt_sink.c: a `vec` or a `mov`).
fn is_copy(nd: &NirNode) -> bool {
    matches!(nd.kind, Kind::Alu(Alu::Vec2 | Alu::Mov16))
}

/// `nir_opt_move_block` (nir_opt_move.c:46-118) for the copies of each
/// block: the block's nodes walked backwards, each copy moved before its
/// first user in the block (several with one user keep their order), or,
/// with no user in the block (a phi is none), to the block's end: after the
/// last node, or before the copy moved there last (`last_instr`).
pub fn move_copies(nir: &mut Nir<'_>) {
    for b in 0..nir.nblocks {
        /* the block's run in the order */
        let Some(start) = (0..nir.len).find(|&k| nir.block[nir.order[k] as usize] == b) else { continue };
        let end = (start..nir.len).take_while(|&k| nir.block[nir.order[k] as usize] == b).count() + start;
        if end <= start + 1 {
            continue;
        }
        let mut index = [0u32; MAX_NODES];
        let mut next = 1u32;
        let mut last_instr: Option<u16> = None;
        let mut k = end;
        while k > start {
            k -= 1;
            let id = nir.order[k];
            index[id as usize] = next;
            next += 1;
            if !is_copy(&nir.nodes[id as usize]) {
                continue;
            }
            /* the first user in the block: the one with the largest index
             * (the indices fall along the block) */
            let mut first_user = last_instr;
            for j in start..end {
                let u = nir.order[j];
                let nd = nir.nodes[u as usize];
                if nd.kind == Kind::Phi {
                    continue;
                }
                let reads = nd.src[..nd.arity()].iter().any(|s| matches!(*s, Val::Lane(n, _) | Val::Pair(n, _) if n == id));
                if reads && first_user.is_none_or(|f| index[u as usize] > index[f as usize]) {
                    first_user = Some(u);
                }
            }
            if let Some(mut f) = first_user {
                /* keep the order among the ones with one user */
                loop {
                    let Some(pf) = nir.position(f) else { break };
                    if pf == start {
                        break;
                    }
                    let prev = nir.order[pf - 1];
                    if index[prev as usize] != index[f as usize] {
                        break;
                    }
                    f = prev;
                }
                let pf = nir.position(f).unwrap_or(k);
                if pf > start && nir.order[pf - 1] == id {
                    continue;
                }
                /* out of `k`, in before `f` */
                nir.order.copy_within(k + 1..pf, k);
                nir.order[pf - 1] = id;
                index[id as usize] = index[f as usize];
                continue;
            }
            /* no user in the block: to the end */
            if k + 1 == end {
                continue;
            }
            nir.order.copy_within(k + 1..end, k);
            nir.order[end - 1] = id;
            index[id as usize] = 0;
            last_instr = Some(id);
        }
    }
}

/// [`vectorise`] followed by the passes Mesa runs after it: [`copy_prop`]
/// and [`move_copies`] (nir_opt_sink moves none of these: a copy sinks to
/// the block of its uses, which the latch's `vec2`s, read by the header's
/// phis alone, already are).
pub fn run(nir: &mut Nir<'_>, origin: &mut [u16], back: &dyn Fn(u8, usize) -> bool) -> Result<(), MaliError> {
    convert_inputs(nir, origin)?;
    vectorise(nir, origin, back)?;
    copy_prop(nir);
    move_copies(nir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzles_of_two_lanes() {
        assert_eq!(swz(0, 1), Swz::H01);
        assert_eq!(swz(1, 0), Swz::H10);
        assert_eq!(swz(0, 0), Swz::H00);
        assert_eq!(swz(1, 1), Swz::H11);
    }

    /// The combination order of nir_opt_algebraic.py:246 as nir_search
    /// tries it: `fadd(fmul(a, k), fmul(b, k))` keeps `a, b` (combination
    /// 6), `fadd(fmul(k, s), fmul(c, k))` takes `c, s` (combination 3).
    #[test]
    fn distributive_combinations() {
        let (k, a, b, c, s) = (Val::Const(1), Val::Lane(1, 0), Val::Lane(2, 0), Val::Lane(3, 0), Val::Lane(4, 0));
        let flip = |p: [Val; 2], on: bool| if on { [p[1], p[0]] } else { p };
        let first = |px: [Val; 2], py: [Val; 2]| {
            (0..8u8).find_map(|comb| {
                let (x, y) = if comb & 1 != 0 { (py, px) } else { (px, py) };
                let [a, b] = flip(x, comb & 2 != 0);
                let [a2, c] = flip(y, comb & 4 != 0);
                (a == a2).then_some((comb, a, b, c))
            })
        };
        assert_eq!(first([a, k], [b, k]), Some((6, k, a, b)));
        assert_eq!(first([k, s], [c, k]), Some((3, k, c, s)));
    }
}
