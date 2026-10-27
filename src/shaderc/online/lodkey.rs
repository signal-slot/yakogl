// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Sampler-state variants in the IR: which look-ups' code depends on the
//! filter state of the sampler they read ([`crate::shaderc::LodKey`]), and
//! the c = 0.5 remap of an explicit level.
//!
//! GL ES 2.0 3.7.7-3.7.8 (ES 3.0 3.8.10-3.8.11): with TEXTURE_MAG_FILTER
//! LINEAR and TEXTURE_MIN_FILTER NEAREST_MIPMAP_NEAREST or
//! NEAREST_MIPMAP_LINEAR, the switch between magnification and minification
//! is at lambda = c = 0.5, and at 0 otherwise. The PowerVR TPU and the GC7000
//! TE have no threshold register: they magnify at lambda <= 0 always (Mesa
//! pvr_arch_sampler.c; Mesa's etnaviv-gc7000-r6204-fails.txt lists the six
//! dEQP cases this costs). For a look-up whose level is EXPLICIT the shader
//! can move the switch itself:
//!
//! ```text
//!   lod' = lod <= 0.5 ? -1.0 : lod
//! ```
//!
//! In (0, 0.5] GL magnifies, and a negative level makes the hardware magnify
//! (the PowerVR clamps it to its minlod 0, which magnifies; the GC7000 the
//! same through its LOD_MIN 0). Above 0.5 both minify with the same lambda:
//! NEAREST_MIPMAP_NEAREST reads level ceil(lambda + 0.5) - 1, and
//! NEAREST_MIPMAP_LINEAR blends floor(lambda) and floor(lambda) + 1 by
//! frac(lambda) -- the lambda is passed unchanged, so (0.5, 1) blends levels
//! 0 and 1 exactly as GL does. At or below 0 nothing changes (the hardware
//! already magnifies). The test below holds the remap to the rule for both
//! filters.
//!
//! An implicit level (a fragment look-up without `Lod`) is the hardware's to
//! compute and is not remapped here; a bias is not either.

use super::arena::{AVec, Arena};
use super::ir::{self, AluOp, InstrId, Module, Node, Op, Range, Src, TexLod, ValueId};
use super::lower::Lowered;
use super::R;

/// The switch GL puts at 0.5 for these filters.
pub const MAG_SWITCH_C: f32 = 0.5;
/// The level a remapped look-up passes when GL magnifies: below the
/// hardware's switch at 0, whatever its clamp.
pub const MAGNIFY_LOD: f32 = -1.0;

/// What the remap does to one look-up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Remap {
    /// Not a look-up the remap concerns.
    None,
    /// A constant level in (0, 0.5]: replaced by [`MAGNIFY_LOD`].
    Fold,
    /// A level computed at run time: `lod <= 0.5 ? -1.0 : lod`, three
    /// instructions.
    Select,
}

/// The remap `op` would take, and the sampler binding it reads.
fn classify(m: &Module<'_>, op: &Op) -> (Remap, u8) {
    let Op::Tex { lod: TexLod::Lod(l), sampler, .. } = *op else {
        return (Remap::None, 0);
    };
    if sampler >= 32 {
        return (Remap::None, sampler);
    }
    if let Some(Op::ConstF32(c)) = m.def(l.val) {
        let v = f32::from_bits(c[usize::from(l.swizzle[0] & 3)]);
        /* at or below 0 the hardware magnifies already; above 0.5 GL
         * minifies with this lambda; a NaN level is left alone */
        return if v > 0.0 && v <= MAG_SWITCH_C { (Remap::Fold, sampler) } else { (Remap::None, sampler) };
    }
    (Remap::Select, sampler)
}

/// The bindings whose explicit-level look-ups the remap would change: the
/// module's [`crate::shaderc::LodKey::mag_switch`] sensitivity. A vertex
/// shader's implicit look-up is `Lod(0.0)` (the base level) and never
/// counts.
pub fn mag_switch_sensitivity(m: &Module<'_>) -> u32 {
    m.instrs.iter().fold(0u32, |acc, op| match classify(m, op) {
        (Remap::None, _) => acc,
        (_, s) => acc | 1 << s,
    })
}

