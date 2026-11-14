// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 128-bit colour outputs on the GC7000: a store to an RGBA32F / RGBA32I /
//! RGBA32UI render target is two stores, one per G32R32 plane.
//!
//! The pixel engine has no 128-bit colour format. Mesa 26.2.1's etnaviv
//! draws such a target as two G32R32 planes (R,G and B,A), each a hardware
//! target of its own, and rewrites the fragment shader's store to the
//! output into two: `vec4(r,g,0,0)` to the output and `vec4(b,a,0,0)` to a
//! companion output, each under its half of the write mask (`wm & 3`,
//! `wm >> 2`), with no conversion so an integer output stays integer
//! (`etnaviv_nir_lower_128bit.c:15-87,172-214`). This pass is the compiler
//! half of that, the fragment analogue of [`super::wide128`]'s look-up
//! join, run on the GC7000's module before selection
//! (docs/notes/gc-128bit-rt-shader.md section 1). A store to an output `n`
//! in [`RtPlanes::mask`] becomes
//!
//! ```text
//!   lo.zw = 0 ; lo.xy = value.xy  -- under wm & 3
//!   out[n] = lo                   -- R,G
//!   hi.zw = 0 ; hi.xy = value.zw  -- under wm >> 2
//!   out[4 + k] = hi               -- B,A; k = n's rank in the mask
//! ```
//!
//! the lane-masked stores into a compiler temporary of the output's own
//! type, so selection sees nothing new. Lanes the store does not write are
//! zero: a scalar or `vec3` output zero-extends, and an output whose B,A
//! half the store does not reach still stores the zero plane, so the
//! companion output is always written.
//!
//! The IR's [`Op::StoreOutput`] has no write mask of its own: the front end
//! stores an output whole, holding one it writes in parts in a local and
//! storing that once at the end (`lower.rs` `shadow_outputs`). The store's
//! write mask is then its declared width (`.x` for a scalar, `.xyz` for a
//! `vec3`); [`split_write_mask`] is the rule either way.
//!
//! The companion's index is `4 + k`, `k` the output's rank among the
//! 128-bit ones (Mesa's `next_output`, which starts at the four render
//! targets this core has): a property of the key's mask, not of the
//! hardware slot the plane lands in, which is the framebuffer's.
//!
//! The key is per draw ([`set_compile_rt_planes`], empty in every compile
//! that does not ask): with an empty key, or one naming only outputs the
//! shader does not write, the pass returns the module untouched. Nothing
//! calls the pass yet (docs/notes/gc-128bit-rt-shader.md, "Sequence of
//! reviewable items" item 1), so no shader the driver ships changes by a
//! byte.

use super::super::arena::{AVec, Arena};
use super::super::ir::{
    self, InstrId, IoKind, IoSlot, LocalId, Module, Node, Op, Range, ScalarType, Src, ValueId, VarDecl,
};
use super::super::lower::Lowered;
use super::super::{Error, R};

/// The colour outputs a key can name: the GC7000's four render targets
/// (Mesa's `ETNA_MAX_128BIT_RTS`, `etnaviv_internal.h:50-52`).
pub const OUTPUTS: u8 = 4;

/// Which colour outputs are 128-bit render targets.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RtPlanes {
    /// Bit `n` set: colour output `n` (`n` < [`OUTPUTS`]) is a 128-bit
    /// target; its R,G plane is output `n`, its B,A plane
    /// [`RtPlanes::companion_of`]`(n)`.
    pub mask: u8,
}

impl RtPlanes {
    /// No 128-bit target: the pass does nothing.
    pub const NONE: RtPlanes = RtPlanes { mask: 0 };

    pub const fn is_empty(&self) -> bool {
        self.mask == 0
    }

    /// `self` with colour output `n` a 128-bit target. An `n` past
    /// [`OUTPUTS`] is left out.
    pub const fn with(mut self, n: u8) -> RtPlanes {
        if n < OUTPUTS {
            self.mask |= 1 << n;
        }
        self
    }

