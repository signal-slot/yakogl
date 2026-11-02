// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! 128-bit samplers on the GC7000: a look-up on an RGBA32F / RGBA32I /
//! RGBA32UI texture is two look-ups, one on each G32R32 plane, joined.
//!
//! The texture engine has no 128-bit texel format. Mesa 26.2.1's etnaviv
//! stores such a texture as two G32R32 planes (R,G and B,A), binds the second
//! to a companion sampler slot, and rewrites each look-up into two whose
//! halves it joins, keyed per draw on which samplers hold a 128-bit texture
//! (`etnaviv_nir_lower_128bit.c:89-170`; the companion slot's descriptor,
//! `etnaviv_texture_state.c:286-296,516-557`; both as cited in
//! docs/es3/gc-formats.md section 3). This pass is the compiler half of that,
//! run on the GC7000's module before selection: every look-up on a unit in
//! [`Planes::mask`] becomes
//!
//! ```text
//!   lo = tex(unit, coord, lod)            -- R,G in .xy
//!   t.xy = lo.xy
//!   hi = tex(companion[unit], coord, lod) -- B,A in .xy
//!   t.zw = hi.xy
//!   value = t                              -- the vec4 the shader reads
//! ```
//!
//! the join a constructor's lane-masked stores into a compiler temporary
//! (`lower.rs` `assemble`), so selection sees nothing new. An integer
//! sampler's two look-ups keep their integer result type here and are given
//! the float one by [`super::int_tex`], which runs after this pass. A shape
//! the pass cannot split -- a shadow look-up, explicit gradients, a constant
//! offset -- is refused by name rather than sampled from one plane.
//! `textureSize` reads the primary unit's size, which is the texture's.
//!
//! The key is per draw ([`set_compile_planes`], empty in every compile that
//! does not ask): with an empty key the pass returns the module untouched,
//! so no shader the driver ships changes by a byte.

use super::super::arena::{AVec, Arena};
use super::super::ir::{self, InstrId, LocalId, Module, Node, Op, Range, Src, TexLod, ValueId, VarDecl};
use super::super::lower::Lowered;
use super::super::{Error, R};

/// The sampler units a key can name: the GC7000's 32 (16 fragment, 16
/// vertex), the range `TEX_ID` encodes (`alloc.rs`).
pub const UNITS: usize = 32;

/// Which sampler units hold a 128-bit texture, and the unit each one's
/// second (B,A) plane is bound to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Planes {
    /// Bit `u` set: unit `u` is a 128-bit sampler; its R,G plane is on `u`.
    pub mask: u32,
    /// `companion[u]`: the unit of `u`'s B,A plane. Read only for the units
    /// in `mask`.
    pub companion: [u8; UNITS],
}

impl Planes {
    /// No 128-bit sampler: the pass does nothing.
    pub const NONE: Planes = Planes { mask: 0, companion: [0; UNITS] };

    pub const fn is_empty(&self) -> bool {
        self.mask == 0
    }

    /// `self` with unit `unit` a 128-bit sampler whose B,A plane is on
    /// `companion`.
    pub const fn with(mut self, unit: u8, companion: u8) -> Planes {
        if (unit as usize) < UNITS {
            self.mask |= 1 << unit;
            self.companion[unit as usize] = companion;
        }
        self
    }

    /// The companion unit of `unit`, when `unit` is a 128-bit sampler.
    pub const fn companion_of(&self, unit: u8) -> Option<u8> {
        if (unit as usize) < UNITS && self.mask & (1 << unit) != 0 {
            Some(self.companion[unit as usize])
        } else {
            None
        }
    }
}

impl Default for Planes {
    fn default() -> Planes {
        Planes::NONE
    }
}

mod key {
    use core::sync::atomic::{AtomicU32, AtomicU8};
    pub static MASK: AtomicU32 = AtomicU32::new(0);
    pub static COMPANION: [AtomicU8; super::UNITS] = [const { AtomicU8::new(0) }; super::UNITS];
}

/// Set the [`Planes`] the next GC7000 compiles split by, and return the
/// previous key so the caller can put it back. The compiler runs one
/// compile at a time (`store::with_scratch`), as the LodKey does
/// (`shaderc::set_compile_lod_key`).
pub fn set_compile_planes(p: Planes) -> Planes {
    use core::sync::atomic::Ordering::Relaxed;
    let mut old = Planes { mask: key::MASK.swap(p.mask, Relaxed), companion: [0; UNITS] };
    for (u, c) in key::COMPANION.iter().enumerate() {
        old.companion[u] = c.swap(p.companion[u], Relaxed);
    }
    old
}

