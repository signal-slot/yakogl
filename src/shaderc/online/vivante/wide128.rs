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
//! (`lower.rs` `assemble`), so selection sees nothing new. A unit keyed
//! with lane sources other than (R,G,B,A) ([`Planes::swizzle`], the GL
//! texture swizzle and no alpha, docs/notes/gc-128bit-swizzle.md section 4)
//! has each lane stored from the plane and channel, or the constant 0 or 1,
//! that it names; both descriptors stay the identity. An integer
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
    /// Bit `u` set (only ever for a unit in `mask`): the binding's texture
    /// has no alpha (an RGB32 texture stored as RGBA32, `GcTexfmt::no_alpha`).
    /// The descriptors' swizzle A = ONE reaches the R,G plane's channels
    /// only, and the join reads the B,A plane's second channel, so the
    /// joined `.w` is the constant 1 (1.0, or 1 for an integer sampler)
    /// instead.
    pub no_alpha: u32,
    /// `swizzle[u]`: what the look-up on unit `u` returns in each lane, a
    /// lane of the joined (R,G,B,A) 0..=3, [`ZERO`] or [`ONE`]: the GL
    /// texture swizzle composed onto the binding (`wide128_lane_sources` in
    /// src/backend.rs; docs/notes/gc-128bit-swizzle.md section 4). Read only
    /// for the units in `mask`; [`IDENTITY`] is the join as it always was.
    /// Where it names lane 3 on a unit in `no_alpha`, the lane reads [`ONE`].
    pub swizzle: [[u8; 4]; UNITS],
}

/// The lane sources of a look-up that returns (R,G,B,A) as stored.
pub const IDENTITY: [u8; 4] = [0, 1, 2, 3];
/// A lane source: the constant 0 (0.0, or 0 for an integer sampler).
pub const ZERO: u8 = 4;
/// A lane source: the constant 1 (1.0, or 1 for an integer sampler).
pub const ONE: u8 = 5;

impl Planes {
    /// No 128-bit sampler: the pass does nothing.
    pub const NONE: Planes = Planes { mask: 0, companion: [0; UNITS], no_alpha: 0, swizzle: [IDENTITY; UNITS] };

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

    /// [`Planes::with`], the look-up on `unit` returning the lane sources
    /// `sw` (see [`Planes::swizzle`]) instead of (R,G,B,A).
    pub const fn with_swizzle(self, unit: u8, companion: u8, sw: [u8; 4]) -> Planes {
        let mut p = self.with(unit, companion);
        if (unit as usize) < UNITS {
            p.swizzle[unit as usize] = sw;
        }
        p
    }

    /// The lane sources of the 128-bit sampler on `unit` as keyed,
    /// [`IDENTITY`] for a unit that is not one. `no_alpha` is not folded in
    /// ([`Planes::lanes_of`] is what the join writes).
    pub const fn swizzle_of(&self, unit: u8) -> [u8; 4] {
        if (unit as usize) < UNITS && self.mask & (1 << unit) != 0 {
            self.swizzle[unit as usize]
        } else {
            IDENTITY
        }
    }

    /// What the join writes into each lane for `unit`: [`Planes::swizzle_of`],
    /// lane 3 read as [`ONE`] when the texture has no alpha.
    pub const fn lanes_of(&self, unit: u8) -> [u8; 4] {
        let mut sw = self.swizzle_of(unit);
        if self.lacks_alpha(unit) {
            let mut k = 0;
            while k < 4 {
                if sw[k] == 3 {
                    sw[k] = ONE;
                }
                k += 1;
            }
        }
        sw
    }

    /// `self` with the 128-bit sampler on `unit` marked as having no alpha
    /// (see [`Planes::no_alpha`]). A unit that is not keyed is left alone.
    pub const fn without_alpha(mut self, unit: u8) -> Planes {
        if (unit as usize) < UNITS && self.mask & (1 << unit) != 0 {
            self.no_alpha |= 1 << unit;
        }
        self
    }