    /// The output `n`'s B,A half is stored to, when `n` is a 128-bit
    /// target: `4 + k`, `k` the number of 128-bit outputs below `n`.
    pub const fn companion_of(&self, n: u8) -> Option<u8> {
        if n < OUTPUTS && self.mask & (1 << n) != 0 {
            let below = (self.mask & ((1u8 << n) - 1)).count_ones() as u8;
            Some(OUTPUTS + below)
        } else {
            None
        }
    }
}

/// A store's write mask (bit 0 = x ... bit 3 = w) as the masks of its two
/// plane stores: R,G (`wm & 3`) and B,A moved down to x,y (`wm >> 2`).
pub const fn split_write_mask(wm: u8) -> (u8, u8) {
    (wm & 0b11, (wm >> 2) & 0b11)
}

mod key {
    use core::sync::atomic::AtomicU8;
    pub static MASK: AtomicU8 = AtomicU8::new(0);
}

/// Set the [`RtPlanes`] the next GC7000 compiles split by, and return the
/// previous key so the caller can put it back. The compiler runs one
/// compile at a time (`store::with_scratch`), as for
/// [`super::wide128::set_compile_planes`].
pub fn set_compile_rt_planes(p: RtPlanes) -> RtPlanes {
    use core::sync::atomic::Ordering::Relaxed;
    RtPlanes { mask: key::MASK.swap(p.mask, Relaxed) }
}

/// The compile's [`RtPlanes`]: [`RtPlanes::NONE`] unless a caller set one.
pub fn compile_rt_planes() -> RtPlanes {
    use core::sync::atomic::Ordering::Relaxed;
    RtPlanes { mask: key::MASK.load(Relaxed) }
}

const fn refuse<'a>(msg: &'static str) -> Error<'a> {
    Error { line: 0, tok: "", msg }
}

/// The instructions a split store adds before its own id (which becomes
/// the B,A store): the zero constant, the R,G temporary's two stores and
/// load, the R,G store; then the B,A temporary's two stores and load, when
/// the store reaches B or A at all.
const EXTRA: u32 = 8;
const EXTRA_NO_HI: u32 = 5;

/// The colour declaration output `n` belongs to: its own location, or the
/// array that starts below it (as selection names the rows).
fn colour_decl<'m>(m: &'m Module<'_>, n: u8) -> Option<&'m ir::IoDecl<'m>> {
    m.outputs.iter().filter(|d| d.kind == IoKind::Colour && d.location <= n).max_by_key(|d| d.location)
}

/// The scalar type of a GL type enum: what an output of that type holds.
const fn scalar_of(gl_type: u16) -> ScalarType {
    match gl_type {
        /* GL_INT, GL_INT_VEC2..4 */
        0x1404 | 0x8B53..=0x8B55 => ScalarType::I32,
        /* GL_UNSIGNED_INT, GL_UNSIGNED_INT_VEC2..4 */
        0x1405 | 0x8DC6..=0x8DC8 => ScalarType::U32,
        _ => ScalarType::F32,
    }
}

/// The type and write mask of a store to colour output `n`: the declared
/// output's scalar and its width (module doc).
fn store_shape(m: &Module<'_>, n: u8) -> (ScalarType, u8) {
    match colour_decl(m, n) {
        Some(d) => (scalar_of(d.gl_type), (1u8 << d.components.clamp(1, 4)) - 1),
        None => (ScalarType::F32, 0b1111),
    }
}