/// The compile's [`Planes`]: [`Planes::NONE`] unless a caller set one.
pub fn compile_planes() -> Planes {
    use core::sync::atomic::Ordering::Relaxed;
    let mut p = Planes { mask: key::MASK.load(Relaxed), companion: [0; UNITS] };
    if p.mask != 0 {
        for (u, c) in key::COMPANION.iter().enumerate() {
            p.companion[u] = c.load(Relaxed);
        }
    }
    p
}

const fn refuse<'a>(msg: &'static str) -> Error<'a> {
    Error { line: 0, tok: "", msg }
}

/// The instructions a split look-up adds before its own id (which becomes
/// the joining load): two look-ups and two stores.
const EXTRA: u32 = 4;

/// `l` with every look-up on a unit of `p.mask` split into its two plane
/// look-ups and joined (module doc). Instructions keep program order in
/// their ids, as in `lodkey::remap_mag_switch`: the four new ones are
/// numbered right before the look-up's id, which the joining load takes
/// over, so every reader of the look-up reads the joined vec4. A module
/// without such a look-up, or an empty key, comes back as it was.
pub fn split_planes<'a>(a: &Arena<'a>, l: Lowered<'a>, p: Planes) -> R<'a, Lowered<'a>> {
    let m = l.module;
    if p.is_empty() {
        return Ok(l);
    }
    let split = |op: &Op| matches!(*op, Op::Tex { sampler, .. } if p.companion_of(sampler).is_some());
    if !m.instrs.iter().any(split) {
        return Ok(l);
    }
    for op in m.instrs.iter() {
        let sampled = match *op {
            Op::Tex { sampler, .. } | Op::TexSize { sampler, .. } => sampler,
            _ => continue,
        };
        for u in 0..UNITS as u8 {
            if p.companion_of(u) == Some(sampled) {
                return Err(refuse("a 128-bit sampler's second plane is on a unit the shader samples itself"));
            }
        }
        let Op::Tex { sampler, shadow, lod, offset, .. } = *op else { continue };
        let Some(c) = p.companion_of(sampler) else { continue };
        if c as usize >= UNITS || c == sampler {
            return Err(refuse("a 128-bit sampler's second plane has no sampler unit of its own"));
        }
        if shadow {
            return Err(refuse("a shadow look-up on a 128-bit sampler is not split into its two planes"));
        }
        if matches!(lod, TexLod::Grad { .. }) {
            return Err(refuse("a gradient look-up on a 128-bit sampler is not split into its two planes"));
        }
        if offset.is_some() {
            return Err(refuse("an offset look-up on a 128-bit sampler is not split into its two planes"));
        }
    }

    let n = m.instrs.len();
    /* old id -> new id: the instructions inserted before it, its own
     * included */
    let mut new_id: AVec<u32> = AVec::with_capacity(a, n)?;
    let mut acc = 0u32;
    for op in m.instrs.iter() {
        if split(op) {
            acc += EXTRA;
        }
        new_id.push(a, new_id.len() as u32 + acc)?;
    }
    let new_id = new_id.into_slice();
    let map = |v: ValueId| ValueId(new_id.get(v.0 as usize).copied().unwrap_or(v.0));

    let total = n + acc as usize;
    let mut instrs: AVec<Op> = AVec::with_capacity(a, total)?;
    let sized = !m.comps.is_empty();
    let mut comps: AVec<u8> = AVec::with_capacity(a, if sized { total } else { 0 })?;
    let mut locals: AVec<VarDecl<'a>> = AVec::with_capacity(a, m.locals.len() + (acc / EXTRA) as usize)?;
    locals.extend_from_slice(a, m.locals)?;
    for (i, op) in m.instrs.iter().enumerate() {
        let op = super::super::lodkey::map_op(*op, &map);
        let Op::Tex { sampler, ret, .. } = op else {
            instrs.push(a, op)?;
            if sized {
                comps.push(a, m.comps.get(i).copied().unwrap_or(0))?;
            }
            continue;
        };
        let Some(companion) = p.companion_of(sampler) else {
            instrs.push(a, op)?;
            if sized {
                comps.push(a, m.comps.get(i).copied().unwrap_or(0))?;
            }
            continue;
        };
        /* one temporary per split look-up, of the look-up's own type */
        let t = LocalId(locals.len() as u32);
        locals.push(a, VarDecl { name: "", ty: ir::Type::new(ret, 4), array_len: 1 })?;
        let base = instrs.len() as u32;
        let hi_op = match op {
            Op::Tex { dim, shadow, proj, ret, coord, lod, offset, .. } => {
                Op::Tex { dim, shadow, proj, ret, sampler: companion, coord, lod, offset }
            }
            other => other,
        };
        instrs.push(a, op)?;
        instrs.push(a, Op::StoreLocal { local: t, index: None, src: Src::id(ValueId(base)), write_mask: 0b0011 })?;
        instrs.push(a, hi_op)?;
        /* lanes z and w read the plane's x and y */
        let hi = Src { val: ValueId(base + 2), swizzle: [0, 1, 0, 1] };
        instrs.push(a, Op::StoreLocal { local: t, index: None, src: hi, write_mask: 0b1100 })?;
        instrs.push(a, Op::LoadLocal { local: t, index: None, comps: 4 })?;
        if sized {
            comps.extend_from_slice(a, &[4, 0, 4, 0, m.comps.get(i).copied().unwrap_or(4)])?;
        }
    }

    let mut nodes: AVec<Node> = AVec::with_capacity(a, m.nodes.len() + acc as usize)?;
    let body = copy_block(a, &m, m.body, &map, &split, new_id, &mut nodes)?;
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
/// look-up's four instructions right before its (joining) node.
fn copy_block<'a>(
    a: &Arena<'a>,
    m: &Module<'_>,
    r: Range,
    map: &dyn Fn(ValueId) -> ValueId,
    split: &dyn Fn(&Op) -> bool,
    new_id: &[u32],
    out: &mut AVec<'a, Node>,
) -> R<'a, Range> {
    let mut level: AVec<Node> = AVec::new();
    for &node in m.nodes_of(r) {
        let copied = match node {
            Node::Instr(InstrId(i)) => {
                let id = new_id.get(i as usize).copied().unwrap_or(i);
                if m.instrs.get(i as usize).is_some_and(split) {
                    for j in (1..=EXTRA).rev() {
                        level.push(a, Node::Instr(InstrId(id - j)))?;
                    }
                }
                Node::Instr(InstrId(id))
            }
            Node::If { cond, then_, else_ } => {
                let then_ = copy_block(a, m, then_, map, split, new_id, out)?;
                let else_ = copy_block(a, m, else_, map, split, new_id, out)?;
                Node::If { cond: Src { val: map(cond.val), ..cond }, then_, else_ }
            }
            Node::Loop { body } => Node::Loop { body: copy_block(a, m, body, map, split, new_id, out)? },
            other => other,
        };
        level.push(a, copied)?;
    }
    let first = out.len() as u32;
    out.extend_from_slice(a, level.as_slice())?;
    Ok(Range { first, len: level.len() as u32 })
}