    /// Whether the 128-bit sampler on `unit` has no alpha.
    pub const fn lacks_alpha(&self, unit: u8) -> bool {
        (unit as usize) < UNITS && self.mask & self.no_alpha & (1 << unit) != 0
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
    pub static NO_ALPHA: AtomicU32 = AtomicU32::new(0);
    pub static COMPANION: [AtomicU8; super::UNITS] = [const { AtomicU8::new(0) }; super::UNITS];
    /// Each unit's lane sources, a byte per lane (`u32::from_le_bytes`).
    pub static SWIZZLE: [AtomicU32; super::UNITS] =
        [const { AtomicU32::new(u32::from_le_bytes(super::IDENTITY)) }; super::UNITS];
}

/// Set the [`Planes`] the next GC7000 compiles split by, and return the
/// previous key so the caller can put it back. The compiler runs one
/// compile at a time (`store::with_scratch`), as the LodKey does
/// (`shaderc::set_compile_lod_key`).
pub fn set_compile_planes(p: Planes) -> Planes {
    use core::sync::atomic::Ordering::Relaxed;
    let mut old = Planes {
        mask: key::MASK.swap(p.mask, Relaxed),
        companion: [0; UNITS],
        no_alpha: key::NO_ALPHA.swap(p.no_alpha, Relaxed),
        swizzle: [IDENTITY; UNITS],
    };
    for (u, c) in key::COMPANION.iter().enumerate() {
        old.companion[u] = c.swap(p.companion[u], Relaxed);
    }
    for (u, s) in key::SWIZZLE.iter().enumerate() {
        old.swizzle[u] = s.swap(u32::from_le_bytes(p.swizzle[u]), Relaxed).to_le_bytes();
    }
    old
}

/// The compile's [`Planes`]: [`Planes::NONE`] unless a caller set one.
pub fn compile_planes() -> Planes {
    use core::sync::atomic::Ordering::Relaxed;
    let mut p = Planes { mask: key::MASK.load(Relaxed), ..Planes::NONE };
    if p.mask != 0 {
        p.no_alpha = key::NO_ALPHA.load(Relaxed);
        for (u, c) in key::COMPANION.iter().enumerate() {
            p.companion[u] = c.load(Relaxed);
        }
        for (u, s) in key::SWIZZLE.iter().enumerate() {
            p.swizzle[u] = s.load(Relaxed).to_le_bytes();
        }
    }
    p
}

const fn refuse<'a>(msg: &'static str) -> Error<'a> {
    Error { line: 0, tok: "", msg }
}

/// How the join of one look-up writes its four lanes, from the unit's lane
/// sources ([`Planes::lanes_of`]): the lanes read from the R,G plane's
/// look-up, those read from the B,A plane's, and those given a constant
/// (docs/notes/gc-128bit-swizzle.md section 5.2 step 2).
#[derive(Clone, Copy)]
struct Join {
    /// Write mask and swizzle of the store from the first look-up.
    lo: u8,
    lo_swizzle: [u8; 4],
    /// Write mask and swizzle of the store from the companion's look-up.
    hi: u8,
    hi_swizzle: [u8; 4],
    /// Write mask of the constant store, and the lanes of the constant that
    /// are 1 (the others 0).
    konst: u8,
    ones: u8,
}

impl Join {
    /// The join for lane sources `sw`, `None` for a source past [`ONE`].
    /// The lanes a store does not write keep the swizzle the identity join
    /// has always had (the first look-up's `.xyzw`, the second's `.xyxy`),
    /// and the constant's unwritten lanes copy its first written one, so an
    /// identity or a no-alpha unit gives the instructions it always did.
    const fn of(sw: [u8; 4]) -> Option<Join> {
        let mut j = Join { lo: 0, lo_swizzle: [0, 1, 2, 3], hi: 0, hi_swizzle: [0, 1, 0, 1], konst: 0, ones: 0 };
        let mut first_one = None;
        let mut k = 0;
        while k < 4 {
            match sw[k] {
                s @ 0..=1 => {
                    j.lo |= 1 << k;
                    j.lo_swizzle[k] = s;
                }
                s @ 2..=3 => {
                    j.hi |= 1 << k;
                    j.hi_swizzle[k] = s - 2;
                }
                s @ (ZERO | ONE) => {
                    j.konst |= 1 << k;
                    if first_one.is_none() {
                        first_one = Some(s == ONE);
                    }
                    if s == ONE {
                        j.ones |= 1 << k;
                    }
                }
                _ => return None,
            }
            k += 1;
        }
        if let Some(true) = first_one {
            j.ones |= !j.konst & 0xf;
        }
        Some(j)
    }