/// `l` with every store to a colour output of `p.mask` split into its two
/// plane stores (module doc). Instructions keep program order in their
/// ids, as in [`super::wide128::split_planes`]: the new ones are numbered
/// right before the store's id, which the B,A store takes over. A module
/// that stores no keyed output, a vertex module, or an empty key comes
/// back as it was.
pub fn split_outputs<'a>(a: &Arena<'a>, l: Lowered<'a>, p: RtPlanes) -> R<'a, Lowered<'a>> {
    let m = l.module;
    if p.is_empty() || m.stage != crate::shaderc::SHC_STAGE_FRAGMENT {
        return Ok(l);
    }
    let keyed = |op: &Op| match *op {
        Op::StoreOutput { slot: IoSlot::FragColor(n), .. } => p.companion_of(n),
        _ => None,
    };
    if !m.instrs.iter().any(|op| keyed(op).is_some()) {
        return Ok(l);
    }
    for op in m.instrs.iter() {
        let Op::StoreOutput { slot: IoSlot::FragColor(n), .. } = *op else { continue };
        if (0..OUTPUTS).any(|k| p.companion_of(k) == Some(n)) {
            return Err(refuse("a 128-bit colour output's second plane is on an output the shader writes itself"));
        }
    }
    let extra = |op: &Op| match *op {
        Op::StoreOutput { slot: IoSlot::FragColor(n), .. } if p.companion_of(n).is_some() => {
            if split_write_mask(store_shape(&m, n).1).1 != 0 {
                EXTRA
            } else {
                EXTRA_NO_HI
            }
        }
        _ => 0,
    };

    let n = m.instrs.len();
    /* old id -> new id: the instructions inserted before it, its own
     * included */
    let mut new_id: AVec<u32> = AVec::with_capacity(a, n)?;
    let mut acc = 0u32;
    let mut temps = 0usize;
    for op in m.instrs.iter() {
        let e = extra(op);
        acc += e;
        temps += match e {
            EXTRA => 2,
            EXTRA_NO_HI => 1,
            _ => 0,
        };
        new_id.push(a, new_id.len() as u32 + acc)?;
    }
    let new_id = new_id.into_slice();
    let map = |v: ValueId| ValueId(new_id.get(v.0 as usize).copied().unwrap_or(v.0));

    let total = n + acc as usize;
    let mut instrs: AVec<Op> = AVec::with_capacity(a, total)?;
    let sized = !m.comps.is_empty();
    let mut comps: AVec<u8> = AVec::with_capacity(a, if sized { total } else { 0 })?;
    let mut locals: AVec<VarDecl<'a>> = AVec::with_capacity(a, m.locals.len() + temps)?;
    locals.extend_from_slice(a, m.locals)?;
    for (i, op) in m.instrs.iter().enumerate() {
        let op = super::super::lodkey::map_op(*op, &map);
        let own = if sized { m.comps.get(i).copied().unwrap_or(0) } else { 0 };
        let (out, src, companion) = match op {
            Op::StoreOutput { slot: IoSlot::FragColor(out), src } => match p.companion_of(out) {
                Some(c) => (out, src, c),
                None => {
                    instrs.push(a, op)?;
                    if sized {
                        comps.push(a, own)?;
                    }
                    continue;
                }
            },
            _ => {
                instrs.push(a, op)?;
                if sized {
                    comps.push(a, own)?;
                }
                continue;
            }
        };
        let (ty, wm) = store_shape(&m, out);
        let (lo, hi) = split_write_mask(wm);
        let sw = src.swizzle;
        let base = instrs.len() as u32;
        /* the zero the unwritten lanes read, in the output's own type: no
         * conversion reaches an integer output */
        instrs.push(a, if ty == ScalarType::F32 { Op::ConstF32([0; 4]) } else { Op::ConstI32([0; 4]) })?;
        let zero = Src::id(ValueId(base));
        /* R,G: value.xy in .xy under wm & 3, zero elsewhere */
        let t_lo = LocalId(locals.len() as u32);
        locals.push(a, VarDecl { name: "", ty: ir::Type::new(ty, 4), array_len: 1 })?;
        instrs.push(a, Op::StoreLocal { local: t_lo, index: None, src: zero, write_mask: !lo & 0b1111 })?;
        let rg = Src { val: src.val, swizzle: [sw[0], sw[1], sw[1], sw[1]] };
        instrs.push(a, Op::StoreLocal { local: t_lo, index: None, src: rg, write_mask: lo })?;
        instrs.push(a, Op::LoadLocal { local: t_lo, index: None, comps: 4 })?;
        instrs.push(a, Op::StoreOutput { slot: IoSlot::FragColor(out), src: Src::id(ValueId(base + 3)) })?;
        if sized {
            comps.extend_from_slice(a, &[4, 0, 0, 4, 0])?;
        }
        /* B,A: value.zw in .xy under wm >> 2, zero elsewhere; the plain
         * zero when the store reaches neither */
        let ba = if hi != 0 {
            let t_hi = LocalId(locals.len() as u32);
            locals.push(a, VarDecl { name: "", ty: ir::Type::new(ty, 4), array_len: 1 })?;
            instrs.push(a, Op::StoreLocal { local: t_hi, index: None, src: zero, write_mask: !hi & 0b1111 })?;
            let src_ba = Src { val: src.val, swizzle: [sw[2], sw[3], sw[3], sw[3]] };
            instrs.push(a, Op::StoreLocal { local: t_hi, index: None, src: src_ba, write_mask: hi })?;
            instrs.push(a, Op::LoadLocal { local: t_hi, index: None, comps: 4 })?;
            if sized {
                comps.extend_from_slice(a, &[0, 0, 4])?;
            }
            Src::id(ValueId(base + 7))
        } else {
            zero
        };
        instrs.push(a, Op::StoreOutput { slot: IoSlot::FragColor(companion), src: ba })?;
        if sized {
            comps.push(a, own)?;
        }
    }

    let mut nodes: AVec<Node> = AVec::with_capacity(a, m.nodes.len() + acc as usize)?;
    let body = copy_block(a, &m, m.body, &map, &extra, new_id, &mut nodes)?;
    Ok(Lowered {
        module: ir::Module {
            instrs: instrs.into_slice(),
            comps: if sized { comps.into_slice() } else { m.comps },
            nodes: nodes.into_slice(),
            body,
            locals: locals.into_slice(),
            ..m
        },
        reflection: l.reflection,
    })
}