/* The GC7000 compile path (`codegen::compile` is another GPU's in their
 * builds), and ES 3.00 sources, which a report-es2 build refuses. */
#[cfg(all(test, feature = "gpu-vivante", not(feature = "report-es2")))]
mod tests {
    use super::super::super::codegen;
    use super::super::super::store;
    use super::super::decode::decode_instr;
    use super::*;
    use crate::shaderc::SHC_STAGE_FRAGMENT;
    use std::string::String;
    use std::vec::Vec;

    /// The key set for one compile and put back after it, panic or not.
    struct Keyed(Planes);
    impl Drop for Keyed {
        fn drop(&mut self) {
            set_compile_planes(self.0);
        }
    }

    /// One fragment shader compiled under `p`, as its instruction words, or
    /// the refusal. The caller holds `store::test_keep()`, which also keeps
    /// every other compile out while the key is set.
    fn compile_fs(src: &str, p: Planes) -> Result<Vec<[u32; 4]>, String> {
        let _key = Keyed(set_compile_planes(p));
        let sh = store::with_scratch(|s| codegen::compile(SHC_STAGE_FRAGMENT, src.as_bytes(), 0, s, store::keep()))
            .expect("the scratch region is free")
            .map_err(|e| std::format!("{e:?}"))?;
        // SAFETY: `usc`/`usc_dw` are what the compiler just wrote into the
        // keep arena, which the caller's `test_keep` holds.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        Ok(dw.as_chunks::<4>().0.to_vec())
    }

    fn listing(ws: &[[u32; 4]]) -> Vec<String> {
        ws.iter()
            .map(|w| {
                let mut s = String::new();
                super::super::listing::body_text(&mut s, &decode_instr(w).expect("decodes")).expect("text");
                s
            })
            .collect()
    }