/// The look-ups of the bindings in `mask` with their explicit level
/// remapped (module doc). Instructions keep program order in their ids: the
/// new ones are numbered right before the look-up they feed and every later
/// id moves up, and each gets its node right before the look-up's node. A
/// module with nothing to remap comes back as it was.
pub fn remap_mag_switch<'a>(a: &Arena<'a>, l: Lowered<'a>, mask: u32) -> R<'a, Lowered<'a>> {
    let m = l.module;
    if mask == 0 {
        return Ok(l);
    }
    let n = m.instrs.len();
    let plan = |i: usize| -> Remap {
        match classify(&m, &m.instrs[i]) {
            (r, s) if r != Remap::None && mask & (1 << s) != 0 => r,
            _ => Remap::None,
        }
    };
    if !(0..n).any(|i| plan(i) != Remap::None) {
        return Ok(l);
    }
    let extra = |r: Remap| -> u32 {
        match r {
            Remap::None => 0,
            Remap::Fold => 1,
            Remap::Select => 3,
        }
    };
    /* old id -> new id: the instructions inserted before it, its own
     * included */
    let mut new_id: AVec<u32> = AVec::with_capacity(a, n)?;
    let mut acc = 0u32;
    for i in 0..n {
        acc += extra(plan(i));
        new_id.push(a, i as u32 + acc)?;
    }
    let new_id = new_id.into_slice();
    let map = |v: ValueId| ValueId(new_id.get(v.0 as usize).copied().unwrap_or(v.0));

    let total = n + acc as usize;
    let mut instrs: AVec<Op> = AVec::with_capacity(a, total)?;
    let sized = !m.comps.is_empty();
    let mut comps: AVec<u8> = AVec::with_capacity(a, if sized { total } else { 0 })?;
    for (i, op) in m.instrs.iter().enumerate() {
        let mut op = map_op(*op, &map);
        let r = plan(i);
        if let (true, Op::Tex { lod: TexLod::Lod(lod), .. }) = (r != Remap::None, op) {
            let lane0 = Src { val: lod.val, swizzle: [lod.swizzle[0]; 4] };
            let base = instrs.len() as u32;
            let level = if r == Remap::Fold {
                instrs.push(a, Op::ConstF32([MAGNIFY_LOD.to_bits(); 4]))?;
                if sized {
                    comps.push(a, 4)?;
                }
                Src::splat(ValueId(base))
            } else {
                let k = ValueId(base);
                instrs.push(a, Op::ConstF32([MAG_SWITCH_C.to_bits(), MAGNIFY_LOD.to_bits(), 0, 0]))?;
                let c = Src { val: k, swizzle: [0; 4] };
                let neg = Src { val: k, swizzle: [1; 4] };
                instrs.push(a, Op::Alu(AluOp::Fle, [lane0, c, lane0]))?;
                let le = Src::splat(ValueId(base + 1));
                instrs.push(a, Op::Alu(AluOp::Bcsel, [le, neg, lane0]))?;
                if sized {
                    comps.extend_from_slice(a, &[4, 1, 1])?;
                }
                Src::splat(ValueId(base + 2))
            };
            if let Op::Tex { lod: ref mut lv, .. } = op {
                *lv = TexLod::Lod(level);
            }
        }
        instrs.push(a, op)?;
        if sized {
            comps.push(a, m.comps.get(i).copied().unwrap_or(0))?;
        }
    }

    /* the nodes: every block copied with the new ids, children first, so
     * each block's nodes stay contiguous */
    let mut nodes: AVec<Node> = AVec::with_capacity(a, m.nodes.len() + acc as usize)?;
    let body = copy_block(a, &m, m.body, &map, &plan, &extra, new_id, &mut nodes)?;
    Ok(Lowered {
        module: ir::Module {
            instrs: instrs.into_slice(),
            comps: if sized { comps.into_slice() } else { m.comps },
            nodes: nodes.into_slice(),
            body,
            ..m
        },
        reflection: l.reflection,
    })
}

/// One block of `m.nodes` into `out`, with the new ids.
#[allow(clippy::too_many_arguments)]
fn copy_block<'a>(
    a: &Arena<'a>,
    m: &Module<'_>,
    r: Range,
    map: &dyn Fn(ValueId) -> ValueId,
    plan: &dyn Fn(usize) -> Remap,
    extra: &dyn Fn(Remap) -> u32,
    new_id: &[u32],
    out: &mut AVec<'a, Node>,
) -> R<'a, Range> {
    let mut level: AVec<Node> = AVec::new();
    for &node in m.nodes_of(r) {
        let copied = match node {
            Node::Instr(InstrId(i)) => {
                let k = extra(plan(i as usize));
                let id = new_id.get(i as usize).copied().unwrap_or(i);
                /* the inserted instructions, right before the look-up */
                for j in (1..=k).rev() {
                    level.push(a, Node::Instr(InstrId(id - j)))?;
                }
                Node::Instr(InstrId(id))
            }
            Node::If { cond, then_, else_ } => {
                let then_ = copy_block(a, m, then_, map, plan, extra, new_id, out)?;
                let else_ = copy_block(a, m, else_, map, plan, extra, new_id, out)?;
                Node::If { cond: Src { val: map(cond.val), ..cond }, then_, else_ }
            }
            Node::Loop { body } => Node::Loop { body: copy_block(a, m, body, map, plan, extra, new_id, out)? },
            other => other,
        };
        level.push(a, copied)?;
    }
    let first = out.len() as u32;
    out.extend_from_slice(a, level.as_slice())?;
    Ok(Range { first, len: level.len() as u32 })
}