/// One block of `m.nodes` into `out`, with the new ids and each split
/// store's added instructions right before its own (B,A store) node.
fn copy_block<'a>(
    a: &Arena<'a>,
    m: &Module<'_>,
    r: Range,
    map: &dyn Fn(ValueId) -> ValueId,
    extra: &dyn Fn(&Op) -> u32,
    new_id: &[u32],
    out: &mut AVec<'a, Node>,
) -> R<'a, Range> {
    let mut level: AVec<Node> = AVec::new();
    for &node in m.nodes_of(r) {
        let copied = match node {
            Node::Instr(InstrId(i)) => {
                let id = new_id.get(i as usize).copied().unwrap_or(i);
                let n = m.instrs.get(i as usize).map_or(0, extra);
                for j in (1..=n).rev() {
                    level.push(a, Node::Instr(InstrId(id - j)))?;
                }
                Node::Instr(InstrId(id))
            }
            Node::If { cond, then_, else_ } => {
                let then_ = copy_block(a, m, then_, map, extra, new_id, out)?;
                let else_ = copy_block(a, m, else_, map, extra, new_id, out)?;
                Node::If { cond: Src { val: map(cond.val), ..cond }, then_, else_ }
            }
            Node::Loop { body } => Node::Loop { body: copy_block(a, m, body, map, extra, new_id, out)? },
            other => other,
        };
        level.push(a, copied)?;
    }
    let first = out.len() as u32;
    out.extend_from_slice(a, level.as_slice())?;
    Ok(Range { first, len: level.len() as u32 })
}

/* ES 3.00 sources, which a report-es2 build refuses; the GC7000's front-end
 * order, as wide128's tests. */
#[cfg(all(test, feature = "gpu-vivante", not(feature = "report-es2")))]
mod tests {
    use super::super::super::lower::compile_to_ir;
    use super::super::super::sema::Stage;
    use super::*;
    use ir::AluOp;
    use std::vec;
    use std::vec::Vec;

    const HEAD: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\nin vec2 v_uv;\n";