    /// The instructions the join adds before the look-up's own id (which
    /// becomes the joining load): the first look-up, always (selection
    /// drops it when no lane reads it), its store, the companion's look-up
    /// and its store when a lane reads that plane, and a constant and its
    /// store when a lane is ZERO or ONE. 4 for identity, 6 for no alpha.
    const fn extra(&self) -> u32 {
        1 + (self.lo != 0) as u32 + 2 * (self.hi != 0) as u32 + 2 * (self.konst != 0) as u32
    }
}

/// `l` with every look-up on a unit of `p.mask` split into its two plane
/// look-ups and joined (module doc), each lane written from the source
/// [`Planes::lanes_of`] names. Instructions keep program order in their
/// ids, as in `lodkey::remap_mag_switch`: the new ones ([`Join::extra`])
/// are numbered right before the look-up's id, which the joining load
/// takes over, so every reader of the look-up reads the joined vec4. A
/// module without such a look-up, or an empty key, comes back as it was.
pub fn split_planes<'a>(a: &Arena<'a>, l: Lowered<'a>, p: Planes) -> R<'a, Lowered<'a>> {
    let m = l.module;
    if p.is_empty() {
        return Ok(l);
    }
    let split = |op: &Op| matches!(*op, Op::Tex { sampler, .. } if p.companion_of(sampler).is_some());
    let join = |sampler: u8| Join::of(p.lanes_of(sampler));
    let extra = |op: &Op| match *op {
        Op::Tex { sampler, .. } if p.companion_of(sampler).is_some() => join(sampler).map_or(0, |j| j.extra()),
        _ => 0,
    };
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
        if join(sampler).is_none() {
            return Err(refuse("a 128-bit sampler's lane source is not a lane, zero or one"));
        }
    }

    let n = m.instrs.len();
    /* old id -> new id: the instructions inserted before it, its own
     * included */
    let mut new_id: AVec<u32> = AVec::with_capacity(a, n)?;
    let mut acc = 0u32;
    let mut splits = 0usize;
    for op in m.instrs.iter() {
        let e = extra(op);
        acc += e;
        splits += (e != 0) as usize;
        new_id.push(a, new_id.len() as u32 + acc)?;
    }
    let new_id = new_id.into_slice();
    let map = |v: ValueId| ValueId(new_id.get(v.0 as usize).copied().unwrap_or(v.0));

    let total = n + acc as usize;
    let mut instrs: AVec<Op> = AVec::with_capacity(a, total)?;
    let sized = !m.comps.is_empty();
    let mut comps: AVec<u8> = AVec::with_capacity(a, if sized { total } else { 0 })?;
    let mut locals: AVec<VarDecl<'a>> = AVec::with_capacity(a, m.locals.len() + splits)?;
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
        let (Some(companion), Some(j)) = (p.companion_of(sampler), join(sampler)) else {
            instrs.push(a, op)?;
            if sized {
                comps.push(a, m.comps.get(i).copied().unwrap_or(0))?;
            }
            continue;
        };
        /* one temporary per split look-up, of the look-up's own type */
        let t = LocalId(locals.len() as u32);
        locals.push(a, VarDecl { name: "", ty: ir::Type::new(ret, 4), array_len: 1 })?;
        let store = |val: u32, swizzle: [u8; 4], write_mask: u8| Op::StoreLocal {
            local: t,
            index: None,
            src: Src { val: ValueId(val), swizzle },
            write_mask,
        };
        /* the R,G plane's look-up: its x,y are R,G */
        let lo = instrs.len() as u32;
        instrs.push(a, op)?;
        if sized {
            comps.push(a, 4)?;
        }
        if j.lo != 0 {
            instrs.push(a, store(lo, j.lo_swizzle, j.lo))?;
            if sized {
                comps.push(a, 0)?;
            }
        }
        /* the B,A plane's look-up on the companion: its x,y are B,A */
        if j.hi != 0 {
            let hi = instrs.len() as u32;
            let hi_op = match op {
                Op::Tex { dim, shadow, proj, ret, coord, lod, offset, .. } => {
                    Op::Tex { dim, shadow, proj, ret, sampler: companion, coord, lod, offset }
                }
                other => other,
            };
            instrs.push(a, hi_op)?;
            instrs.push(a, store(hi, j.hi_swizzle, j.hi))?;
            if sized {
                comps.extend_from_slice(a, &[4, 0])?;
            }
        }
        /* ZERO and ONE: GL's 0 and 1 in the look-up's own type, never the
         * bits of 1.0 in an integer sampler (int_tex retypes only the
         * look-ups, after this pass) */
        if j.konst != 0 {
            let k = instrs.len() as u32;
            let lane = |l: u32| j.ones >> l & 1;
            instrs.push(
                a,
                if ret == ir::ScalarType::F32 {
                    Op::ConstF32([0, 1, 2, 3].map(|l| (lane(l) as f32).to_bits()))
                } else {
                    Op::ConstI32([0, 1, 2, 3].map(|l| lane(l) as i32))
                },
            )?;
            instrs.push(a, store(k, ir::SWIZZLE_IDENTITY, j.konst))?;
            if sized {
                comps.extend_from_slice(a, &[4, 0])?;
            }
        }
        instrs.push(a, Op::LoadLocal { local: t, index: None, comps: 4 })?;
        if sized {
            comps.push(a, m.comps.get(i).copied().unwrap_or(4))?;
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
            /* renumbered: no precision (ir.rs `Module::prec`) */
            lanes: &[],
            prec: &[],
            local_prec: &[],
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
                if n != 0 {
                    for j in (1..=n).rev() {
                        level.push(a, Node::Instr(InstrId(id - j)))?;
                    }
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

/* The GC7000 compile path (`codegen::compile` is another GPU's in their
 * builds), and ES 3.00 sources, which a report-es2 build refuses. */
#[cfg(all(test, feature = "gpu-vivante", not(feature = "report-es2")))]
mod tests {
    use super::super::super::codegen;
    use super::super::super::lower;
    use super::super::super::sema::Stage;
    use super::super::super::store;
    use super::super::decode::decode_instr;
    use super::*;
    use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
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
        compile(SHC_STAGE_FRAGMENT, src, p)
    }

    /// [`compile_fs`] for either stage.
    fn compile(stage: u32, src: &str, p: Planes) -> Result<Vec<[u32; 4]>, String> {
        let _key = Keyed(set_compile_planes(p));
        let sh = store::with_scratch(|s| codegen::compile(stage, src.as_bytes(), 0, s, store::keep()))
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

    /// The front end declares no `textureGather`, so no gather reaches
    /// `split_planes` (the `Op` has no such form): a gather on a keyed
    /// sampler2D or usampler2D is refused by name, never half a texel, and
    /// an unkeyed one is refused the same way. When the front end learns the
    /// gather, this pin moves to the pass's own refusal (the 16-bit halves
    /// split is Mesa's etnaviv_nir_lower_128bit.c:115-151).
    #[test]
    fn a_gather_on_a_keyed_unit_is_refused_by_name() {
        let _k = store::test_keep();
        let p = Planes::NONE.with(0, 1);
        for (decl, out) in [("uniform highp sampler2D s;", "vec4"), ("uniform highp usampler2D s;", "uvec4")] {
            let src = fs(decl, out, "textureGather(s, v_uv)");
            for key in [p, Planes::NONE] {
                let e = compile_fs(&src, key).expect_err("refused");
                assert!(e.contains("textureGather") && e.contains("no function of this name is declared"), "{e}");
            }
        }
    }

    /* ---- lane sources (docs/notes/gc-128bit-swizzle.md sections 4, 5.2, 5.3) ---- */

    /// The module of one shader after the pass under `p`, as its
    /// instructions, or the refusal.
    fn split_ir(stage: Stage, src: &str, p: Planes) -> Result<Vec<Op>, String> {
        store::with_scratch(|a| {
            let text = a.alloc_str(src).map_err(|e| std::format!("{e:?}"))?;
            let l = lower::compile_to_ir(a, stage, text).map_err(|e| String::from(e.msg))?;
            let l = split_planes(a, l, p).map_err(|e| String::from(e.msg))?;
            Ok(l.module.instrs.to_vec())
        })
        .expect("the scratch region is free")
    }

    fn split_fs(src: &str, p: Planes) -> Vec<Op> {
        split_ir(Stage::Fragment, src, p).expect("split")
    }

    /// Where a joined lane comes from: lane `.1` of the look-up on unit
    /// `.0`, or a constant of the look-up's type.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Lane {
        Plane(u8, u8),
        F(f32),
        I(i32),
        Unset,
    }
    use Lane::{Plane, Unset, F, I};

    /// The lanes of every join, in program order: each store into a
    /// temporary from a look-up or a constant, decoded lane by lane.
    fn joins(ops: &[Op]) -> Vec<[Lane; 4]> {
        let mut out: Vec<(LocalId, [Lane; 4])> = Vec::new();
        for op in ops {
            let Op::StoreLocal { local, src, write_mask, .. } = *op else { continue };
            let i = match out.iter().position(|(l, _)| *l == local) {
                Some(i) => i,
                None => {
                    out.push((local, [Unset; 4]));
                    out.len() - 1
                }
            };
            for k in 0..4 {
                if write_mask & 1 << k == 0 {
                    continue;
                }
                let c = src.swizzle[k] as usize;
                out[i].1[k] = match ops[src.val.0 as usize] {
                    Op::Tex { sampler, .. } => Plane(sampler, c as u8),
                    Op::ConstF32(v) => F(f32::from_bits(v[c])),
                    Op::ConstI32(v) => I(v[c]),
                    other => panic!("a join stores {other:?}"),
                };
            }
        }
        out.into_iter().map(|(_, l)| l).collect()
    }

    fn tex_units(ops: &[Op]) -> Vec<u8> {
        ops.iter().filter_map(|op| if let Op::Tex { sampler, .. } = *op { Some(sampler) } else { None }).collect()
    }

    fn uint_fs() -> String {
        fs("uniform highp usampler2D s;", "uvec4", "texture(s, v_uv)")
    }

    fn int_fs() -> String {
        fs("uniform highp isampler2D s;", "ivec4", "texture(s, v_uv)")
    }

    /// The identity join, lane for lane.
    const RGBA: [Lane; 4] = [Plane(0, 0), Plane(0, 1), Plane(5, 0), Plane(5, 1)];

    /// GL's swizzle names as lane sources.
    const R: u8 = 0;
    const G: u8 = 1;
    const B: u8 = 2;
    const A: u8 = 3;

    #[test]
    fn the_key_carries_lane_sources() {
        let p = Planes::NONE.with(0, 5);
        assert_eq!(p.swizzle_of(0), IDENTITY, "with() keys the identity");
        assert_eq!(p.swizzle_of(3), IDENTITY, "a unit that is not keyed reads the identity");
        assert_eq!(p, Planes::NONE.with_swizzle(0, 5, IDENTITY));
        let q = Planes::NONE.with_swizzle(2, 7, [B, G, R, A]);
        assert_eq!((q.companion_of(2), q.swizzle_of(2)), (Some(7), [B, G, R, A]));
        assert_ne!(q, Planes::NONE.with(2, 7), "the lane sources are part of the key");
        assert_eq!(Planes::NONE.with_swizzle(40, 1, [A; 4]), Planes::NONE, "units past 31 are not keyed");
        /* no_alpha reads ONE wherever a source names alpha */
        let n = Planes::NONE.with_swizzle(0, 5, [A, R, A, A]).without_alpha(0);
        assert_eq!((n.swizzle_of(0), n.lanes_of(0)), ([A, R, A, A], [ONE, R, ONE, ONE]));
        /* the compile key round-trips, and an empty one reads as NONE */
        let _k = store::test_keep();
        let two = q.with_swizzle(9, 10, [ZERO, ONE, A, B]);
        let old = set_compile_planes(two);
        assert_eq!(compile_planes(), two);
        assert_eq!(set_compile_planes(old), two);
        assert_eq!(compile_planes(), Planes::NONE);
    }

    /// The identity unit is today's join: the same five instructions
    /// (masks 0b0011 and 0b1100, the companion's `.xyxy`), and a key that
    /// names the identity, or no alpha as `[R, G, B, ONE]`, is the key it
    /// always was, to the instruction word.
    #[test]
    fn an_identity_unit_is_the_join_as_it_was() {
        let ops = split_fs(&float_fs(), Planes::NONE.with(0, 5));
        let at = ops.iter().position(|op| matches!(op, Op::Tex { .. })).expect("a look-up");
        let base = at as u32;
        let Op::StoreLocal { local: t, .. } = ops[at + 1] else { panic!("{ops:#?}") };
        let store = |val: u32, swizzle: [u8; 4], write_mask: u8| Op::StoreLocal {
            local: t,
            index: None,
            src: Src { val: ValueId(val), swizzle },
            write_mask,
        };
        assert!(matches!(ops[at + 2], Op::Tex { sampler: 5, .. }), "{ops:#?}");
        assert_eq!(ops[at + 1], store(base, [0, 1, 2, 3], 0b0011));
        assert_eq!(ops[at + 3], store(base + 2, [0, 1, 0, 1], 0b1100));
        assert_eq!(ops[at + 4], Op::LoadLocal { local: t, index: None, comps: 4 });
        assert_eq!(split_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, IDENTITY)), ops);

        let na = split_fs(&float_fs(), Planes::NONE.with(0, 5).without_alpha(0));
        assert_eq!(na[at + 3], store(base + 2, [0, 1, 0, 1], 0b0100), "{na:#?}");
        assert_eq!(na[at + 4], Op::ConstF32([1f32.to_bits(); 4]));
        assert_eq!(na[at + 5], store(base + 4, [0, 1, 2, 3], 0b1000));
        assert_eq!(split_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, [R, G, B, ONE])).len(), na.len());

        let _k = store::test_keep();
        for src in [float_fs(), uint_fs(), int_fs()] {
            let plain = compile_fs(&src, Planes::NONE.with(0, 5)).expect("compiles");
            assert_eq!(compile_fs(&src, Planes::NONE.with_swizzle(0, 5, IDENTITY)).expect("compiles"), plain);
            let no_alpha = compile_fs(&src, Planes::NONE.with(0, 5).without_alpha(0)).expect("compiles");
            let one = compile_fs(&src, Planes::NONE.with_swizzle(0, 5, [R, G, B, ONE])).expect("compiles");
            assert_eq!(one, no_alpha, "no alpha is the lane source ONE: {src}");
        }
    }

    /// (B,G,R,A): lane 0 reads the companion's `.x`, lane 2 the first
    /// look-up's `.x`.
    #[test]
    fn bgra_swaps_lanes_across_the_planes() {
        let p = Planes::NONE.with_swizzle(0, 5, [B, G, R, A]);
        let ops = split_fs(&float_fs(), p);
        assert_eq!(joins(&ops), [[Plane(5, 0), Plane(0, 1), Plane(0, 0), Plane(5, 1)]]);
        let _k = store::test_keep();
        let ws = compile_fs(&float_fs(), p).expect("compiles");
        assert_eq!(units(&ws), [("texld", 0), ("texld", 5)], "{:#?}", listing(&ws));
        assert_ne!(ws, compile_fs(&float_fs(), Planes::NONE.with(0, 5)).expect("compiles"));
    }

    /// (A,A,A,A) reads only the B,A plane; the first look-up stays (the
    /// join's shape) and nothing is stored from it.
    #[test]
    fn all_alpha_reads_the_companion_only() {
        let ops = split_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, [A; 4]));
        assert_eq!(joins(&ops), [[Plane(5, 1); 4]]);
        assert_eq!(tex_units(&ops), [0, 5]);
        let _k = store::test_keep();
        let ws = compile_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, [A; 4])).expect("compiles");
        assert_eq!(units(&ws), [("texld", 5)], "selection drops the unread look-up: {:#?}", listing(&ws));
    }

    /// ZERO and ONE are constants of the look-up's type: 0.0/1.0 for a
    /// `sampler2D`, the integers 0/1 for `usampler2D` and `isampler2D`,
    /// never the bits of 1.0. No lane reads the companion, so it is not
    /// sampled.
    #[test]
    fn zero_and_one_are_constants_of_the_lookups_type() {
        let p = Planes::NONE.with_swizzle(0, 5, [ZERO, ONE, ZERO, ONE]);
        let ops = split_fs(&float_fs(), p);
        assert_eq!(joins(&ops), [[F(0.0), F(1.0), F(0.0), F(1.0)]]);
        assert_eq!(tex_units(&ops), [0]);
        for src in [uint_fs(), int_fs()] {
            let ops = split_fs(&src, p);
            assert_eq!(joins(&ops), [[I(0), I(1), I(0), I(1)]], "{src}");
            assert!(!ops.iter().any(|op| matches!(op, Op::ConstF32(_))), "{src}: {ops:#?}");
            assert!(!ops.iter().any(|op| matches!(op, Op::ConstI32(v) if v.contains(&0x3f80_0000))), "{src}");
        }
        /* the first look-up nothing reads is dropped by selection */
        let _k = store::test_keep();
        for src in [float_fs(), uint_fs(), int_fs()] {
            let ws = compile_fs(&src, p).expect("compiles");
            assert_eq!(units(&ws), [], "{src}: {:#?}", listing(&ws));
        }
    }

    /// No alpha alone, `[R, G, B, ONE]`: R,G,B from the planes, alpha the
    /// constant 1 of the type, as `without_alpha` gives.
    #[test]
    fn no_alpha_alone_is_lane_three_one() {
        let sw = [R, G, B, ONE];
        let ops = split_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, sw));
        assert_eq!(joins(&ops), [[Plane(0, 0), Plane(0, 1), Plane(5, 0), F(1.0)]]);
        assert_eq!(ops, split_fs(&float_fs(), Planes::NONE.with(0, 5).without_alpha(0)));
        let ops = split_fs(&uint_fs(), Planes::NONE.with_swizzle(0, 5, sw));
        assert_eq!(joins(&ops), [[Plane(0, 0), Plane(0, 1), Plane(5, 0), I(1)]]);
    }

    /// The nine swizzles of dEQP-GLES3 texture.swizzle
    /// (es3fTextureSwizzleTests.cpp:287-295, gc-texture-swizzle-wrap.md
    /// section 6 case 3) through the key: each lane's source in the join,
    /// and every one compiles for a float and an integer sampler.
    #[test]
    fn the_deqp_swizzles_through_the_key() {
        let c = |s: u8| match s {
            0..=3 => RGBA[s as usize],
            ZERO => F(0.0),
            _ => F(1.0),
        };
        let cases: [([u8; 4], &str); 9] = [
            ([R; 4], "all_red"),
            ([G; 4], "all_green"),
            ([B; 4], "all_blue"),
            ([A; 4], "all_alpha"),
            ([ZERO; 4], "all_zero"),
            ([ONE; 4], "all_one"),
            ([B, G, R, A], "bgra"),
            ([A, B, G, R], "abgr"),
            ([ONE, ONE, R, G], "one_one_red_green"),
        ];
        for (sw, name) in cases {
            let ops = split_fs(&float_fs(), Planes::NONE.with_swizzle(0, 5, sw));
            assert_eq!(joins(&ops), [sw.map(c)], "{name}");
            let companion = sw.iter().any(|&s| s == B || s == A);
            assert_eq!(tex_units(&ops), if companion { &[0, 5][..] } else { &[0][..] }, "{name}");
        }
        let _k = store::test_keep();
        for (sw, name) in cases {
            for src in [float_fs(), uint_fs()] {
                compile_fs(&src, Planes::NONE.with_swizzle(0, 5, sw)).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
    }

    /// Two keyed units in one shader, each with its own lane sources.
    #[test]
    fn two_units_are_swizzled_independently() {
        let src = fs(
            "uniform highp sampler2D s;\nuniform highp sampler2D t;",
            "vec4",
            "texture(s, v_uv) + texture(t, v_uv * 2.0)",
        );
        let p = Planes::NONE.with_swizzle(0, 4, [B, G, R, A]).with_swizzle(1, 5, [ONE, A, ZERO, R]);
        let ops = split_fs(&src, p);
        assert_eq!(
            joins(&ops),
            [[Plane(4, 0), Plane(0, 1), Plane(0, 0), Plane(4, 1)], [F(1.0), Plane(5, 1), F(0.0), Plane(1, 0)]]
        );
        let q = Planes::NONE.with_swizzle(0, 4, [B, G, R, A]).with(1, 5);
        assert_eq!(joins(&split_fs(&src, q))[1], [Plane(1, 0), Plane(1, 1), Plane(5, 0), Plane(5, 1)]);
        let _k = store::test_keep();
        let ws = compile_fs(&src, p).expect("compiles");
        assert_eq!(units(&ws), [("texld", 0), ("texld", 4), ("texld", 1), ("texld", 5)], "{:#?}", listing(&ws));
    }

    /// The vertex stage's key has its own lane sources.
    #[test]
    fn a_vertex_lookup_is_swizzled_by_its_own_key() {
        let vs = "#version 300 es\nin vec2 a;\nuniform highp sampler2D s;\n\
                  void main() { gl_Position = textureLod(s, a, 0.0); }";
        let p = Planes::NONE.with_swizzle(0, 3, [A, B, G, R]);
        let ops = split_ir(Stage::Vertex, vs, p).expect("split");
        assert_eq!(joins(&ops), [[Plane(3, 1), Plane(3, 0), Plane(0, 1), Plane(0, 0)]]);
        let _k = store::test_keep();
        let ws = compile(SHC_STAGE_VERTEX, vs, p).expect("compiles");
        assert_eq!(units(&ws).len(), 2, "{:#?}", listing(&ws));
    }

    /// `textureSize` on a keyed unit reads the primary unit's size as it
    /// did; `texelFetch` is joined and swizzled like `texture`.
    #[test]
    fn texture_size_is_unchanged_and_texel_fetch_is_swizzled() {
        let src = fs("uniform highp usampler2D s;", "uvec4", "texelFetch(s, textureSize(s, 0) - 1, 0)");
        let sw = [ONE, B, R, ZERO];
        let ops = split_fs(&src, Planes::NONE.with_swizzle(0, 5, sw));
        let sizes = |ops: &[Op]| ops.iter().filter(|op| matches!(op, Op::TexSize { sampler: 0, .. })).count();
        assert_eq!(sizes(&ops), 1);
        assert_eq!(sizes(&ops), sizes(&split_fs(&src, Planes::NONE)));
        assert_eq!(joins(&ops), [[I(1), Plane(5, 0), Plane(0, 0), I(0)]]);
        let _k = store::test_keep();
        compile_fs(&src, Planes::NONE.with_swizzle(0, 5, sw)).expect("compiles");
    }

    /// A lane source past ONE is no lane: refused by name.
    #[test]
    fn a_source_that_is_no_lane_is_refused() {
        let e = split_ir(Stage::Fragment, &float_fs(), Planes::NONE.with_swizzle(0, 5, [R, G, B, 6])).expect_err("no");
        assert!(e.contains("lane source is not a lane, zero or one"), "{e}");
    }
}