/// `op` with every value it reads renamed by `map`. Exhaustive, as
/// [`ir::Op::for_each_src`] is, so an operation added later is not missed.
fn map_op(op: Op, map: &dyn Fn(ValueId) -> ValueId) -> Op {
    let ms = |s: Src| Src { val: map(s.val), ..s };
    match op {
        Op::ConstF32(_) | Op::ConstI32(_) | Op::ConstBool(_) | Op::LoadUniform { .. } | Op::LoadInput { .. } => op,
        Op::Discard => op,
        Op::Alu(o, srcs) => Op::Alu(o, srcs.map(ms)),
        Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } => {
            Op::LoadUniformIdx { base_dw, stride_dw, len, index: ms(index), comps }
        }
        Op::StoreOutput { slot, src } => Op::StoreOutput { slot, src: ms(src) },
        Op::LoadLocal { local, index, comps } => Op::LoadLocal { local, index: index.map(ms), comps },
        Op::StoreLocal { local, index, src, write_mask } => {
            Op::StoreLocal { local, index: index.map(ms), src: ms(src), write_mask }
        }
        Op::Tex { dim, shadow, proj, ret, sampler, coord, lod, offset } => {
            let lod = match lod {
                TexLod::Implicit => TexLod::Implicit,
                TexLod::Bias(s) => TexLod::Bias(ms(s)),
                TexLod::Lod(s) => TexLod::Lod(ms(s)),
                TexLod::Fetch(s) => TexLod::Fetch(ms(s)),
                TexLod::Grad { ddx, ddy } => TexLod::Grad { ddx: map(ddx), ddy: map(ddy) },
            };
            Op::Tex { dim, shadow, proj, ret, sampler, coord: ms(coord), lod, offset }
        }
        Op::TexSize { sampler, dim, lod } => Op::TexSize { sampler, dim, lod: ms(lod) },
    }
}

#[cfg(test)]
mod tests {
    use super::super::eval;
    use super::super::ir::IoSlot;
    use super::super::sema::Stage;
    use super::super::store;
    use super::*;
    use std::vec::Vec;

    /// The two minification filters the rule concerns.
    #[derive(Clone, Copy, Debug)]
    enum Min {
        NearestMipmapNearest,
        NearestMipmapLinear,
    }