    /// `src` lowered for the fragment stage and split under `p`, handed to
    /// `f` with the module it came from (both in one arena).
    fn split<T>(src: &str, p: RtPlanes, f: impl FnOnce(&Module<'_>, R<'_, Lowered<'_>>) -> T) -> T {
        let mut buf = vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let src = std::format!("{HEAD}{src}");
        let l = match compile_to_ir(&a, Stage::Fragment, &src) {
            Ok(l) => l,
            Err(e) => panic!("lowers: line {} at {:?}: {}", e.line, e.tok, e.msg),
        };
        let before = l.module;
        f(&before, split_outputs(&a, l, p))
    }

    /// The node order of `m`'s body, flattened, as instructions.
    fn program<'m>(m: &'m Module<'m>) -> Vec<&'m Op> {
        fn walk<'m>(m: &'m Module<'m>, r: Range, v: &mut Vec<&'m Op>) {
            for node in m.nodes_of(r) {
                match *node {
                    Node::Instr(id) => v.push(&m.instrs[id.0 as usize]),
                    Node::If { then_, else_, .. } => {
                        walk(m, then_, v);
                        walk(m, else_, v);
                    }
                    Node::Loop { body } => walk(m, body, v),
                    _ => {}
                }
            }
        }
        let mut v = Vec::new();
        walk(m, m.body, &mut v);
        v
    }

    /// Every colour store of `m`, in program order: (output, source).
    fn stores(m: &Module<'_>) -> Vec<(u8, Src)> {
        program(m)
            .into_iter()
            .filter_map(|op| match *op {
                Op::StoreOutput { slot: IoSlot::FragColor(n), src } => Some((n, src)),
                _ => None,
            })
            .collect()
    }

    /// What lane `lane` of the value `s` stores reads: `Some((v, c))` for
    /// lane `c` of the instruction `v` before the pass, `None` for zero.
    /// Follows the temporary's masked stores and the pass's zero constant;
    /// a value the pass did not make is its own instruction's.
    fn lane(m: &Module<'_>, s: Src, i: usize) -> Option<(Op, u8)> {
        let c = s.swizzle[i];
        match m.instrs[s.val.0 as usize] {
            Op::LoadLocal { local, index: None, .. } if m.locals[local.0 as usize].name.is_empty() => {
                /* the last store before the load that writes lane c */
                let last = m.instrs[..s.val.0 as usize]
                    .iter()
                    .rev()
                    .find_map(|op| match *op {
                        Op::StoreLocal { local: l, src, write_mask, .. }
                            if l == local && write_mask & (1 << c) != 0 =>
                        {
                            Some(src)
                        }
                        _ => None,
                    })
                    .expect("the lane is written");
                lane(m, last, c as usize)
            }
            Op::ConstF32([0, 0, 0, 0]) | Op::ConstI32([0, 0, 0, 0]) => None,
            op => Some((op, c)),
        }
    }

    /// The four lanes `s` stores, each as (the instruction, its lane) or
    /// zero.
    fn lanes(m: &Module<'_>, s: Src) -> [Option<(Op, u8)>; 4] {
        [0, 1, 2, 3].map(|i| lane(m, s, i))
    }

    /// The four lanes of the value the original store of output `n` wrote.
    fn written(m: &Module<'_>, n: u8) -> [Option<(Op, u8)>; 4] {
        let (_, s) = stores(m).into_iter().find(|&(o, _)| o == n).expect("the output is stored");
        lanes(m, s)
    }

    /// The module is the very one handed in: every slice the same memory.
    fn same(x: &Module<'_>, y: &Module<'_>) -> bool {
        core::ptr::eq(x.instrs, y.instrs)
            && core::ptr::eq(x.comps, y.comps)
            && core::ptr::eq(x.nodes, y.nodes)
            && core::ptr::eq(x.locals, y.locals)
            && core::ptr::eq(x.outputs, y.outputs)
            && x.body == y.body
            && x.stage == y.stage
    }

    const TWO: &str = "layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\nuniform vec4 u;\n\
                       void main() { o0 = u; o1 = u.wzyx; }";

    #[test]
    fn the_companion_is_four_plus_the_rank() {
        assert_eq!(RtPlanes::NONE.companion_of(0), None);
        let p = RtPlanes::NONE.with(0).with(2).with(3);
        assert_eq!([0, 1, 2, 3].map(|n| p.companion_of(n)), [Some(4), None, Some(5), Some(6)]);
        assert_eq!(RtPlanes::NONE.with(1).companion_of(1), Some(4));
        assert_eq!(RtPlanes::NONE.with(4), RtPlanes::NONE, "four colour outputs");
    }