    /// The `TEX_ID` of every look-up, in program order.
    fn units(ws: &[[u32; 4]]) -> Vec<(&'static str, u32)> {
        ws.iter()
            .map(|w| decode_instr(w).expect("decodes"))
            .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
            .map(|i| (i.op().name, i.get("TEX_ID").expect("a look-up names its unit")))
            .collect()
    }

    const HEAD: &str = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\n";

    fn fs(decl: &str, out: &str, body: &str) -> String {
        std::format!("{HEAD}{decl}\nout {out} o;\nvoid main() {{ o = {body}; }}")
    }

    fn float_fs() -> String {
        fs("uniform highp sampler2D s;", "vec4", "texture(s, v_uv)")
    }

    /// With no key, and with a key naming a unit the shader does not
    /// sample, the binary is the one the driver has always built.
    #[test]
    fn an_empty_key_changes_nothing() {
        let _k = store::test_keep();
        let plain = compile_fs(&float_fs(), Planes::NONE).expect("compiles");
        assert_eq!(units(&plain), [("texld", 0)]);
        let other = compile_fs(&float_fs(), Planes::NONE.with(3, 4)).expect("compiles");
        assert_eq!(plain, other, "a key for another unit leaves the shader as it was");
    }

    /// A look-up on a 128-bit unit is a texld on the unit and one on its
    /// companion, the second plane's x,y landing in the result's z,w.
    #[test]
    fn a_128_bit_lookup_is_two_plane_lookups_joined() {
        let _k = store::test_keep();
        let ws = compile_fs(&float_fs(), Planes::NONE.with(0, 5)).expect("compiles");
        let text = listing(&ws);
        assert_eq!(units(&ws), [("texld", 0), ("texld", 5)], "{text:#?}");
        /* each plane's R,G (B,A) lands in .xy, and the second plane's x,y
         * is moved into the first's z,w: `mov tA.__zw, tB.xyxy` */
        let (lo, hi) = (&text[0], &text[1]);
        assert!(lo.starts_with("texld") && lo.contains(".xy__, tex0."), "{text:#?}");
        assert!(hi.starts_with("texld") && hi.contains(".xy__, tex5."), "{text:#?}");
        let reg = |t: &str| t.split_whitespace().nth(1).and_then(|d| d.split('.').next()).map(String::from);
        let (lo_reg, hi_reg) = (reg(lo).expect("a destination"), reg(hi).expect("a destination"));
        let join = std::format!("{lo_reg}.__zw, void, void, {hi_reg}.xyxy");
        assert!(text.iter().any(|t| t.starts_with("mov") && t.contains(&join)), "{join}: {text:#?}");
        let plain = compile_fs(&float_fs(), Planes::NONE).expect("compiles");
        assert!(ws.len() > plain.len(), "the split costs instructions: {text:#?}");
    }

    /// An integer sampler is split the same way and keeps int_tex's float
    /// look-up: two texld, as an RGBA8UI one is one.
    #[test]
    fn an_integer_128_bit_lookup_is_split_too() {
        let _k = store::test_keep();
        let src = fs("uniform highp usampler2D s;", "uvec4", "texture(s, v_uv)");
        let ws = compile_fs(&src, Planes::NONE.with(0, 1)).expect("compiles");
        assert_eq!(units(&ws), [("texld", 0), ("texld", 1)], "{:#?}", listing(&ws));
    }

    /// The shapes the pass cannot split are refused by name.
    #[test]
    fn unsplittable_shapes_are_refused_by_name() {
        let _k = store::test_keep();
        let p = Planes::NONE.with(0, 1);
        let s = "uniform highp sampler2D s;";
        let e = compile_fs(&fs(s, "vec4", "textureOffset(s, v_uv, ivec2(1, 0))"), p).expect_err("refused");
        assert!(e.contains("offset look-up on a 128-bit sampler"), "{e}");
        let e = compile_fs(&fs(s, "vec4", "textureGrad(s, v_uv, vec2(0.1), vec2(0.1))"), p).expect_err("refused");
        assert!(e.contains("gradient look-up on a 128-bit sampler"), "{e}");
        let two = "uniform highp sampler2D s;\nuniform highp sampler2D t;";
        let e = compile_fs(&fs(two, "vec4", "texture(s, v_uv) + texture(t, v_uv)"), p).expect_err("refused");
        assert!(e.contains("second plane is on a unit the shader samples"), "{e}");
    }
}