    /// What a look-up reads, by GL ES 2.0 3.7.7-3.7.8 with TEXTURE_MAG_FILTER
    /// LINEAR, the switch at `c`, over `levels` levels.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Pick {
        /// Magnified: LINEAR on level 0.
        Magnified,
        /// Minified, one level, NEAREST.
        Level(u32),
        /// Minified, two levels NEAREST blended by the weight of the second.
        Blend(u32, u32, f32),
    }

    fn pick(lambda: f32, min: Min, c: f32, levels: u32) -> Pick {
        if lambda <= c {
            return Pick::Magnified;
        }
        let top = levels as i32 - 1;
        match min {
            /* d = ceil(lambda + 0.5) - 1 (3.7.7, "for lambda > 1/2") */
            Min::NearestMipmapNearest => Pick::Level(((lambda + 0.5).ceil() as i32 - 1).clamp(0, top) as u32),
            Min::NearestMipmapLinear => {
                let l0 = (lambda.floor() as i32).clamp(0, top) as u32;
                let l1 = (l0 + 1).min(top as u32);
                let f = lambda - lambda.floor();
                if f == 0.0 || l0 == l1 {
                    Pick::Level(l0)
                } else {
                    Pick::Blend(l0, l1, f)
                }
            }
        }
    }

    /// The level the remapped vertex shader hands its look-up, for `lod`:
    /// the module run in the IR evaluator with the look-up replaced by its
    /// level operand.
    fn remapped_level(src: &str, lod: f32) -> f32 {
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = super::super::lower::compile_to_ir(a, Stage::Vertex, text).expect("lowers");
            assert_eq!(mag_switch_sensitivity(&l.module), 1, "one explicit-level look-up of binding 0");
            let l = remap_mag_switch(a, l, 1).expect("remaps");
            let m = l.module;
            let mut instrs: Vec<Op> = m.instrs.to_vec();
            let mut n_tex = 0;
            for op in instrs.iter_mut() {
                if let Op::Tex { lod: TexLod::Lod(s), .. } = *op {
                    *op = Op::Alu(AluOp::Fmax, [s, s, s]);
                    n_tex += 1;
                }
            }
            assert_eq!(n_tex, 1);
            let m2 = Module { instrs: &instrs, ..m };
            let off = m.uniforms.iter().find(|u| u.name == "u_lod").expect("u_lod").dword_off as usize;
            let mut uniforms = std::vec![0u32; m.uniform_dwords.max(1) as usize];
            uniforms[off] = lod.to_bits();
            let io = [(IoSlot::Attrib(0), eval::f32x4([0.3, -0.2, 1.0, 1.0]))];
            let out = eval::run(&m2, &eval::Inputs { io: &io, uniforms: &uniforms }).expect("runs");
            let v = m.outputs.iter().find(|o| o.name == "v").expect("v").location;
            out.output_f32(IoSlot::Varying(v)).expect("v stored")[0]
        })
        .unwrap()
    }

    const VS_CUBE: &str = "attribute vec4 a; uniform highp float u_lod; uniform samplerCube s; varying vec4 v; \
                           void main(){ v = textureCubeLod(s, a.xyz, u_lod); gl_Position = a; }";
    const VS_2D: &str = "attribute vec4 a; uniform highp float u_lod; uniform sampler2D s; varying vec4 v; \
                         void main(){ v = texture2DLod(s, a.xy, u_lod); gl_Position = a; }";

    /// The remap, run in the IR evaluator, on hardware that switches at 0,
    /// reads what GL ES 2.0 3.7.8 reads with the switch at 0.5 -- for both
    /// minification filters, a 2D and a cube look-up, and the levels of
    /// detail around the switch. Without it the two differ exactly in
    /// (0, 0.5] (the six dEQP cases sit at log2(32/28) = 0.19).
    #[test]
    fn the_remap_on_a_switch_at_zero_reads_what_gl_reads_at_one_half() {
        let lambdas = [0.0f32, 0.25, 0.5, 0.75, 1.0, 1.5, 0.192_645, -1.5, 3.0];
        for src in [VS_CUBE, VS_2D] {
            for &lambda in &lambdas {
                let hw_lod = remapped_level(src, lambda);
                for min in [Min::NearestMipmapNearest, Min::NearestMipmapLinear] {
                    let gl = pick(lambda, min, MAG_SWITCH_C, 6);
                    let hw = pick(hw_lod, min, 0.0, 6);
                    assert_eq!(hw, gl, "lambda {lambda} -> {hw_lod}, {min:?}");
                    /* the unremapped program differs exactly in (0, 0.5] */
                    let before = pick(lambda, min, 0.0, 6);
                    assert_eq!(before == gl, !(lambda > 0.0 && lambda <= 0.5), "lambda {lambda} unremapped, {min:?}");
                }
                /* the level itself: unchanged above the switch, -1 at or
                 * below it (where the hardware magnifies either way) */
                let want = if lambda > MAG_SWITCH_C { lambda } else { MAGNIFY_LOD };
                assert_eq!(hw_lod.to_bits(), want.to_bits(), "lambda {lambda}");
            }
        }
    }

    fn lowered_sensitivity(stage: Stage, src: &str) -> u32 {
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = super::super::lower::compile_to_ir(a, stage, text).expect("lowers");
            mag_switch_sensitivity(&l.module)
        })
        .unwrap()
    }

    /// Which look-ups count: an explicit level computed at run time, or a
    /// constant in (0, 0.5]; not a vertex shader's implicit look-up (the
    /// base level, `Lod(0.0)`), not a constant level outside that band, not
    /// a fragment shader's implicit or biased look-up. Bit b is binding b.
    #[test]
    fn only_an_explicit_level_that_can_fall_in_the_band_is_sensitive() {
        let v = |body: &str| {
            std::format!(
                "attribute vec4 a; uniform highp float u_lod; uniform sampler2D s0; uniform samplerCube s1; \
                 varying vec4 v; void main(){{ {body} gl_Position = a; }}"
            )
        };
        assert_eq!(lowered_sensitivity(Stage::Vertex, &v("v = texture2D(s0, a.xy);")), 0);
        assert_eq!(lowered_sensitivity(Stage::Vertex, &v("v = textureCube(s1, a.xyz);")), 0);
        assert_eq!(lowered_sensitivity(Stage::Vertex, &v("v = texture2DLod(s0, a.xy, 2.0);")), 0);
        assert_eq!(lowered_sensitivity(Stage::Vertex, &v("v = texture2DLod(s0, a.xy, 0.25);")), 1);
        assert_eq!(lowered_sensitivity(Stage::Vertex, &v("v = textureCubeLod(s1, a.xyz, u_lod);")), 2);
        assert_eq!(
            lowered_sensitivity(
                Stage::Vertex,
                &v("v = texture2DLod(s0, a.xy, u_lod) + textureCubeLod(s1, a.xyz, u_lod);")
            ),
            3
        );
        let f = "precision mediump float; uniform sampler2D s0; uniform float b; varying vec2 uv; \
                 void main(){ gl_FragColor = texture2D(s0, uv) + texture2D(s0, uv, b); }";
        assert_eq!(lowered_sensitivity(Stage::Fragment, f), 0);
    }

    /// A remapped module runs as the original does wherever the level is
    /// outside the band, in control flow too: the ids are renumbered and
    /// the blocks rebuilt, and a program that branches and loops around the
    /// look-up gives the same outputs.
    #[test]
    fn the_renumbered_module_keeps_its_control_flow() {
        let src = "attribute vec4 a; uniform highp float u_lod; uniform sampler2D s; varying vec4 v; varying vec4 w; \
                   void main(){ vec4 acc = vec4(0.0); \
                   for (int i = 0; i < 3; i++) { if (a.x > 0.0) { acc += texture2DLod(s, a.xy, u_lod + float(i)); } \
                   else { acc -= a; } } v = acc; w = a * 2.0; gl_Position = a; }";
        let run = |remap: bool, lod: f32, x: f32| -> ([f32; 4], [f32; 4]) {
            store::with_scratch(|a| {
                let text = a.alloc_str(src).unwrap();
                let mut l = super::super::lower::compile_to_ir(a, Stage::Vertex, text).expect("lowers");
                if remap {
                    l = remap_mag_switch(a, l, 1).expect("remaps");
                }
                let m = l.module;
                let instrs: Vec<Op> = m
                    .instrs
                    .iter()
                    .map(|op| match *op {
                        Op::Tex { lod: TexLod::Lod(s), .. } => Op::Alu(AluOp::Fmax, [s, s, s]),
                        other => other,
                    })
                    .collect();
                let m2 = Module { instrs: &instrs, ..m };
                let off = m.uniforms.iter().find(|u| u.name == "u_lod").unwrap().dword_off as usize;
                let mut uniforms = std::vec![0u32; m.uniform_dwords.max(1) as usize];
                uniforms[off] = lod.to_bits();
                let io = [(IoSlot::Attrib(0), eval::f32x4([x, 0.5, 0.25, 1.0]))];
                let out = eval::run(&m2, &eval::Inputs { io: &io, uniforms: &uniforms }).expect("runs");
                let loc = |n: &str| m.outputs.iter().find(|o| o.name == n).unwrap().location;
                (out.output_f32(IoSlot::Varying(loc("v"))).unwrap(), out.output_f32(IoSlot::Varying(loc("w"))).unwrap())
            })
            .unwrap()
        };
        for (lod, x) in [(0.75f32, 1.0f32), (2.0, 1.0), (-2.0, 1.0), (0.75, -1.0)] {
            assert_eq!(run(true, lod, x), run(false, lod, x), "lod {lod} x {x}");
        }
        /* in the band the first iteration's level is remapped: 0.25 -> -1,
         * then 1.25 and 2.25 unchanged */
        let (v, _) = run(true, 0.25, 1.0);
        assert_eq!(v[0], -1.0 + 1.25 + 2.25);
    }

    /// No bit, or no look-up of the bit's binding: the module is returned
    /// as it came, the same slices.
    #[test]
    fn nothing_to_remap_leaves_the_module_alone() {
        store::with_scratch(|a| {
            let text = a.alloc_str(VS_2D).unwrap();
            let l = super::super::lower::compile_to_ir(a, Stage::Vertex, text).expect("lowers");
            let before = l.module.instrs.as_ptr();
            let l = remap_mag_switch(a, l, 0).unwrap();
            assert_eq!(l.module.instrs.as_ptr(), before);
            let l = remap_mag_switch(a, l, 2).unwrap();
            assert_eq!(l.module.instrs.as_ptr(), before);
            let n = l.module.instrs.len();
            let l = remap_mag_switch(a, l, 1).unwrap();
            assert_eq!(l.module.instrs.len(), n + 3, "a constant, a compare and a select");
            assert_eq!(l.module.comps.len(), l.module.instrs.len());
        })
        .unwrap();
    }
}