    /// `.x`, `.zw`, `.yz` and the widths as the two planes' masks.
    #[test]
    fn write_masks_split_low_and_high() {
        assert_eq!(split_write_mask(0b0001), (0b01, 0b00));
        assert_eq!(split_write_mask(0b1100), (0b00, 0b11));
        assert_eq!(split_write_mask(0b0110), (0b10, 0b01));
        assert_eq!(split_write_mask(0b0111), (0b11, 0b01));
        assert_eq!(split_write_mask(0b1111), (0b11, 0b11));
    }

    #[test]
    fn the_compile_key_is_set_and_put_back() {
        let old = set_compile_rt_planes(RtPlanes::NONE.with(1));
        assert_eq!(compile_rt_planes(), RtPlanes { mask: 0b10 });
        assert_eq!(set_compile_rt_planes(old), RtPlanes { mask: 0b10 });
        assert_eq!(compile_rt_planes(), old);
    }

    /// With no key the pass hands back the module it was given, byte for
    /// byte: the same slices, so selection sees the same program.
    #[test]
    fn an_empty_key_returns_the_module_bit_identical() {
        split(TWO, RtPlanes::NONE, |before, r| {
            let after = r.expect("splits").module;
            assert!(same(before, &after), "the module is untouched");
        });
    }

    /// A key naming outputs the shader does not write changes nothing.
    #[test]
    fn a_key_naming_an_unwritten_output_is_a_no_op() {
        split(TWO, RtPlanes::NONE.with(2).with(3), |before, r| {
            assert!(same(before, &r.expect("splits").module), "the module is untouched");
        });
    }

    /// The vertex stage has no colour output: the key does not reach it.
    #[test]
    fn a_vertex_module_is_untouched() {
        let mut buf = vec![0u8; 1 << 21];
        let a = Arena::new(&mut buf);
        let l = compile_to_ir(&a, Stage::Vertex, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }")
            .expect("lowers");
        let before = l.module;
        let after = split_outputs(&a, l, RtPlanes::NONE.with(0)).expect("splits").module;
        assert!(same(&before, &after));
    }

    /// Output 0 keyed: (r,g) to output 0, (b,a) to output 4, output 1 as
    /// it was.
    fn split_two(ty: &str, scalar: ScalarType) {
        let src = std::format!(
            "layout(location = 0) out {ty} o0;\nlayout(location = 1) out {ty} o1;\nuniform {ty} u;\n\
             void main() {{ o0 = u; o1 = u.wzyx; }}"
        );
        split(&src, RtPlanes::NONE.with(0), |before, r| {
            let after = r.expect("splits").module;
            let orig = written(before, 0);
            let one = written(before, 1);
            assert_eq!(stores(&after).iter().map(|s| s.0).collect::<Vec<_>>(), [0, 4, 1], "{ty}");
            assert_eq!(written(&after, 0), [orig[0], orig[1], None, None], "{ty}: R,G");
            assert_eq!(written(&after, 4), [orig[2], orig[3], None, None], "{ty}: B,A");
            assert_eq!(written(&after, 1), one, "{ty}: output 1 untouched");
            assert!(orig.iter().all(Option::is_some), "{ty}: a whole vec4");
            /* the temporaries are of the output's type, and no conversion
             * was added */
            let temps = &after.locals[before.locals.len()..];
            assert_eq!(temps.len(), 2, "{ty}");
            assert!(temps.iter().all(|t| t.ty == ir::Type::new(scalar, 4)), "{ty}: {temps:?}");
            let converts = |m: &Module<'_>| {
                m.instrs
                    .iter()
                    .filter(|op| {
                        matches!(op, Op::Alu(AluOp::I2f | AluOp::U2f | AluOp::F2i | AluOp::F2u | AluOp::B2f, _))
                    })
                    .count()
            };
            assert_eq!(converts(&after), converts(before), "{ty}: no conversion");
            let zero_float = after.instrs.contains(&Op::ConstF32([0; 4]));
            assert_eq!(zero_float, scalar == ScalarType::F32, "{ty}: the zero is of the output's type");
            assert_eq!(after.comps.len(), after.instrs.len(), "{ty}: every value has its width");
        });
    }

    #[test]
    fn a_vec4_store_splits_into_its_two_planes() {
        split_two("vec4", ScalarType::F32);
    }

    #[test]
    fn an_unsigned_store_stays_unsigned() {
        split_two("uvec4", ScalarType::U32);
    }

    #[test]
    fn a_signed_store_stays_signed() {
        split_two("ivec4", ScalarType::I32);
    }

    /// A scalar store is `.x`: (r, 0) and the zero plane. A `vec3` store
    /// is `.xyz`: (r, g) and (b, 0).
    #[test]
    fn scalar_and_vec3_stores_zero_extend() {
        let src = "out uint o;\nuniform uint u;\nvoid main() { o = u; }";
        split(src, RtPlanes::NONE.with(0), |before, r| {
            let after = r.expect("splits").module;
            let orig = written(before, 0);
            assert_eq!(written(&after, 0), [orig[0], None, None, None]);
            assert_eq!(written(&after, 4), [None; 4]);
        });
        let src = "out vec3 o;\nuniform vec3 u;\nvoid main() { o = u; }";
        split(src, RtPlanes::NONE.with(0), |before, r| {
            let after = r.expect("splits").module;
            let orig = written(before, 0);
            assert_eq!(written(&after, 0), [orig[0], orig[1], None, None]);
            assert_eq!(written(&after, 4), [orig[2], None, None, None]);
        });
    }

    /// A store the shader writes in parts (`.x`, `.zw`, `.yz`) is held in
    /// a local and stored whole; the split reads its lanes from that one
    /// store, wherever the store sits.
    #[test]
    fn a_store_in_parts_and_under_a_branch_splits() {
        let src = "out ivec4 o;\nuniform ivec4 u;\nuniform int c;\n\
                   void main() { o.x = u.x; o.zw = u.yx; if (c > 0) { o.yz = u.zw; } }";
        split(src, RtPlanes::NONE.with(0), |before, r| {
            let after = r.expect("splits").module;
            let orig = written(before, 0);
            assert_eq!(written(&after, 0), [orig[0], orig[1], None, None]);
            assert_eq!(written(&after, 4), [orig[2], orig[3], None, None]);
        });
        let src =
            "out vec4 o;\nuniform vec4 u;\nuniform int c;\nvoid main() { if (c > 0) { o = u; } else { o = u.wzyx; } }";
        split(src, RtPlanes::NONE.with(0), |before, r| {
            let after = r.expect("splits").module;
            assert_eq!(stores(before).len(), 2);
            assert_eq!(stores(&after).iter().map(|s| s.0).collect::<Vec<_>>(), [0, 4, 0, 4]);
            /* each branch's pair stores that branch's value */
            for (k, (_, s)) in stores(before).into_iter().enumerate() {
                let orig = lanes(before, s);
                let pair = &stores(&after)[2 * k..2 * k + 2];
                assert_eq!(lanes(&after, pair[0].1), [orig[0], orig[1], None, None]);
                assert_eq!(lanes(&after, pair[1].1), [orig[2], orig[3], None, None]);
            }
        });
    }

    /// Outputs 1 and 3 keyed: their B,A halves are outputs 4 and 5, in
    /// rank order, whatever their slots.
    #[test]
    fn companions_follow_the_rank() {
        let src = "layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
                   layout(location = 3) out vec4 o3;\nuniform vec4 u;\n\
                   void main() { o0 = u; o1 = u.yxwz; o3 = u.wzyx; }";
        split(src, RtPlanes::NONE.with(1).with(3), |before, r| {
            let after = r.expect("splits").module;
            assert_eq!(stores(&after).iter().map(|s| s.0).collect::<Vec<_>>(), [0, 1, 4, 3, 5]);
            assert_eq!(written(&after, 0), written(before, 0));
            let (o1, o3) = (written(before, 1), written(before, 3));
            assert_eq!(written(&after, 4), [o1[2], o1[3], None, None]);
            assert_eq!(written(&after, 5), [o3[2], o3[3], None, None]);
        });
    }
}
