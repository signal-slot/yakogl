// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A fragment shader on the shared program graph ([`Nir`]): the general
//! lowering's fragment stage (driver/docs/nanopc-t6-mali.md, "The general
//! lowering", S1). The shapes it takes, built as Mesa's
//! NIR has them, then selected through `bi_emit_alu`'s rows (emit.rs) and
//! Mesa's BIR passes (opt.rs):
//!
//! * the walk: each IR value as scalar lanes, the arithmetic as 16-bit
//!   nodes (the mediump lowering), the algebraic rules applied to a 16-bit
//!   lane pair (`bi_vectorize_filter` splits a mediump op into vec2s,
//!   bifrost_nir.c:1088, before the algebraic passes) ([`walk`]);
//! * the varyings: the lanes Mesa's link keeps (those read, or every
//!   lane of one a lowered tree converts, [`kept_whole`]), compacted from
//!   VAR0 in layout order (`compact_varyings`), each slot read loaded at
//!   its width where its first read stands ([`layout`]);
//! * the pairs: lanes `2p` and `2p + 1` of the colour as one two-lane node
//!   per operation (`nir_opt_vectorize` at the 16-bit width of two), the
//!   late fusion where the fadd stands, `fmul(a, 2.0)` as `fadd(a, a)`
//!   ([`pair`], [`late_rules`]);
//! * the selection: each load and its conversions, each two-lane operation
//!   as its row, every fneg and fabs as `FABSNEG` (the passes fold it into
//!   the users that take it, or make it an FADD), ATEST and BLEND
//!   ([`select`]).
//!
//! A highp shader ([`FsPrecision`]) is the vertex stage's form instead: its
//! arithmetic is scalar fp32 nodes (`bi_vectorize_filter` gives a 32-bit
//! op width 1, bifrost_nir.c:176-179) under the vertex stage's rules and
//! fusion (nir.rs), its loads stay fp32, its uniforms are the vertex
//! stage's fp32 loads of Mesa's packed block ([`uniform_load`], read as FAU
//! words by `bi_opt_push_ubo`, fau.rs), and only the colour is converted
//! to fp16 at its store ([`select_highp`]).
//!
//! A shape it does not take is refused with its own gap ([`MaliError`]).

use super::bir::{clamp, Cmpf, Dest, Idx, Ins, Op, Prog, Special, Src, Swz, VaryingName};
use super::emit::{self, Row};
use super::motion::{Moved, MAX_MOVED};
use super::nir::{self, Alu, Kind, Nir, NirNode, Val, IDIV_RCP_SCALE, MAX_LOCALS, MAX_NODES, MAX_VALUES};
use super::select::{
    alu, f16_exact, f16_rtne, generic_layout, preloads, split, UboLayout, VaryingAt, LOOP_CAP, MAX_VARYINGS,
};
use super::{Defect, FsPrecision, MaliError};
use crate::gl::consts::{
    GL_BOOL, GL_BOOL_VEC2, GL_BOOL_VEC3, GL_BOOL_VEC4, GL_FLOAT, GL_INT, GL_INT_VEC2, GL_INT_VEC3, GL_INT_VEC4,
};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::{self, AluOp, Es100Tex, IoKind, IoSlot, Module, Node};

/// The vectorised operations and operands of the colour: a pair node is
/// named by a `u8` ([`P::Op`]), so up to 255 (case 166's sixteen look-ups
/// and their sums take 126, unitype_166c).
pub const MAX_PAIR_NODES: usize = 255;

/// The `demote_if`s one shader may hold ([`Fs::discard`]).
pub const MAX_DISCARDS: usize = 8;

/// The colour outputs a shader may write, one a render target
/// (`BIR_FAU_BLEND_0 ... + 7`, va_pack.c:100-101).
pub const MAX_TARGETS: usize = 8;

/// The run-time indexed uniform reads one shader may make ([`Fs::ubo_idx`]).
pub const MAX_UBO_READS: usize = 8;

/// The products of compare selects one shader keeps the chain of
/// ([`Fs::cmp_product`]); a product past them folds its masks in source
/// order.
const MAX_CMP_CHAIN: usize = 32;

/// `nir_opt_loop_unroll`'s bounds as bifrost sets them: a loop of at most
/// `max_unroll_iterations` passes (bifrost_compile.h:140-144, read at
/// nir_opt_loop_unroll.c:899-931) whose unrolled body costs at most
/// `LOOP_UNROLL_LIMIT * max_unroll_iterations` instructions (:36, :925)
/// is unrolled; any other keeps its back edge, which no reference draws
/// ([`MaliError::LoopForm`]).
pub const MAX_UNROLL_ITERATIONS: usize = 32;
pub const LOOP_UNROLL_LIMIT: usize = 26;

/// How a run of nodes ends ([`walk_nodes`]): by falling through, or by a
/// `break` or `continue` of the loop being unrolled around it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flow {
    Next,
    Break,
    Continue,
}

/// The coordinate of a fragment shader's look-up: the two lanes of one
/// vec2 varying read whole, or two constants (gl_test cases 122 and 197's
/// `vec2(0.5)`). `va_lower_tex` makes each lane a staging register
/// whatever it is (pan_nir_lower_tex.c:831-832); a constant's is a
/// `COLLECT` source the constant passes make a FAU word, moved into the
/// staging register (tools/mali/testdata/tex_coord_const.expect.json:
/// `MOV.i32 r0, u2.w0`, `MOV.i32 r1, u2.w0`). Any other coordinate is
/// `Computed`: its two lanes are the look-up node's `src[0]` and `src[1]`,
/// values of the graph like the colour's, each a staging word as it is
/// (measured: fvg_fill_image.expect.json's `MOV.i32 r6, r5^`, `MOV.i32 r7,
/// r4^` of `(paintMat * vec3(fpos, 1.0)).xy / extent`;
/// docs/notes/mali-texture-refusals.md, section (d)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexCoord {
    Varying(u8),
    Const(u32, u32),
    Computed,
}

/// A shader's texture look-up: `texture2D(sampler, coordinate)`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TexLookup {
    /// The sampler binding (`Op::Tex::sampler`): which GL sampler the
    /// descriptors at [`TEX_INDEX`] describe.
    pub sampler: u8,
    pub coord: TexCoord,
    /// A cube map's look-up (`textureLod(samplerCube, dir, lod)`,
    /// [`walk_nodes`]): its coordinate is [`TexCoord::Computed`], the
    /// look-up node's three sources.
    pub cube: Option<Cube>,
    /// A `texelFetch` at level 0 of a highp shader ([`walk_nodes`]): its
    /// coordinate is [`TexCoord::Computed`], two integer lanes, and the
    /// look-up `TEX_FETCH` ([`super::bir::LodMode::Fetch`]).
    pub fetch: bool,
    /// A 2D array's look-up of a mediump shader ([`walk_nodes`]; case 42
    /// es3array): its coordinate is [`TexCoord::Computed`], three lanes
    /// (the layer the third, fp32 of a look-up and an integer of a
    /// `texelFetch`), the dimension `2d.array_enable`, and the texels
    /// written those the colour reads ([`tex_mask`]).
    pub array: bool,
    /// A 3D texture's look-up of a highp shader ([`walk_nodes`]; case 48
    /// es3vol, texmore_48): its coordinate is [`TexCoord::Computed`], three
    /// lanes (fp32 of `texture` and `textureLod`, integers of a
    /// `texelFetch`), the dimension `3d`.
    pub d3: bool,
    /// `textureLod(s, p, 0.0)`: the level the constant 0, `ZERO_LOD` with
    /// no level word ([`super::bir::LodMode::Zero`]; `nir_scalar_is_zero`,
    /// pan_nir_lower_tex.c:929-930; texmore_48's `TEX_SINGLE ... .3d.zero`).
    pub zero: bool,
    /// `textureCube(s, dir, bias)`'s bias (case 101 cubebias), a mediump
    /// uniform's lane, else undefined: `COMPUTED_BIAS`
    /// ([`super::bir::LodMode::ComputedBias`]), the third staging word its
    /// 8.8 form ([`select`]).
    pub bias: Val,
    /// `texture(sampler2DShadow, vec3)` (case 218 shadowcmp,
    /// docs/notes/mali-fragcoord-fp16-ops.md): the coordinate is
    /// [`TexCoord::Computed`] with the reference as its third lane
    /// (`nir_tex_src_comparator`, the third staging word), the look-up
    /// `TEX_SINGLE ... .shadow` (`compare_enable`) writing `.r` alone, the
    /// one lane the node has.
    pub shadow: bool,
    /// The IR instruction of the look-up.
    pub at: u16,
    /// The look-up's node (the texels' lanes).
    pub node: u16,
    /// The texture and sampler index of its descriptors ([`tex_handle`]),
    /// set by [`layout`].
    pub index: u8,
    /// Its coordinate's load (none for a constant or computed coordinate),
    /// set by [`layout`].
    pub load: Option<u8>,
    /// The lane of [`TexLookup::load`] the coordinate starts at, set by
    /// [`layout`]: 0, or 2 for a vec2 varying the link packed after
    /// another in one slot (gm2_fclane_2's `t1` beside `t0`).
    pub load_lane: u8,
    /// The texels the colour reads (bit `c`: texel `c`), set by [`layout`].
    pub lanes: u8,
}

impl TexLookup {
    /// A look-up at IR instruction `at`, its node `node`.
    fn new(sampler: u8, coord: TexCoord, cube: Option<Cube>, fetch: bool, at: usize, node: u16) -> TexLookup {
        TexLookup { sampler, coord, cube, fetch, at: at as u16, node, ..TexLookup::NONE }
    }

    const NONE: TexLookup = TexLookup {
        sampler: 0,
        coord: TexCoord::Computed,
        cube: None,
        fetch: false,
        array: false,
        d3: false,
        zero: false,
        bias: Val::Undef,
        shadow: false,
        at: 0,
        node: 0,
        index: 0,
        load: None,
        load_lane: 0,
        lanes: 0,
    };

    /// The colour reads it ([`layout`]; Mesa's DCE takes one it does not).
    pub fn live(&self) -> bool {
        self.lanes != 0
    }

    /// Its level comes from the derivatives (`COMPUTED_LOD`, or
    /// `COMPUTED_BIAS`; `bi_instr_uses_helpers`): neither a fetch, nor a
    /// level given.
    pub fn computed(&self) -> bool {
        match self.cube {
            Some(Cube { lod }) => lod == CubeLod::Computed,
            None => !self.fetch && !self.zero,
        }
    }
}

/// A `textureSize(s, 0)` the walk met ([`Fs::tex_size`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SizeQuery {
    /// The sampler binding.
    pub sampler: u8,
    /// The node of the descriptor's first eight bytes ([`Kind::TexDesc`]
    /// at byte 0).
    pub node: u16,
    /// The size's lanes (the join's values; the third undefined for a 2D
    /// texture).
    pub out: [Val; 3],
    /// The descriptor index of its sampler ([`tex_handle`]), set by
    /// [`layout`].
    pub index: u8,
    /// The colour reads a lane of it ([`layout`]; Mesa's DCE takes the
    /// query, its `if` and its loads when none is read).
    pub live: bool,
}

impl SizeQuery {
    const NONE: SizeQuery = SizeQuery { sampler: 0, node: 0, out: [Val::Undef; 3], index: 0, live: false };
}

/// The size queries one fragment shader may make (texmore_46 makes two).
pub const MAX_SIZE_QUERIES: usize = 8;

/// The look-ups one fragment shader may make (femtovg's filter_image makes
/// 49: one, then two in each of 24 unrolled passes,
/// fvg_filter_image.expect.json).
pub const MAX_LOOK_UPS: usize = 64;

/// A cube look-up's staging ([`TexLookup::cube`]). In a highp shader the
/// look-up node's sources are `build_cube_desc`'s S and T and, at an
/// explicit level, its 8.8 word ([`nir::Nir::cube_desc`],
/// [`nir::Nir::sfixed_8_8`]), fp32 nodes of the graph; in a mediump one
/// they are the direction's three lanes, fp16 ones converted at the
/// look-up (`F16_TO_F32` each, the cube ops being fp32 only) and the
/// descriptor built there ([`select`]; case41.expect.json).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cube {
    /// The level mode (pan_nir_lower_tex.c:927-946).
    pub lod: CubeLod,
}

/// A cube look-up's level ([`Cube::lod`]), as `va_lower_tex` picks it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CubeLod {
    /// `textureLod` at the constant 0 (`nir_scalar_is_zero`): `ZERO_LOD`,
    /// two staging words.
    Zero,
    /// `textureLod` at another level: `EXPLICIT`, a third staging word.
    Explicit,
    /// `textureCube` with no level, or with a bias ([`TexLookup::bias`]):
    /// the level from the derivatives, `COMPUTED_LOD` (fstex_cube.expect
    /// .json's `.cube.computed`), or `COMPUTED_BIAS` with the bias word.
    Computed,
}

/// The texture and sampler index of a shader's first sampler. Mesa
/// numbers the samplers the linked program reads, not the ones it
/// declares (sel_t_second: `u_tex1` read alone is index 0), in their
/// declaration order (texmore_2: `u_tex0` 0, `u_tex1` 1), and the gallium
/// driver binds the texture of that sampler's unit there.
pub const TEX_INDEX: u32 = 0;

/// The handle's low word of descriptor index `index` (`va_tex_handle`,
/// pan_nir_lower_tex.c:723-745): texture table 4 (`PAN_TABLE_TEXTURE`)
/// and sampler table 3 (`PAN_TABLE_SAMPLER`), each at `index`; a pushed
/// FAU word (texmore_2.expect.json's `20001800 20011801`, `MOV.i32 r2,
/// u2.w0` and `MOV.i32 r4, u2.w1`).
pub const fn tex_handle(index: u32) -> u32 {
    (4 << 27) | (index << 16) | (3 << 11) | index
}

/// The handle of [`TEX_INDEX`].
pub const TEX_HANDLE: u32 = tex_handle(TEX_INDEX);

/// The location [`FsVarying`] give gl_PointCoord: none
/// of a varying's (the selector keeps 16).
pub const POINT_COORD: u8 = u8::MAX;

/// The location [`FsVarying`] give gl_FragCoord.z of a highp shader, read
/// as a special varying (`load_frag_coord_z`, which
/// `pan_nir_lower_var_special_pan` makes `load_var_special_pan` of
/// `BI_VARYING_NAME_FRAG_Z`, pan_nir_lower_var_special.c:17-38; Valhall's
/// `LD_VAR_SPECIAL.frag_z`, bifrost_compile.c:1419-1440): none of a
/// varying's either.
pub const FRAG_Z: u8 = u8::MAX - 1;

/// The uniform dword the graph names `gl_FbWposYTransform` by, the state
/// uniform `st_nir_lower_wpos_ytransform` adds for gl_FragCoord's y
/// (st_glsl_to_nir.cpp:395-435, `STATE_FB_WPOS_Y_TRANSFORM`): past every
/// front-end dword, so it is no declared uniform's. Mesa's state slot
/// follows the declared uniforms in the default block (select.rs
/// `UboLayout::fs32`), and its words are the draw's
/// ([`super::fau::FauWord::FbWposYTransform`]).
pub const FB_WPOS_Y_TRANSFORM: u16 = 0xfff0;

/// 0.5, the pixel centre `nir_lower_wpos_ytransform` adds (`adjX`, `adjY`
/// with panfrost's integer-centre cap, nir_lower_wpos_ytransform.c:194-200).
const F32_HALF: u32 = 0x3f00_0000;

/// NIR's program order, as a key per IR instruction (`u32::MAX` for one no
/// store needs). GLSL's front end emits a constructor's component
/// assignments to a temporary before the statement that uses it
/// (ast_function.cpp `emit_inline_vector_constructor`), and glsl_to_nir
/// visits each statement's tree children first, left to right, loading a
/// variable where it is visited; so `v_color * vec4(v_uv, 1.0, 1.0)` loads
/// v_uv first (sel_v_color_uv) and `(v_c + v_d) * v_e` loads v_e after
/// the sum (sel_v_sum_times). The front end's IR is that tree order, but it
/// stores a constructor's lanes to a local where the constructor stands:
/// so an instruction's place is its statement (the first store, to a local
/// or an output, whose source tree holds it through arithmetic), then its
/// IR index.
pub fn order_keys(m: &Module<'_>, keys: &mut [u32]) {
    let n = m.instrs.len().min(MAX_VALUES).min(keys.len());
    let grafted = grafted_locals(m, n);
    /* an `if`'s condition tree is placed by the condition's own
     * instruction (the `if` stands right after it) */
    let conds = m.nodes.iter().filter_map(|nd| match *nd {
        Node::If { cond, .. } => Some((cond.val.0 as usize, cond.val)),
        _ => None,
    });
    let stores = (0..n).filter_map(|i| match m.instrs[i] {
        ir::Op::StoreLocal { local, .. } if grafted.get(local.0 as usize).is_some_and(|g| g.is_some()) => None,
        ir::Op::StoreLocal { src, .. } | ir::Op::StoreOutput { src, .. } => Some((i, src.val)),
        _ => None,
    });
    /* a `discard` is its own statement (the `demote_if` stands where its
     * `if` did, after the condition) */
    let discards = (0..n).filter_map(|i| matches!(m.instrs[i], ir::Op::Discard).then_some((i, ir::ValueId(i as u32))));
    /* a value one instruction reads twice (`dot(x, x)`, the inlined
     * `length(x)`) is a parameter GLSL's front end assigned to a temporary
     * before the call, which stays its own statement when the body reads
     * it more than once (`opt_tree_grafting` grafts a temporary with one
     * use only): the value and its tree are computed at the value's own
     * place (measured: femtovg's `length(max(d, 0.0))`, whose clamps
     * stand before `min(max(d.x, d.y), 0.0)` in
     * fvg_fill_color_nodiscard.expect.json) */
    let params = (0..n).filter_map(|i| match m.instrs[i] {
        ir::Op::Alu(a, s) => {
            let k = ir_arity(a);
            let twice = (0..k).any(|p| (p + 1..k).any(|q| s[p].val == s[q].val));
            let expr = twice && matches!(m.instrs.get(s[0].val.0 as usize), Some(ir::Op::Alu(..)));
            expr.then_some((s[0].val.0 as usize, s[0].val))
        }
        _ => None,
    });
    /* every instruction before an `if` belongs to a statement before it
     * (the front end lowers statements in order, the condition last), and
     * GLSL emits those statements before the `if`, an inlined call's body
     * among them (`strokeMask()` is computed before `scissorMask`'s `if`
     * in fvgtest_cover.expect.json, though the constructor that reads it
     * stands after): their place is at most the `if`'s */
    for nd in m.nodes {
        let Node::If { cond, .. } = *nd else { continue };
        let c = cond.val.0;
        for (v, k) in keys.iter_mut().enumerate().take((c as usize).min(n)) {
            *k = (*k).min(c << 16 | v as u32);
        }
    }
    /* a `?:` whose condition is no constant is GLSL's `conditional_tmp`:
     * ast_to_hir emits an `if` assigning it while the expression around it
     * is still being built, so it stands before that statement, a vector
     * constructor's component assignments included (`nir_opt_peephole_select`
     * then leaves its `bcsel` there). Placed by the constructor's first
     * assignment, below every key of that statement (measured:
     * intuni_ishr.expect.json, `s < 0 ? 1.0 : 0.0` the third lane, its
     * `ICMP_OR` and `CSEL` before the first lane's `U8_TO_F32`) */
    let ternaries = (0..n).filter_map(|i| match m.instrs[i] {
        ir::Op::Alu(AluOp::Bcsel, s) if !matches!(m.def(s[0].val), Some(ir::Op::ConstBool(_))) => {
            let at = (i + 1..n)
                .find(|&j| matches!(m.instrs[j], ir::Op::StoreLocal { src, .. } if src.val.0 as usize == i))?;
            let ir::Op::StoreLocal { local, write_mask, .. } = m.instrs[at] else { return None };
            if write_mask == 0xf {
                return None;
            }
            /* the constructor's first assignment: the earliest store to the
             * local with no read of it in between */
            let mut first = at;
            for j in (0..at).rev() {
                match m.instrs[j] {
                    ir::Op::LoadLocal { local: l, .. } if l == local => break,
                    ir::Op::StoreLocal { local: l, write_mask: w, .. } if l == local && w != 0xf => first = j,
                    ir::Op::StoreLocal { local: l, .. } if l == local => break,
                    _ => {}
                }
            }
            /* the constructor's first lane too (intloop_int_compare: the
             * four selects' compares and MUXes in lane order) */
            Some((first, ir::ValueId(i as u32)))
        }
        _ => None,
    });
    /* a `?:` inside a statement's arithmetic: its `if` stands before the
     * statement (the first store after it whose tree holds it, that of a
     * grafted local's assignment, not where the local is read), with
     * the tree of its condition and sides (measured: intbool_76_boolany's
     * `r = (u_b ? 0.5 : 0.0) + (any(u_bv) ? ..) + (all(u_bv) ? ..)`: the
     * three `CSEL`s and the `any` and `all` chains, then both `FADD`s) */
    let stored = |i: usize| {
        m.instrs[..n].iter().any(|op| {
            matches!(op, ir::Op::StoreLocal { src, .. } | ir::Op::StoreOutput { src, .. } if src.val.0 as usize == i)
        })
    };
    let nested = (0..n).filter(|&i| is_ternary(m, i) && !stored(i)).filter_map(|i| {
        let holds = |root: u32| {
            let mut stack = [0u32; MAX_VALUES];
            let (mut sp, mut found) = (1, false);
            stack[0] = root;
            while sp > 0 && !found {
                sp -= 1;
                let v = stack[sp] as usize;
                found = v == i;
                if let Some(ir::Op::Alu(a, srcs)) = m.instrs.get(v).filter(|_| v > i) {
                    for x in srcs.iter().take(a.arity()) {
                        if sp < stack.len() {
                            stack[sp] = x.val.0;
                            sp += 1;
                        }
                    }
                }
            }
            found
        };
        let at = (i + 1..n).find(|&j| {
            matches!(m.instrs[j], ir::Op::StoreLocal { src, .. } | ir::Op::StoreOutput { src, .. } if holds(src.val.0))
        })?;
        Some((at, ir::ValueId(i as u32)))
    });
    for (first, root) in ternaries.chain(nested) {
        let mut stack = [0u32; MAX_VALUES];
        let mut under = [false; MAX_VALUES];
        let mut sp = 1;
        stack[0] = root.0;
        while sp > 0 {
            sp -= 1;
            let v = stack[sp] as usize;
            if v >= n || under[v] {
                continue;
            }
            under[v] = true;
            keys[v] = keys[v].min(((first as u32) << 16).saturating_sub(0x8000) + v as u32);
            if let ir::Op::Alu(a, srcs) = m.instrs[v] {
                for s in srcs.iter().take(a.arity()) {
                    if sp < stack.len() {
                        stack[sp] = s.val.0;
                        sp += 1;
                    }
                }
            }
        }
    }
    for (i, root) in stores.chain(conds).chain(params).chain(discards) {
        if i >= n {
            continue;
        }
        keys[i] = keys[i].min((i as u32) << 16 | i as u32);
        /* each value is pushed once (`under`), so the stack holds them all;
         * with each, the place its key takes: its own index, or, inside a
         * grafted local's tree, the index of the read the tree was grafted
         * into (glsl_to_nir visits the grafted rvalue where the
         * dereference stood: texmore_48's `a` and `f`, `textureLod` and
         * `texelFetch` of earlier statements, are emitted after the
         * `texture` the colour's constructor names first, texmore_48.stderr
         * `%22`-`%25`) */
        let mut under = [false; MAX_VALUES];
        let mut stack = [(0u32, u32::MAX); MAX_VALUES];
        let mut sp = 1;
        stack[0] = (root.0, u32::MAX);
        while sp > 0 {
            sp -= 1;
            let (v, pos) = stack[sp];
            let v = v as usize;
            if v >= n || under[v] {
                continue;
            }
            under[v] = true;
            let at = if pos == u32::MAX { v as u32 } else { pos };
            keys[v] = keys[v].min((i as u32) << 16 | at);
            let mut child = |x: u32, pos: u32| {
                if (x as usize) < n && !under[x as usize] && sp < stack.len() {
                    stack[sp] = (x, pos);
                    sp += 1;
                }
            };
            match m.instrs[v] {
                ir::Op::Alu(a, srcs) => srcs.iter().take(a.arity()).for_each(|s| child(s.val.0, pos)),
                /* a look-up's coordinate is visited as its child */
                ir::Op::Tex { coord, .. } => child(coord.val.0, pos),
                /* and an indexed uniform read's index (builtin_45's `idx +
                 * i`, an int sum the walk keeps as a node) */
                ir::Op::LoadUniformIdx { index, .. } => child(index.val.0, pos),
                /* a grafted local's one read is its stored tree, placed at
                 * the read (an outer read's place holds for a tree grafted
                 * into a grafted tree) */
                ir::Op::LoadLocal { local, index: None, .. } => {
                    if let Some(src) = grafted.get(local.0 as usize).copied().flatten() {
                        child(src.0, if pos == u32::MAX { v as u32 } else { pos })
                    }
                }
                _ => {}
            }
        }
    }
    call_returns(m, n, keys);
    swapped_compares(m, n, &grafted, keys);
}

/// An inlined call's return expression of `?:` values is its own GLSL
/// statement (`retval = a * b`), emitted before the next call's parameter
/// assignments: so an operation of the statement at `S` whose operands
/// are all `?:` trees placed before `S` (`order_keys`'s ternaries, keyed
/// `S << 16 - 0x8000 + v`), and after which a separate statement's store
/// stands before a further `?:` of `S`, takes their place, by its index.
/// Measured: deqp_mixprec_cmpmat2, `compare_vec2(a[0], b[0]) *
/// compare_vec2(a[1], b[1])`, column 0's `fmax` before column 1's `fadd`
/// (the drm-shim's NIR); `compare_vec4`'s products of four calls in one
/// return stay after every call (deqp_mixprec_cmpv4).
fn call_returns(m: &Module<'_>, n: usize, keys: &mut [u32]) {
    for i in 0..n {
        let ir::Op::Alu(a, srcs) = m.instrs[i] else { continue };
        let s = keys[i] >> 16;
        if keys[i] == u32::MAX || keys[i] & 0xffff != i as u32 || s == 0 {
            continue;
        }
        let base = (s << 16).saturating_sub(0x8000);
        let placed = |k: u32| k >= base && k < s << 16;
        let operands = srcs
            .iter()
            .take(a.arity())
            .all(|x| matches!(m.instrs.get(x.val.0 as usize), Some(ir::Op::Alu(..))) && placed(keys[x.val.0 as usize]));
        if !operands {
            continue;
        }
        let Some(t) = (i + 1..n).find(|&t| placed(keys[t]) && is_ternary(m, t)) else { continue };
        let foreign = (i + 1..t)
            .any(|j| matches!(m.instrs[j], ir::Op::StoreLocal { .. } | ir::Op::StoreOutput { .. }) && keys[j] < base);
        if foreign {
            keys[i] = base + i as u32;
        }
    }
}

/// GLSL IR has no `>` or `<=`: ast_to_hir makes a scalar `a > b` and `a <=
/// b` the `b < a` and `b >= a` of swapped operands (ast_to_hir.cpp:1574-1581),
/// and glsl_to_nir visits a tree's operands in order, so `b`'s tree is
/// emitted before `a`'s. The IR keeps `a`'s tree first (`Flt(b, a)`,
/// `Fle(a, b)`): within the statement, the tree of NIR's first source
/// (`Flt`'s first, `Fle`'s second, `fge(b, a)`) takes the earlier places
/// and the other tree the later ones, each in its own order (measured: the
/// `isOk` of deqp_constop_ce_floor_fs, `abs(a - b) <= eps * abs(b) + eps`:
/// `FMA.f32` of the right side, then `FADD.f32` of `a - b`). A vector
/// compare is `lessThanEqual` and the like, a call whose operands are its
/// parameters' temporaries in order (deqp_varylink_swz_xx: `a - b` first).
fn swapped_compares(m: &Module<'_>, n: usize, grafted: &[Option<ir::ValueId>; MAX_LOCALS], keys: &mut [u32]) {
    for v in 0..n {
        let ir::Op::Alu(op @ (AluOp::Flt | AluOp::Fle | AluOp::Ilt | AluOp::Ile), s) = m.instrs[v] else { continue };
        if keys[v] == u32::MAX || m.comps_of(ir::ValueId(v as u32)) != 1 {
            continue;
        }
        let (first, second) = match op {
            AluOp::Flt | AluOp::Ilt => (s[0].val, s[1].val),
            _ => (s[1].val, s[0].val),
        };
        /* each tree's instructions of the statement, a grafted local's read
         * through to its tree; one in both is the first tree's */
        let mut tree = [0u8; MAX_VALUES];
        for (mark, root) in [(1u8, first), (2u8, second)] {
            let mut stack = [0u32; MAX_VALUES];
            let mut sp = 1;
            stack[0] = root.0;
            while sp > 0 {
                sp -= 1;
                let u = stack[sp] as usize;
                if u >= n || tree[u] != 0 || keys[u] >> 16 != keys[v] >> 16 || keys[u] >= keys[v] {
                    continue;
                }
                tree[u] = mark;
                let mut child = |x: u32| {
                    if sp < stack.len() {
                        stack[sp] = x;
                        sp += 1;
                    }
                };
                match m.instrs[u] {
                    ir::Op::Alu(a, srcs) => srcs.iter().take(a.arity()).for_each(|x| child(x.val.0)),
                    ir::Op::LoadLocal { local, index: None, .. } => {
                        if let Some(src) = grafted.get(local.0 as usize).copied().flatten() {
                            child(src.0)
                        }
                    }
                    _ => {}
                }
            }
        }
        /* the trees' places, in order, handed out first tree first */
        let mut places = [0u32; MAX_VALUES];
        let mut members = [0u16; MAX_VALUES];
        let mut k = 0;
        for u in (0..n).filter(|&u| tree[u] != 0) {
            places[k] = keys[u];
            members[k] = u as u16;
            k += 1;
        }
        places[..k].sort_unstable();
        members[..k].sort_unstable_by_key(|&u| (tree[u as usize], keys[u as usize]));
        for (u, place) in members[..k].iter().zip(places[..k].iter()) {
            keys[*u as usize] = *place;
        }
    }
}

/// GLSL IR's `opt_tree_grafting` (glsl/opt_tree_grafting.cpp): a local
/// assigned once and read once has its assignment's tree grafted into the
/// read, so the tree is computed where the read stands, not where it was
/// assigned (measured: fbload_int.frag's `float y = gl_FragCoord.y *
/// u_flip.x + u_flip.y`, whose `ffma` follows `int(gl_FragCoord.x)`'s
/// `u2u32` in fbload_fbload_int.stderr). The value each grafted local's
/// read stands for, by local; only in a body without control flow, and
/// not past a store to a local the tree reads (`can_graft`'s check of the
/// instructions between, :210-240).
fn grafted_locals(m: &Module<'_>, n: usize) -> [Option<ir::ValueId>; MAX_LOCALS] {
    let mut out = [None; MAX_LOCALS];
    if m.nodes.iter().any(|nd| matches!(nd, Node::If { .. } | Node::Loop { .. })) {
        return out;
    }
    for (l, o) in out.iter_mut().enumerate().take(m.locals.len()) {
        let stores =
            (0..n).filter(|&i| matches!(m.instrs[i], ir::Op::StoreLocal { local, .. } if local.0 as usize == l));
        let loads = (0..n).filter(|&i| matches!(m.instrs[i], ir::Op::LoadLocal { local, .. } if local.0 as usize == l));
        let (Some(st), 1, Some(ld), 1) = (stores.clone().next(), stores.count(), loads.clone().next(), loads.count())
        else {
            continue;
        };
        let ir::Op::StoreLocal { src, index: None, write_mask, .. } = m.instrs[st] else { continue };
        if st > ld || !matches!(m.instrs[ld], ir::Op::LoadLocal { index: None, .. }) {
            continue;
        }
        /* a whole assignment, read with the identity swizzle; or one lane
         * of a 2D array's or 3D texture's look-up (`float a =
         * textureLod(vol, p, 0.0).r`, texmore_48: an `ir_swizzle` of the
         * `ir_texture`, one rvalue, grafted as the look-up itself is) */
        /* whole: every lane of the local (its own width; the value's may be
         * the front end's blanket four, gm2_fsarith_1's `vec3 L =
         * normalize(light)` and `float d = max(..)`) */
        let comps = match m.locals.get(l).map(|d| d.ty.comps) {
            Some(c @ 1..=4) if write_mask == (1u8 << c) - 1 => c as usize,
            _ => width_of(m, src.val),
        };
        let whole = write_mask == (1u8 << comps) - 1 && (0..comps).all(|c| src.swizzle[c] as usize == c);
        let texel = write_mask.count_ones() == 1
            && matches!(
                m.instrs.get(src.val.0 as usize),
                Some(ir::Op::Tex { dim: ir::TexDim::D2Array | ir::TexDim::D3, .. })
            );
        if !whole && !texel {
            continue;
        }
        /* the locals the tree reads, none stored in between; a tree of
         * other instructions (a look-up, an indexed read) is not grafted
         * here */
        let mut reads = [false; MAX_LOCALS];
        let mut stack = [0u32; MAX_VALUES];
        let (mut sp, mut ok) = (1, true);
        stack[0] = src.val.0;
        while sp > 0 && ok {
            sp -= 1;
            match m.instrs.get(stack[sp] as usize) {
                Some(ir::Op::Alu(a, srcs)) => {
                    for x in srcs.iter().take(a.arity()) {
                        ok &= sp < stack.len();
                        if ok {
                            stack[sp] = x.val.0;
                            sp += 1;
                        }
                    }
                }
                /* a 2D array's or a 3D texture's look-up (an `ir_texture`,
                 * grafted as any rvalue: case 42's `t` and `f`, whose `tex`
                 * and `txf` stand at the colour's store in
                 * intbig_case42.stderr, after the fetch's coordinates;
                 * texmore_48's `a` and `f`, whose `txl` and `txf` follow the
                 * `texture` the constructor names first) */
                Some(ir::Op::Tex { dim: ir::TexDim::D2Array | ir::TexDim::D3, coord, .. }) => {
                    stack[sp] = coord.val.0;
                    sp += 1;
                }
                /* and a 2D look-up's (gm2_fclane_2's `vec4 c = texture2D(s1,
                 * t1)`, read once in the colour: its `TEX_SINGLE` follows
                 * that of `m`, read twice, so `m`'s handle is pushed first,
                 * FAU `20001800, 20011801`, though the schedule issues `c`'s
                 * look-up first) */
                Some(ir::Op::Tex { dim: ir::TexDim::D2, shadow: false, proj: false, offset: None, coord, .. }) => {
                    stack[sp] = coord.val.0;
                    sp += 1;
                }
                Some(ir::Op::LoadLocal { local, .. }) => match reads.get_mut(local.0 as usize) {
                    Some(r) => *r = true,
                    None => ok = false,
                },
                Some(
                    ir::Op::ConstF32(_) | ir::Op::ConstI32(_) | ir::Op::LoadUniform { .. } | ir::Op::LoadInput { .. },
                ) => {}
                _ => ok = false,
            }
        }
        if !ok {
            continue;
        }
        let clobbered = (st + 1..ld).any(
            |i| matches!(m.instrs[i], ir::Op::StoreLocal { local, .. } if reads.get(local.0 as usize) == Some(&true)),
        );
        /* a `?:` on a run-time condition between them is an `if` (GLSL's
         * `conditional_tmp`, [`order_keys`]), which ends the basic block
         * the grafting stays in (`tree_grafting_basic_block`): measured,
         * intbool_76_boolany's `r`, whose sum stands before `g`'s compares */
        let ternary = (st + 1..ld).any(|i| is_ternary(m, i));
        if !clobbered && !ternary {
            *o = Some(src.val);
        }
    }
    out
}

/// The operand of product `s` that is the `fneg` of a literal (GLSL's
/// `-2.0`, which the front end keeps as an `fneg`), when the other is not
/// a constant.
fn neg_literal(m: &Module<'_>, s: &[ir::Src; 3]) -> Option<usize> {
    let lit = |k: usize| match m.def(s[k].val) {
        Some(ir::Op::Alu(AluOp::Fneg, [c, ..])) => matches!(m.def(c.val), Some(ir::Op::ConstF32(_))),
        _ => false,
    };
    let konst = |k: usize| matches!(m.def(s[k].val), Some(ir::Op::ConstF32(_)));
    match (lit(0), lit(1)) {
        (true, false) if !konst(1) => Some(0),
        (false, true) if !konst(0) => Some(1),
        _ => None,
    }
}

/// IR instruction `i` is a `?:` whose condition is no constant: GLSL's
/// `conditional_tmp`, an `if` before the statement ([`order_keys`]).
/// `s` reads a word of a `bool` / `bvecN` uniform (`LoadUniform`), or its
/// `i2b`.
fn bool_uniform32(m: &Module<'_>, s: ir::Src) -> bool {
    let dword_off = match m.def(s.val) {
        Some(ir::Op::LoadUniform { dword_off, .. }) => dword_off,
        Some(ir::Op::Alu(AluOp::I2b, x)) => match m.def(x[0].val) {
            Some(ir::Op::LoadUniform { dword_off, .. }) => dword_off,
            _ => return false,
        },
        _ => return false,
    };
    m.uniforms.iter().any(|u| {
        *dword_off >= u.dword_off
            && *dword_off < u.dword_off + u.dwords.max(1)
            && matches!(u.gl_type as u32, GL_BOOL | GL_BOOL_VEC2 | GL_BOOL_VEC3 | GL_BOOL_VEC4)
    })
}

fn is_ternary(m: &Module<'_>, i: usize) -> bool {
    matches!(m.instrs.get(i), Some(ir::Op::Alu(AluOp::Bcsel, s)) if !matches!(m.def(s[0].val), Some(ir::Op::ConstBool(_))))
}

/// `s` is a read of a local GLSL's `opt_tree_grafting` leaves in place
/// ([`grafted_locals`]): the value it stands for is another expression
/// tree of GLSL IR, so its rules saw the two apart and NIR's see them
/// together.
fn kept_local(m: &Module<'_>, s: ir::Src) -> bool {
    match m.def(s.val) {
        Some(ir::Op::LoadLocal { local, index: None, .. }) => {
            let grafted = grafted_locals(m, m.instrs.len().min(MAX_VALUES));
            grafted.get(local.0 as usize).is_some_and(|g| g.is_none())
        }
        _ => false,
    }
}

/// IR value `id` (an ES 1.00 gl_FragCoord read) is read only by stores to
/// locals the precision table holds highp ([`Module::local_prec_of`]):
/// glmark2's `highp vec2 FragCoord = gl_FragCoord.xy` of the conditionals,
/// function and loop scenes. GLSL's lower_precision narrows no bare
/// dereference (`handle_rvalue`, lower_precision.cpp:835-843), so Mesa's
/// local takes the fp32 read. A read any other instruction takes stays the
/// mediump one ([`Fs::frag_coord16`]); a module without precision tables
/// has no highp local.
fn highp_copy(m: &Module<'_>, id: u32) -> bool {
    let mut users = 0usize;
    let mut highp = true;
    for op in m.instrs.iter() {
        let mut reads = false;
        op.for_each_src(|s| reads |= s.val.0 == id);
        if !reads {
            continue;
        }
        users += 1;
        highp &=
            matches!(op, ir::Op::StoreLocal { local, index: None, .. } if m.local_prec_of(*local) == ir::Prec::High);
    }
    users > 0 && highp
}

/// Every local the mediump ES 1.00 shader declares highp
/// ([`FsPrecision::highp_locals`]) is a copy of gl_FragCoord: each store to
/// it stores a read [`highp_copy`] gives to highp locals alone. Any other
/// highp local has no reference (the shader is mixed).
fn highp_locals_copy(m: &Module<'_>, prec: &FsPrecision) -> bool {
    let named = |h: u64| prec.highp_locals.contains(&h) && h != 0;
    m.locals.iter().enumerate().filter(|(_, d)| named(super::name_hash(d.name))).all(|(l, _)| {
        let mut stores = 0usize;
        let all = m.instrs.iter().all(|op| match *op {
            ir::Op::StoreLocal { local, src, .. } if local.0 as usize == l => {
                stores += 1;
                matches!(m.def(src.val), Some(ir::Op::LoadInput { slot: IoSlot::FragCoord, .. }))
                    && highp_copy(m, src.val.0)
            }
            _ => true,
        });
        all && stores > 0
    })
}

/// What a node of the colour is, as an operand or as an output word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Form {
    Half,
    HalfSwapped,
    Splat,
    Mk,
    Uniform,
    Const,
    /// An operation's result.
    Op,
    Neg,
    Abs,
    /// A word of the texture look-up's result.
    Tex,
    /// A word of the uniform array element read at a run-time index
    /// ([`Kind::UniformIdx`], Mesa's `LD_PKA`).
    Ubo,
}

/// Where a node stands: an operand of an operation, or an output word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Place {
    AddA,
    AddB,
    MulA,
    MulB,
    FmaA,
    FmaB,
    FmaC,
    Output,
    /// The operand of an fneg.
    NegSrc,
    AbsSrc,
}

/// The places a [`Form`] may stand in.
const PLACES: u8 = Place::AbsSrc as u8 + 1;
const _: () = assert!((Form::Ubo as u32 + 1) * PLACES as u32 <= u128::BITS);

/// A production of the vectoriser: a form in a place, one bit each
/// (`Form as u8 * PLACES + Place as u8`).
pub const fn prod(f: Form, at: Place) -> u128 {
    1 << (f as u8 * PLACES + at as u8)
}

/// The productions the references draw: a node in any other
/// place is refused ([`MaliError::FsOperandForm`]), until a reference
/// draws it.
pub const ADMITTED: u128 = {
    use Form::*;
    use Place::*;
    prod(Half, AddA)
        | prod(Half, AddB)
        | prod(Half, MulA)
        | prod(Half, MulB)
        | prod(Half, FmaA)
        | prod(Half, FmaB)
        | prod(Half, FmaC)
        | prod(Half, Output)
        | prod(HalfSwapped, AddA)
        | prod(HalfSwapped, MulA)
        | prod(HalfSwapped, FmaA)
        | prod(Splat, AddB)
        | prod(Splat, MulB)
        | prod(Splat, FmaB)
        | prod(Mk, AddB)
        | prod(Mk, FmaC)
        | prod(Mk, MulA)
        | prod(Mk, Output)
        | prod(Uniform, AddB)
        | prod(Uniform, MulB)
        | prod(Const, MulA)
        | prod(Const, MulB)
        | prod(Const, FmaB)
        | prod(Const, FmaC)
        | prod(Uniform, FmaB)
        | prod(Uniform, FmaC)
        | prod(Uniform, Output)
        | prod(Const, Output)
        | prod(Op, MulA)
        | prod(Op, Output)
        | prod(Const, AddA)
        | prod(HalfSwapped, AddB)
        /* `-a2.yx` (fsneg_swapped, fstex_cubedir_negyx) */
        | prod(HalfSwapped, NegSrc)
        | prod(Half, NegSrc)
        | prod(Op, NegSrc)
        | prod(Neg, AddB)
        | prod(Neg, FmaA)
        | prod(Neg, Output)
        | prod(Half, AbsSrc)
        | prod(Op, AbsSrc)
        | prod(Const, AddB)
        | prod(Abs, AddA)
        | prod(Abs, MulA)
        | prod(Abs, NegSrc)
        | prod(Abs, Output)
        | prod(Tex, Output)
        /* dEQP's operator colour, `vec4(res, 0.0, 0.0, 1.0) * scale + bias`
         * ([`vectorise`]'s computed lane beside a constant): the lane and
         * the constant's half fused-multiplied (deqp_constop_op_abs:
         * `U16_TO_U32` then `FMA.v2f16`) */
        | prod(Mk, FmaA)
        /* #32 (b): case 122's `0.5 * (fcols[fidx] + t)` (uniidx_i), case
         * 108 D's `fcols[fidx]` alone (uniidx_d) and case 197's `mix(v,
         * t, u_pick)`, `ffma(u_pick, t - v, v)` (vtl_fs) */
        | prod(Ubo, AddA)
        | prod(Ubo, Output)
        | prod(Tex, AddA)
        | prod(Tex, AddB)
        | prod(Op, MulB)
        | prod(Op, FmaB)
        | prod(Uniform, FmaA)
        /* case 2's `t0 * 0.5 + t1 * 0.25` (texmore_2): the fusion's
         * `ffma(t0, 0.5, fmul(t1, 0.25))` */
        | prod(Tex, MulA)
        | prod(Tex, FmaA)
        | prod(Op, FmaC)
        /* case 75's `vec4(u_k.x - 0.5, u_k.y - 1.0, 1.0 - u_k.z, 1.0)`
         * (varyconst_75_immneg): the uniform's pair first in a sum */
        | prod(Uniform, AddA)
        /* case 166's `u_p[56].x * u_p[0]` (unitype_166a): a uniform's
         * component first in a product, `fmul %2.xx, %3.xy` */
        | prod(Uniform, MulA)
        /* case 166's `r += u_w[0].x * texture2D(u_s0, c)` (unitype_166c):
         * a look-up's word second in the product and in its fusion,
         * `ffma(u_w[k].x, t, r)` */
        | prod(Tex, MulB)
        | prod(Tex, FmaB)
        /* glmark2's `Color = vec4(diffuse * Albedo.rgb, Albedo.a)` of a
         * white `Albedo` (gm2_arithconst_3): three colour lanes the link
         * merged into one, the first output word that lane in both
         * halves, `IADD.v2u16 r1, r0.h00, 0x0` */
        | prod(Splat, Output)
        /* glmark2's lit `vec3(0.1) + vec3(0.8 * d)` (gm2_fsarith_1): the
         * constant pair first in the fused product, `FMA.v2f16 r1, u8.w0,
         * r1^.h00, 0x2E660000.h11` */
        | prod(Const, FmaA)
        /* `mix(a, b, t)` of a float varying `t` (gm2_fsmisc_7): `t`
         * converted alone, first in the `ffma`, `FMA.v2f16 r0, r4.h00, r2^,
         * r0^` */
        | prod(Splat, FmaA)
        /* `mix(FILL, LINE, I)` of a computed scalar `I` (gm2_fsmisc_6): `I`
         * first in the `ffma`, `FMA.v2f16 r1, r0.h00, u2.w1, 0x38000000` */
        | prod(Op, FmaA)
        /* terrain's `mix(a, b, 1.0 - c)` of three look-ups (gm2_texswz_5):
         * a word negated in the sum, `FADD.v2f16 r0, 0x3C000000.h11,
         * r0.neg`, and the addend of the `ffma`, `FMA.v2f16 r0, r0^, r4^,
         * r2^` */
        | prod(Tex, NegSrc)
        | prod(Tex, FmaC)
        /* dEQP's `res += u_arr[k]` over a mediump vec2, vec3 or vec4 array
         * (deqp_uniarray_vec4_const_fs): the running sum first, `FADD.v2f16
         * r0, r0^, u4.w0` */
        | prod(Op, AddA)
        /* deqp_uniarray_dyn_vec4_fs's first sum of two loaded elements,
         * `FADD.v2f16 r1, r4^, r2^` */
        | prod(Ubo, AddB)
};

/// One load of the fragment shader: a 16-byte slot of the compacted
/// generic varyings (`slot`, its first `comps` components, at byte
/// `16 * slot`; [`layout`]), or gl_PointCoord (slot [`POINT_COORD`], two
/// components); `key` is its first read ([`order_keys`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FsVarying {
    pub location: u8,
    pub comps: u8,
    pub offset: u16,
    key: u32,
    /// Loaded flat (`.src_flat32`): the vertex shader computes its lanes
    /// from uniforms alone ([`super::link::FsConsts::flat`]).
    pub flat: bool,
    /// Holds lanes the program does not read: of a varying the link keeps
    /// whole ([`kept_whole`], a dot product's), loaded at the slot's width
    /// ([`layout`]).
    pub whole: bool,
}

impl FsVarying {
    pub const NONE: FsVarying = FsVarying { location: 0, comps: 0, offset: 0, key: 0, flat: false, whole: false };

    pub const fn new(location: u8, comps: u8, offset: u16, key: u32) -> FsVarying {
        FsVarying { location, comps, offset, key, flat: false, whole: false }
    }

    /// Its first read, as an order key.
    pub const fn key(&self) -> u32 {
        self.key
    }
}

/// A varying the fragment shader reads, as the link pairs it: its location,
/// the components read (`mask`) and the byte offset of the first in the
/// compacted layout, where they follow each other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FsRead {
    pub location: u8,
    pub mask: u8,
    pub offset: u16,
}
const F32_ONE: u32 = 0x3f80_0000;
const F32_MINUS_ONE: u32 = 0xbf80_0000;
const F32_NEG_ZERO: u32 = 0x8000_0000;
const F32_TWO: u32 = 0x4000_0000;
const F32_MINUS_TWO: u32 = 0xc000_0000;
/// 2.0 in both halves of an fp16 pair.
const F16X2_TWO: u32 = 0x4000_4000;

/// `nir_opt_constant_folding` of a 16-bit `fmul` or `fadd` of two constants
/// (fp32 bits): each operand rounded to fp16 first (lower_precision's
/// constants, `_mesa_float_to_half`), the operation in `float` on the
/// halves (exact for a product or sum of two halves) and the result
/// rounded to nearest even (nir_constant_expressions.py:551-556), returned
/// as the fp32 bits of the half. A subnormal, an overflow or a zero from a
/// non-zero result has no reference: refused.
fn fold_const16(a: AluOp, p: u32, q: u32) -> Result<u32, MaliError> {
    let h = |b: u32| f16_rtne(b).map(|h| f32::from_bits(super::select::f16_bits(h)));
    let (Some(p), Some(q)) = (h(p), h(q)) else { return Err(MaliError::ConstantFold) };
    let r = if a == AluOp::Fadd { p + q } else { p * q }.to_bits();
    let r = f16_rtne(r).ok_or(MaliError::ConstantFold)?;
    Ok(super::select::f16_bits(r))
}

/// Which algebraic rule an `fmul` or `fadd`'s constant source makes for a
/// 16-bit lane pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rule {
    None,
    /// The second source is the identity (1.0, or 0.0 for fadd) in both lanes.
    IdentitySecond,
    IdentityFirst,
    /// A source is -1.0 in both lanes (an fneg).
    Negate,
    /// A source of an `fmul` is 0.0 (or -0.0) in both lanes.
    Zero,
}

/// A fragment shader built on the graph.
pub struct Fs<'s> {
    pub nir: Nir<'s>,
    /// The shader's float precision: a highp one's arithmetic is fp32.
    pub(super) prec: FsPrecision,
    /// A value of the other width met ([`MaliError::MixedPrecision`],
    /// reported when no other gap is).
    pub mixed: bool,
    /// The varying lanes its fp16 operations read were converted explicitly
    /// ([`convert_inputs16`], the colour-of-booleans route).
    pub(super) inputs16: bool,
    /// Each node's IR instruction (its place in NIR's order, [`keys`]).
    origin: &'s mut [u16],
    /// Each node's pass of the unrolled loop that made it (0 outside a
    /// loop; the outer loop's for a nested one, [`walk_passes`]): the order
    /// of a loop's nodes, which share their [`Fs::origin`] ([`Fs::key`]).
    pass: &'s mut [u8],
    /// Each IR instruction's place in NIR's order.
    keys: &'s mut [u32],
    /// Each IR instruction's width (`Module::lanes_of`): a node whose
    /// instruction is a vector stands for lanes `nir_lower_alu_width`
    /// splits two by two, though they CSE'd into one node here
    /// ([`Fs::wide`]).
    ir_comps: &'s mut [u8],
    /// Each IR instruction's lanes as the walk left them ([`Fs::pair_lane`];
    /// empty until the walk ends).
    ir_vals: &'s [[Val; 4]],
    /// Bit `n % 64` of word `n / 64`: node `n` is read ([`Fs::mark_live`]).
    live: [u64; MAX_NODES / 64],
    /// The colour's lanes, once the shader stores it.
    pub(super) colour: Option<[Val; 4]>,
    /// The lanes of the outputs at locations 1.. (`layout(location = n)
    /// out vec4`, render target n), once stored; each is its own `BLEND`
    /// after output 0's (`pan_nir_lower_fs_outputs`,
    /// docs/notes/mali-fragment-outputs.md). Only in straight-line code
    /// ([`walk`]).
    pub(super) targets: [Option<[Val; 4]>; MAX_TARGETS - 1],
    /// The shader writes gl_FragColor: Mesa lowers it to all eight targets
    /// (`nir_lower_fragcolor(.., 8)`, pan_shader.c:556-561) and drops the
    /// stores past the first (`panfrost_nir_remove_fragcolor_stores`), but
    /// `outputs_written` keeps them, so `blend_pan` copies its colour into
    /// a new vector at the store (bifrost_compile.c:1683-1688, "explicit
    /// copy since BLEND inputs are precoloured"): its moves after ATEST.
    fragcolor: bool,
    /// The expressions Mesa's link moves into the program's vertex shader
    /// (motion.rs), in the order they were found.
    pub moved: [Moved; MAX_MOVED],
    pub nmoved: usize,
    /// Each varying location's first read (`u32::MAX`: not read).
    var_key: [u32; MAX_VARYINGS],
    /// Each varying location's lanes an fp16 `fmul` or `fadd` read and the
    /// walk folded away (`x * 1.0`, `x + 0.0` of a lane pair) where the
    /// identity holds on some lanes of the operation only: at the link
    /// the operation is still an fp16 one, so its operands are converted
    /// with the tree's `f2fmp`, not at the colour's store (sel_v_color_uv:
    /// `v_color * vec4(v_uv, 1.0, 1.0)` converts z,w right after x,y).
    /// An identity on every lane is folded at the link (the walk's `Fmul`
    /// and `Fadd` arm), so its operand is the colour's own lane.
    folded: [u8; MAX_VARYINGS],
    pcoord_key: u32,
    /// gl_FragCoord's first read in a highp shader ([`FRAG_Z`]).
    fz_key: u32,
    /// The input node of each varying location, of gl_PointCoord and of
    /// gl_FragCoord.z ([`FRAG_Z`]).
    inputs: [Option<u16>; MAX_VARYINGS],
    pcoord: Option<u16>,
    fz: Option<u16>,
    /// The loads, in Mesa's order.
    pub varyings: [FsVarying; MAX_VARYINGS],
    pub nvar: usize,
    pub reads: [FsRead; MAX_VARYINGS],
    pub nreads: usize,
    lane_of: [[Option<(u8, u8)>; 4]; MAX_VARYINGS],
    pub compacted: bool,
    alpha: Option<u16>,
    /// The uniform array elements read at a run-time index
    /// ([`Kind::UniformIdx`]), in the walk's order: one vec4 element
    /// (uniidx_d), or a float array's elements, one lane each
    /// (case108_uniidx). With any, the program loads the block from memory
    /// (`LD_PKA`) and the frame uploads it (`MaliShader::ubo`).
    pub ubo_idx: [u16; MAX_UBO_READS],
    pub nubo: usize,
    /// A highp shader's uniform block ([`select_highp`]'s): where Mesa's
    /// loads are, which [`uniform_vector`] vectorises by.
    ubo: UboLayout,
    /// The `b2f` nodes the walk made of a product of compare selects
    /// ([`Fs::cmp_product`]), with the leaves a chain folded on so far
    /// (Undef for a product of two leaves or of two products) and the
    /// mask of the product the chain started from.
    cmp_chain: [(Val, Val, Val); MAX_CMP_CHAIN],
    ncmp_chain: usize,
    pub prods: u128,
    /// The texture look-ups the shader makes, in the walk's order (an
    /// unrolled loop's once per pass). Each node is 16 bits wide
    /// (`TEX_SINGLE ... .16`, the lowp sampler's texel read by the mediump
    /// colour alone) until a lane of it is stored to a local: a highp
    /// variable, which GLSL's lower_precision leaves at fp32, so the
    /// look-up returns fp32 lanes (`.32`; measured:
    /// fvg_texture_copy_unclipped.expect.json's `TEX_SINGLE.slot1.skip
    /// .reserved.32.2d.computed @r0:r1:r2:r3`).
    look_ups: [TexLookup; MAX_LOOK_UPS],
    nlook: usize,
    /// The size queries the shader makes, in the walk's order
    /// ([`Fs::tex_size`]).
    sizes: [SizeQuery; MAX_SIZE_QUERIES],
    nsizes: usize,
    /// The rows of emit.rs selected ([`emit::bit`]).
    pub rows: u128,
    /// The pair nodes the colour took (its use of [`MAX_PAIRS`]).
    pub npairs: usize,
    /// gl_FragCoord's x and y once the shader reads it ([`Fs::frag_coord`]),
    /// and the node of its z and w.
    frag_xy: Option<[Val; 2]>,
    frag_zw: Option<u16>,
    /// gl_FragCoord's x and y narrowed to fp16, the mediump read of an ES
    /// 1.00 shader ([`Fs::frag_coord16`]).
    frag16: Option<[Val; 2]>,
    /// The mediump ES 1.00 shader's gl_FragCoord read is taken whole by a
    /// highp local (glmark2's `highp vec2 FragCoord = gl_FragCoord.xy`,
    /// [`highp_copy`]): the read is the fp32 lanes ([`Fs::frag_coord`]), an
    /// island of fp32 as a mediump ES 3.00 shader's ([`Fs::fc32`];
    /// docs/notes/mali-fragcoord-fp16.md).
    fc_copy: bool,
    /// Each local's rank by its first load or store in the walk, and the
    /// colour's by its first store (`u32::MAX`: none yet): the order
    /// `nir_lower_vars_to_ssa` registers the variables in
    /// (`register_variable_uses`, nir_lower_vars_to_ssa.c:245), whose
    /// phis the phi builder then inserts each at its block's start
    /// (nir_phi_builder.c:274-296), so a join's phis stand in the reverse
    /// of this order ([`walk_if`]).
    rank: [u32; MAX_LOCALS],
    colour_rank: u32,
    next_rank: u32,
    /// The `demote_if`s the walk made ([`Fs::discard`]): the program's
    /// `can_discard` (`MaliShader::can_discard`).
    pub discards: u8,
    /// The loops being unrolled around the nodes being walked
    /// ([`walk_loop`]).
    loops: u8,
    /// The run-time `break` of the innermost loop being unrolled, by its
    /// condition's IR value (`u32::MAX`: none yet): `complex_unroll` takes
    /// one ([`walk_guard`]).
    guard: u32,
    /// The innermost loop Mesa keeps around the nodes being walked
    /// ([`walk_kept_loop`]): its index in `nir.loops`.
    kept: Option<u8>,
    /// That loop's body (its nodes' address and count): the run whose
    /// `if (c) { step; continue; }` [`walk_nodes`] reshapes.
    kept_body: (usize, usize),
    /// That loop's pass counter, until its head test takes it
    /// ([`LoopCap`]).
    cap: Option<LoopCap>,
    /// The `break`s of the kept loops walked so far ([`BreakEdge`]).
    breaks: [BreakEdge; MAX_BREAKS],
    nbreaks: usize,
    /// The locals the module has (`Module::locals`, at most [`MAX_LOCALS`]):
    /// the entries of every copy of the locals' lanes ([`Fs::pool`]).
    nl: usize,
    /// The copies of the locals' lanes the walk keeps, `nl` entries each, in
    /// the scratch arena rather than on the stack (the walk nests a frame per
    /// `if` and per guarded pass, and a copy of every local in each would put
    /// [`MAX_LOCALS`] lanes on the stack per level; docs/notes/
    /// mali-local-arrays.md section 7): the first [`MAX_BREAKS`] are the
    /// `break`s' ([`BreakEdge`]), the rest a stack of the `if`s' saved and
    /// then-side lanes ([`walk_if`], [`walk_if16`]) from `ptop` up.
    pool: &'s mut [[Val; 4]],
    ptop: usize,
    /// Each local's place in `nir_lower_vars_to_ssa`'s registration
    /// ([`nir::registration_order`]): the order of a loop header's phis.
    reg: [u16; MAX_LOCALS],
}

/// The `break`s a kept loop may take ([`BreakEdge`]).
pub const MAX_BREAKS: usize = 8;

/// The hidden pass counter of a kept loop ([`walk_kept_loop`]): its header
/// phi, the passes left, `limit` from the pre-header ([`loop_cap_limit`]).
/// The head test subtracts 1 and ORs `count < 0` into its `break`
/// ([`walk_nodes`]).
#[derive(Clone, Copy)]
struct LoopCap {
    phi: u16,
    /// The phi's value at the latch: the head's `count - 1`, once made.
    next: Val,
}

/// A `break` of a kept loop: the block whose jump enters the loop's exit,
/// and the locals and the colour as they stand there (the exit's phis
/// take one source from each, in the order the jumps were emitted).
/// The locals' lanes of break `k` are [`Fs::pool`]'s table `k`.
#[derive(Clone, Copy)]
struct BreakEdge {
    lp: u8,
    block: u8,
    /// Its table in [`Fs::pool`].
    slot: u8,
    colour: Option<[Val; 4]>,
}

impl BreakEdge {
    const NONE: BreakEdge = BreakEdge { lp: 0, block: 0, slot: 0, colour: None };
}

/// The `if`s' tables [`Fs::pool`] holds past the `break`s': a saved and a
/// then-side table per nesting level of [`walk_if`] and [`walk_if16`]. An
/// `if` region is at most [`nir::MAX_IFS`] deep; a deeper nesting of
/// flattened mediump `if`s is [`MaliError::BlockTableFull`].
const POOL_IF_TABLES: usize = 2 * (nir::MAX_IFS + 2);

impl<'s> Fs<'s> {
    /// Table `t` of the pool.
    fn table(&self, t: usize) -> &[[Val; 4]] {
        &self.pool[t * self.nl..(t + 1) * self.nl]
    }

    fn table_mut(&mut self, t: usize) -> &mut [[Val; 4]] {
        let nl = self.nl;
        &mut self.pool[t * nl..(t + 1) * nl]
    }

    /// `locals` copied into the pool's next free table; its index.
    fn push_locals(&mut self, locals: &[[Val; 4]; MAX_LOCALS]) -> Result<usize, MaliError> {
        let t = self.ptop;
        if (t + 1) * self.nl > self.pool.len() {
            return Err(MaliError::BlockTableFull);
        }
        let nl = self.nl;
        self.table_mut(t).copy_from_slice(&locals[..nl]);
        self.ptop += 1;
        Ok(t)
    }

    /// Table `t` copied back into `locals`.
    fn load_locals(&self, t: usize, locals: &mut [[Val; 4]; MAX_LOCALS]) {
        locals[..self.nl].copy_from_slice(self.table(t));
    }
}

impl<'s> Fs<'s> {
    /// The render targets the program writes, bit i = target i: the colour
    /// (target 0) and each further target stored (`MaliShader::colour_outputs`).
    pub fn colour_outputs(&self) -> u32 {
        targets(self).fold(u32::from(self.colour.is_some()), |m, (rt, _)| m | 1 << rt)
    }

    fn key(&self, v: Val) -> u32 {
        /* an unrolled loop's nodes stand at the loop's last instruction
         * ([`walk_loop`]), pass after pass, as Mesa's unrolled body
         * (deqp_uniarray_loop_vec4_fs: each pass's two `FADD.v2f16`, xy then
         * zw, before the next pass's) */
        match v {
            Val::Lane(n, _) | Val::Pair(n, _) => {
                self.keys[self.origin[n as usize] as usize].saturating_add(u32::from(self.pass[n as usize]))
            }
            _ => u32::MAX,
        }
    }

    /// Node `n` is a dot product's (its IR instruction, the one
    /// [`Self::alu32`] lowered into the `fmul` and `ffma` chain): what
    /// Mesa's linker still sees as one `fdot` (motion.rs).
    pub(super) fn is_dot_node(&self, m: &Module<'_>, n: u16) -> bool {
        matches!(
            m.def(ir::ValueId(self.origin[n as usize] as u32)),
            Some(ir::Op::Alu(AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4, _))
        )
    }

    /// Node `n` is one lane of a vector operation: its IR instruction made
    /// another node of the same operation (the lanes [`Self::alu32`] wrote
    /// out; the IR's width table is no answer, a scalar conversion has no
    /// width there). Mesa's linker runs before `nir_lower_alu_width` and
    /// sees the vector, which is unmovable (nir_opt_varyings.c:3845-3848,
    /// motion.rs). Lanes CSE'd into one node read as a scalar.
    pub(super) fn is_vector_node(&self, n: u16) -> bool {
        let (at, kind) = (self.origin[n as usize], self.nir.nodes[n as usize].kind);
        (0..self.nir.n).any(|k| k != n as usize && self.origin[k] == at && self.nir.nodes[k].kind == kind)
    }

    fn is_const(&self, v: Val, bits: &[u32]) -> bool {
        matches!(v, Val::Const(b) if bits.contains(&b))
    }

    fn alu_of(&self, v: Val) -> Option<(Alu, [Val; 3])> {
        match v {
            Val::Lane(n, _) => match self.nir.nodes[n as usize] {
                NirNode { kind: Kind::Alu(a), src, width: 1, .. } => Some((a, src)),
                _ => None,
            },
            _ => None,
        }
    }

    /// A scalar 16-bit node, CSE'd (`nir_opt_cse`), made by IR instruction
    /// `at`.
    fn node(&mut self, op: Alu, src: [Val; 3], at: usize) -> Result<Val, MaliError> {
        let nd = NirNode { kind: Kind::Alu(op), src, bits: 16, width: 1 };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let id = self.nir.push_node(nd)?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// Operand `v` of an fp16 operation the walk folds away: a varying's
    /// lane is marked [`Fs::folded`].
    fn fold(&mut self, v: Val) -> Val {
        if let Some((loc, c)) = self.input_lane(v) {
            if let Some(m) = self.folded.get_mut(loc as usize) {
                *m |= 1 << (c & 3);
            }
        }
        v
    }

    /// One lane of `fmul` or `fadd` under the pair's `rule`. `pair`: `None`
    /// for an operation of one lane; for a lane of the pair walk, whether
    /// the pair's operand is an fneg in each of its lanes, so that the
    /// rules on `fneg` match the pair's vector source (a `vec2(fneg(a),
    /// 0.0)` source is no `fneg`: deqp_constop_op_minus).
    fn op(&mut self, a: AluOp, x: Val, y: Val, at: usize, rule: Rule, pair: Option<bool>) -> Result<Val, MaliError> {
        if x == Val::Undef || y == Val::Undef {
            return Err(MaliError::ColourForm);
        }
        let (cx, cy) = (matches!(x, Val::Const(_)), matches!(y, Val::Const(_)));
        let neg = |s: &Self, v: Val| pair != Some(false) && matches!(s.alu_of(v), Some((Alu::Fneg, _)));
        match (a, rule) {
            (AluOp::Fmul | AluOp::Fadd, Rule::IdentitySecond) => Ok(x),
            (AluOp::Fmul | AluOp::Fadd, Rule::IdentityFirst) => Ok(y),
            /* `fmul(a, 0.0)` is 0.0 (nir_opt_algebraic.py's inexact rule, in
             * force without `precise`): case 122's `t.w * 0.0` (uniidx_i,
             * no FMA in Mesa's program). Like the others it holds for a
             * lane pair whose constant is 0.0 in both lanes: case 166's
             * `u_p[58].y * vec4(1.0, 0.0, ..)` keeps its x and y lanes as
             * `ffma %6.yy, (1.0, 0.0), %4` (unitype_166a.stderr) */
            (AluOp::Fmul, Rule::Zero) => Ok(Val::Const(0)),
            /* `fmul(b2f(a), b)` is `bcsel(a, b, 0.0)` (nir_opt_algebraic.py:1695;
             * gm2_fsmisc_8's `step(0.5, x) * y`, `MUX.v2i16 r0, 0x0, r2^, r0^`) */
            (AluOp::Fmul, _) if self.b2f16(x).is_some() || self.b2f16(y).is_some() => {
                let (m, b) = match self.b2f16(x) {
                    Some(m) => (m, y),
                    None => (self.b2f16(y).ok_or(MaliError::Defect(Defect::IrIndex))?, x),
                };
                if matches!(b, Val::Const(_)) || self.b2f16(b).is_some() {
                    return Err(MaliError::IntValue);
                }
                self.node(Alu::Bcsel, [m, b, Val::Const(0)], at)
            }
            /* two constants in a pair with a computed lane: the pair is
             * one 16-bit vec2 operation, not all constant, so constant
             * folding leaves it and the lane stays an ALU lane
             * (deqp_constop_op_minus: `FMA.v2f16 r0, r0^, 0x38000000.h11,
             * 0x38000000.h11`, the y lane `0.0 * 0.5 + 0.5`) */
            (AluOp::Fmul, _) if cx && cy && pair.is_some() => self.node(Alu::Fmul, [x, y, Val::Undef], at),
            /* two constants alone: the front end folds them (a mediump
             * `normalize` of a constant, which Mesa folds and a lane-wise
             * product would not: glmark2's bump d442) */
            (AluOp::Fmul, _) if cx && cy => Err(MaliError::ConstantFold),
            /* `* -1.0` is an fneg (nir_opt_algebraic.py:296) */
            (AluOp::Fmul, Rule::Negate) => self.neg(if cx { y } else { x }, at),
            /* `fmul(fneg(a), b)` is `fneg(fmul(a, b))` (nir_opt_algebraic.py:2387) */
            (AluOp::Fmul, _) if neg(self, x) && neg(self, y) => Err(MaliError::ArithConstant),
            (AluOp::Fmul, _) if neg(self, x) || neg(self, y) => {
                let (n, other) = if neg(self, x) { (x, y) } else { (y, x) };
                let Some((_, [inner, ..])) = self.alu_of(n) else { return Err(MaliError::Defect(Defect::IrIndex)) };
                let (a, b) = if n == x { (inner, other) } else { (other, inner) };
                let m = self.node(Alu::Fmul, [a, b, Val::Undef], at)?;
                self.neg(m, at)
            }
            /* `fmul(#a, fmul(b, #c))` is `fmul(fmul(a, c), b)`
             * (nir_opt_algebraic.py:2406), the constants folded and the
             * product run through the rules of a constant: case 70's
             * unrolled `(acc * 2.0) * 0.5` is `acc` (:292, :161-167;
             * fsarith_70_loopcarry.expect.json, docs/notes/mali-constant-products.md).
             * Mesa keeps both multiplies where the folded constant is not
             * exact in fp16 (deqp_constop_op_degrees: `x * 57.28125` then
             * `* 0.04 + 0.5`, two `FMA.v2f16`): measured on these two only,
             * the cause not established (docs/notes/mali-deqp-constop.md
             * 10.5), so the product of two constants exact in fp16 is the
             * criterion */
            (AluOp::Fmul, _)
                if self.const_product(x, y).is_some_and(|(_, c1, c2)| {
                    f16_exact((f32::from_bits(c1) * f32::from_bits(c2)).to_bits()).is_some()
                }) =>
            {
                let (b, c1, c2) = self.const_product(x, y).ok_or(MaliError::Defect(Defect::IrIndex))?;
                let r = (f32::from_bits(c1) * f32::from_bits(c2)).to_bits();
                match r {
                    F32_ONE => Ok(b),
                    F32_MINUS_ONE => self.neg(b, at),
                    0 | F32_NEG_ZERO => Ok(Val::Const(0)),
                    _ => self.node(Alu::Fmul, [b, Val::Const(r), Val::Undef], at),
                }
            }
            /* `fmul(fmul(a, b), #c)` of no other constant is `fmul(fmul(a,
             * c), b)` (nir_opt_algebraic.py:2394), here of the narrowed
             * position's two lanes alone (an ES 1.00 mediump shader's
             * `gl_FragCoord.x * gl_FragCoord.y * 0.0001`: gm2_fcoord16_2
             * .expect.json, `FMA.v2f16 r0, r0^.h00, u3.w0, 0x0.neg` then
             * `FMA.v2f16 r0, r0^.h00, r1^.h00, 0x0.neg`). The walk cannot
             * see the inner product's other readers yet (the rule's
             * `is_used_once`): no reference reads it twice */
            (AluOp::Fmul, _) if self.position_product(x, y).is_some() => {
                let (p, q, c) = self.position_product(x, y).ok_or(MaliError::Defect(Defect::IrIndex))?;
                let pc = self.node(Alu::Fmul, [p, c, Val::Undef], at)?;
                self.node(Alu::Fmul, [pc, q, Val::Undef], at)
            }
            (AluOp::Fmul, _) => self.node(Alu::Fmul, [x, y, Val::Undef], at),
            (AluOp::Fadd, _) if cx && cy && pair.is_some() => self.node(Alu::Fadd, [x, y, Val::Undef], at),
            (AluOp::Fadd, _) if cx && cy => Err(MaliError::ConstantFold),
            /* `fadd(fmul(a, b), fmul(a, c))` is `fmul(a, fadd(b, c))`
             * (nir_opt_algebraic.py's factoring), the constants' sum
             * folded and `fmul(a, 1.0)` `a`: cflow_68_armselect's `v * 0.5
             * + w * 0.5` once both arms' selects are one (no `FMA` in
             * Mesa's program). A sum of other constants has no reference */
            (AluOp::Fadd, _) if self.factor(x, y).is_some() => {
                let (a, b, c) = self.factor(x, y).ok_or(MaliError::Defect(Defect::IrIndex))?;
                match f32::from_bits(b) + f32::from_bits(c) {
                    1.0 => Ok(a),
                    _ => Err(MaliError::AlgebraicForm),
                }
            }
            /* `fadd(a, a)` is `fmul(a, 2.0)` (nir_opt_algebraic.py:274): an
             * unrolled `acc += u` at its second pass (cflow_71_deadloop's
             * `%3 = fmul .., (3.0, 2.0)`, whose `y` lane is `2 * u_k.y`) */
            (AluOp::Fadd, _) if x == y => self.node(Alu::Fmul, [x, Val::Const(F32_TWO), Val::Undef], at),
            (AluOp::Fadd, _) => self.node(Alu::Fadd, [x, y, Val::Undef], at),
            _ => Err(MaliError::Arithmetic),
        }
    }

    /// `v` is the 16-bit `b2f` of a mediump compare's mask: Some(the mask).
    fn b2f16(&self, v: Val) -> Option<Val> {
        let Val::Lane(n, _) = v else { return None };
        match self.nir.nodes[n as usize] {
            NirNode { kind: Kind::Alu(Alu::B2f), bits: 16, src: [m, ..], .. } => Some(m),
            _ => None,
        }
    }

    /// `x` and `y` are products of one 16-bit value `a` with the constants
    /// `b` and `c` ([`Fs::op`]'s factoring).
    fn factor(&self, x: Val, y: Val) -> Option<(Val, u32, u32)> {
        let prod = |v: Val| match self.alu_of(v) {
            Some((Alu::Fmul, [a, Val::Const(k), _])) | Some((Alu::Fmul, [Val::Const(k), a, _])) => Some((a, k)),
            _ => None,
        };
        let ((a, b), (a2, c)) = (prod(x)?, prod(y)?);
        (a == a2 && !matches!(a, Val::Const(_))).then_some((a, b, c))
    }

    /// Lane `v` is a node of a vector IR operation (its lanes CSE'd into
    /// one): Mesa pairs it as an operation of two lanes reading its
    /// operands' halves (gm2_fsarith_1's `vec3(0.1) + vec3(0.8 * d) + ..`,
    /// `FMA.v2f16 r1, u8.w0, r1^.h00, 0x2E660000.h11`), where a scalar's is
    /// one lane, packed (gm2_fsmisc_4's `vec4(smoothstep(..))`).
    fn wide(&self, v: Val) -> bool {
        match v {
            Val::Lane(n, _) => self.ir_comps.get(self.origin[n as usize] as usize).is_some_and(|&c| c > 1),
            _ => false,
        }
    }

    /// Lane `v` is lane `i` of a vector IR operation that `nir_opt_vectorize`
    /// pairs (an `fadd`, `fmul` or `ffma` of 16-bit lanes): Some((the
    /// operation's lanes, `i`, its width)). A lane the walk's last pass of
    /// a loop no longer holds is none.
    fn pair_lane(&self, v: Val, hint: Option<usize>) -> Option<([Val; 4], usize, usize)> {
        let Val::Lane(n, _) = v else { return None };
        if self.nir.nodes[n as usize].bits != 16 || !self.wide(v) {
            return None;
        }
        if !matches!(self.alu_of(v), Some((Alu::Fadd | Alu::Fmul | Alu::Ffma, _))) {
            return None;
        }
        let o = self.origin[n as usize] as usize;
        let mut w = usize::from(*self.ir_comps.get(o)?).min(4);
        let lanes = *self.ir_vals.get(o)?;
        /* the lanes past the last one read are gone (`nir_opt_shrink_vectors`:
         * `vec3(nm * vec4(n, 1.0))`'s `w`, so `z` is an operation of its own) */
        while w > 1 && !self.read(lanes[w - 1]) {
            w -= 1;
        }
        /* a value in several lanes (CSE'd) is the lane the reader names, when
         * it names one ([`Pairs::hint`]): the colour's `z` of `vec3(0.1) +
         * ..` is the operation of one lane, not a half of the `(x, y)` pair */
        let i = match hint {
            Some(h) if h < w && lanes[h] == v => h,
            _ => lanes[..w].iter().position(|&x| x == v)?,
        };
        Some((lanes, i, w))
    }

    /// `fmul(x, y)` is a product of the narrowed position's two lanes
    /// ([`Fs::frag16_lane`]) and a constant, in either order: (the first
    /// lane, the second, the constant).
    fn position_product(&self, x: Val, y: Val) -> Option<(Val, Val, Val)> {
        let (o, c) = match (x, y) {
            (o, Val::Const(_)) => (o, y),
            (Val::Const(_), o) => (o, x),
            _ => return None,
        };
        let (Alu::Fmul, [p, q, _]) = self.alu_of(o)? else { return None };
        (self.frag16_lane(p).is_some() && self.frag16_lane(q).is_some()).then_some((p, q, c))
    }

    /// Lane `v` has one reader among the live nodes and the colour's lanes
    /// (`is_used_once`: the late `ffma` fusion takes only such a product,
    /// nir_opt_algebraic.py `('fmul(is_used_once)', a, b)`; gm2_fsmisc_2's
    /// `3.0 * x.x`, read by `fract`'s floor and its sum, stays a product).
    fn used_once(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        let mut uses = self.colour.map_or(0, |c| usize::from(c.contains(&v)));
        for (k, nd) in self.nir.nodes[..self.nir.n].iter().enumerate() {
            if self.live[k / 64] >> (k % 64) & 1 == 0 {
                continue;
            }
            if nd.src[..nd.arity().min(3)].iter().any(|s| matches!(*s, Val::Lane(x, _) if x == n)) {
                uses += 1;
            }
        }
        uses == 1
    }

    /// Lane `v` is live ([`Fs::live`]).
    fn read(&self, v: Val) -> bool {
        match v {
            Val::Lane(n, _) => self.live[n as usize / 64] >> (n % 64) & 1 != 0,
            _ => true,
        }
    }

    /// The nodes the program reads ([`Fs::live`]): the colour's, the other
    /// targets', the discards and what they read, the way `nir_opt_dce`
    /// leaves them.
    fn mark_live(&mut self) {
        let mut stack = [0u16; MAX_NODES];
        let mut n = 0;
        let push = |live: &mut [u64], stack: &mut [u16], n: &mut usize, v: Val| {
            if let Val::Lane(x, _) | Val::Pair(x, _) = v {
                if live[x as usize / 64] >> (x % 64) & 1 == 0 && *n < stack.len() {
                    live[x as usize / 64] |= 1 << (x % 64);
                    stack[*n] = x;
                    *n += 1;
                }
            }
        };
        let roots = self.colour.into_iter().chain(self.targets.iter().flatten().copied()).flatten();
        for v in roots {
            push(&mut self.live, &mut stack, &mut n, v);
        }
        for k in 0..self.nir.n {
            if self.nir.nodes[k].kind == Kind::Discard {
                push(&mut self.live, &mut stack, &mut n, Val::Lane(k as u16, 0));
            }
        }
        while n > 0 {
            n -= 1;
            let nd = self.nir.nodes[stack[n] as usize];
            for &s in &nd.src[..nd.arity().min(3)] {
                push(&mut self.live, &mut stack, &mut n, s);
            }
        }
    }

    /// `a` and `b` are lanes of one vector IR operation ([`Fs::wide`]).
    fn same_op(&self, a: Val, b: Val) -> bool {
        match (a, b) {
            (Val::Lane(x, _), Val::Lane(y, _)) => self.wide(a) && self.origin[x as usize] == self.origin[y as usize],
            _ => false,
        }
    }

    /// `ffma(a, 1.0, acc)` of a mediump lane, `acc` a chain of `ffma`s down
    /// to an `fmul` (the front end's `M * vec4(v, 1.0)`: `fmul(c1, v.y)`,
    /// `ffma(c0, v.x, ..)`, `ffma(c2, v.z, ..)`, then `ffma(c3, 1.0, ..)`):
    /// `fadd(acc, a)`, which `reassoc_fma_optimizations` (nir_opt_algebraic.py:
    /// 4353-4360) makes `fadd(c2 v.z, fadd(c0 v.x, fadd(c1 v.y, a)))`, then
    /// fused: `a` the addend of the first product (gm2_fsarith_1: `FMA.v2f16
    /// r2, u2.w0, r0.h11, u2.w1`, `.. u6.w0, r0.h00, r2^`, `.. u7.w0, r1.h00,
    /// r2^`). None for any other `acc`.
    fn reassoc_last(&mut self, a: Val, acc: Val, at: usize) -> Result<Option<Val>, MaliError> {
        /* two terms (`mat3 * vec3(p, 1.0)`) are `ffma(m1, y, ffma(m0, x,
         * m2))` at fp32 ([`Fs::alu32_lanes`]); at fp16 no reference draws
         * them, nor four */
        let mut terms = [(Val::Undef, Val::Undef); 4];
        let mut n = 0;
        let mut v = acc;
        loop {
            match self.alu_of(v) {
                Some((Alu::Ffma, [p, q, r])) if n < 3 => {
                    terms[n] = (p, q);
                    n += 1;
                    v = r;
                }
                Some((Alu::Fmul, [p, q, _])) => {
                    terms[n] = (p, q);
                    n += 1;
                    break;
                }
                _ => return Ok(None),
            }
        }
        if n != 3 {
            return Ok(None);
        }
        let mut m = a;
        for &(p, q) in terms[..n].iter().rev() {
            m = self.node(Alu::Ffma, [p, q, m], at)?;
        }
        Ok(Some(m))
    }

    /// `v` is the 16-bit `fmul(flog2(a), #b)` (either order): Some((a, b)),
    /// the operands of the `fpow` an `exp2` of it is ([`walk_nodes`]).
    fn pow_of(&self, v: Val) -> Option<(Val, u32)> {
        let Val::Lane(n, _) = v else { return None };
        if self.nir.nodes[n as usize].bits != 16 {
            return None;
        }
        let (Alu::Fmul, [p, q, _]) = self.alu_of(v)? else { return None };
        let (l, b) = match (p, q) {
            (l, Val::Const(b)) | (Val::Const(b), l) => (l, b),
            _ => return None,
        };
        match self.alu_of(l)? {
            (Alu::Flog2, [a, ..]) => Some((a, b)),
            _ => None,
        }
    }

    /// `x * y` is `smoothstep`'s last product as the front end writes it
    /// (builtin_call.rs): `x` the 16-bit `t * t` and `y` `ffma(-2.0, t,
    /// 3.0)` of the same `t`, a saturate. Some(`t`).
    fn smoothstep_tail(&self, x: Val, y: Val) -> Option<Val> {
        const F32_THREE: u32 = 0x4040_0000;
        let Val::Lane(n, _) = x else { return None };
        if self.nir.nodes[n as usize].bits != 16 {
            return None;
        }
        let (Alu::Fmul, [t, t2, _]) = self.alu_of(x)? else { return None };
        let (Alu::Ffma, [Val::Const(F32_MINUS_TWO), t3, Val::Const(F32_THREE)]) = self.alu_of(y)? else { return None };
        let sat = matches!(self.alu_of(t), Some((Alu::Fclamp(clamp::ZERO_ONE), _)));
        (t == t2 && t == t3 && sat).then_some(t)
    }

    /// One of `x` and `y` is a constant `c2` and the other a 16-bit product
    /// of a non-constant `b` with a constant `c1`: `(b, c1, c2)`
    /// ([`Fs::op`]'s reassociation).
    fn const_product(&self, x: Val, y: Val) -> Option<(Val, u32, u32)> {
        let (c2, p) = match (x, y) {
            (Val::Const(c), p) | (p, Val::Const(c)) => (c, p),
            _ => return None,
        };
        let Val::Lane(n, _) = p else { return None };
        if self.nir.nodes[n as usize].bits != 16 {
            return None;
        }
        match self.alu_of(p)? {
            (Alu::Fmul, [b, Val::Const(c1), _]) | (Alu::Fmul, [Val::Const(c1), b, _])
                if !matches!(b, Val::Const(_)) =>
            {
                Some((b, c1, c2))
            }
            _ => None,
        }
    }

    /// One lane of `fneg`: a constant's is folded, `fneg(fneg(a))` is
    /// `fcanonicalize(a)` (nir_opt_algebraic.py:230), which is `a`
    /// (:161-167).
    fn neg(&mut self, x: Val, at: usize) -> Result<Val, MaliError> {
        match x {
            Val::Undef => Err(MaliError::ColourForm),
            Val::Const(b) => Ok(Val::Const(b ^ F32_NEG_ZERO)),
            _ if matches!(self.alu_of(x), Some((Alu::Fneg, _))) => {
                let Some((_, [inner, ..])) = self.alu_of(x) else { return Err(MaliError::Defect(Defect::IrIndex)) };
                Ok(inner)
            }
            _ => self.node(Alu::Fneg, [x, Val::Undef, Val::Undef], at),
        }
    }

    /// One lane of `fabs`: a constant's is folded, `fabs(fneg(a))` and
    /// `fabs(fabs(a))` are `fabs(a)` (nir_opt_algebraic.py). `fabs` of a
    /// derivative (`fwidth`'s form) has no reference: Mesa leaves the
    /// derivative's sign fix out when every use ignores the sign bit
    /// (`bi_emit_derivative`'s `nir_def_all_uses_ignore_sign_bit`), a shape
    /// no reference draws ([`MaliError::FsOperandForm`];
    /// docs/notes/mali-derivatives.md 6.1).
    fn abs(&mut self, x: Val, at: usize) -> Result<Val, MaliError> {
        match (x, self.alu_of(x)) {
            (Val::Undef, _) => Err(MaliError::ColourForm),
            (Val::Const(b), _) => Ok(Val::Const(b & !F32_NEG_ZERO)),
            (_, Some((Alu::Fneg, [y, ..]))) => self.abs(y, at),
            (_, Some((Alu::Fabs, _))) => Ok(x),
            (_, Some((Alu::Fddx | Alu::Fddy, _))) => Err(MaliError::FsOperandForm),
            _ => self.node(Alu::Fabs, [x, Val::Undef, Val::Undef], at),
        }
    }

    /// A phi's source `v` as the side whose last block is `block` leaves
    /// it: a 16-bit lane of a vector (a texel, `c.r = texture2D(..).r`) is
    /// a `mov` of the lane at the end of the side, which
    /// `nir_lower_phis_to_scalar` makes for each scalar phi it splits off
    /// (nir_lower_phis_to_scalar.c: `nir_mov_alu` of the source with the
    /// lane's swizzle, inserted before the block's jump) and copy
    /// propagation keeps, a phi source taking no swizzle ([`Alu::Mov`];
    /// cflow_148_texbranch.stderr's `%8 = mov %7.x` on each side of the
    /// first `if`). A scalar value or a constant is the source itself
    /// (`nir_opt_copy_propagate`; the constant a move `bi_out_of_ssa`
    /// makes, s5_phi_const). An fp32 lane of a vector has no reference.
    fn phi_src(&mut self, v: Val, block: u8, at: usize) -> Result<Val, MaliError> {
        let Val::Lane(n, _) = v else { return Ok(v) };
        let nd = self.nir.nodes[n as usize];
        let vector = nd.kind == Kind::Tex || nd.width > 1;
        if !vector || nd.bits != 16 {
            return Ok(v);
        }
        let saved = self.nir.cur;
        self.nir.cur = block;
        let mov = NirNode { kind: Kind::Alu(Alu::Mov), src: [v, Val::Undef, Val::Undef], bits: 16, width: 1 };
        let r = self.nir.push_node(mov);
        self.nir.cur = saved;
        let id = r?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// The join's value of a lane the sides of an `if` left as `a` (then)
    /// and `b` (else): one of them when they agree or the other is
    /// unwritten (`nir_opt_remove_phis`, `nir_opt_undef`), else a phi node
    /// in the join block ([`Kind::Phi`]).
    fn phi(&mut self, a: Val, b: Val, at: usize) -> Result<Val, MaliError> {
        if a == b || b == Val::Undef {
            return Ok(a);
        }
        if a == Val::Undef {
            return Ok(b);
        }
        let bits = match (a, b) {
            (Val::Lane(n, _), _) | (_, Val::Lane(n, _)) => self.nir.nodes[n as usize].bits,
            _ => 32,
        };
        let id = self.nir.push_node(NirNode { kind: Kind::Phi, src: [a, b, Val::Undef], bits, width: 1 })?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// `if (c) discard;` with no else, IR instruction `at` the discard: the
    /// `demote_if(c)` GLSL's front end makes of it (`nir_discard_if` under
    /// `discard_is_demote`, glsl_to_nir.cpp:812-825, bifrost_compile.h:151),
    /// a [`Kind::Discard`] node in the current block, which the selection
    /// makes `DISCARD.b32 c` and the BIR passes fuse with the compare into
    /// one `DISCARD.f32` (opt.rs; measured: every discard reference,
    /// tools/mali/testdata/fvgtest_discard2.expect.json's `DISCARD.f32.lt
    /// r0, u4.w0`). The condition is a float compare's mask: an integer
    /// compare's is lowered to `DISCARD.f32 mask, 0 .ne`, a constant's to
    /// `demote`'s `DISCARD.f32 0, 0 .eq`, forms no reference draws
    /// ([`MaliError::DiscardForm`]).
    fn discard(&mut self, c: Val, at: usize) -> Result<(), MaliError> {
        let Val::Lane(cn, _) = c else { return Err(MaliError::DiscardForm) };
        if !matches!(self.nir.nodes[cn as usize].kind, Kind::Alu(Alu::Fcmp(_))) {
            return Err(MaliError::DiscardForm);
        }
        /* a mediump shader's compare is 16-bit (`pan_nir_lower_bool_to_bitsize`
         * gives the mask its sources' size), `FCMP.v2f16` of one lane,
         * which the fusion makes `DISCARD.f32` of the halves
         * (alu_branchlane.expect.json: `DISCARD.f32.lt 0x38000000.h1,
         * u2.w0.h1`) */
        if self.nir.nodes[cn as usize].bits != if self.prec.highp { 32 } else { 16 } {
            return Err(MaliError::DiscardForm);
        }
        /* two varying lanes, of one varying or two (`v.y < v.x`, `v.x <
         * w.x`): ours narrows each and compares the halves (10 words);
         * Mesa compares the fp32 lanes, `DISCARD.f32.lt r1^, r0^` (8
         * words, tools/mali/testdata/discard16_two_lanes*). Not
         * transcribed: refused */
        let s = self.nir.nodes[cn as usize].src;
        if !self.prec.highp && self.input_lane(s[0]).is_some() && self.input_lane(s[1]).is_some() {
            return Err(MaliError::FsOperandForm);
        }
        if self.discards as usize >= MAX_DISCARDS {
            return Err(MaliError::BlockTableFull);
        }
        let nd = NirNode { kind: Kind::Discard, src: [c, Val::Undef, Val::Undef], bits: 32, width: 1 };
        let id = self.nir.push_node(nd)?;
        self.origin[id as usize] = at as u16;
        self.discards += 1;
        Ok(())
    }

    /// Local `l`'s first reference, if this is it ([`Fs::rank`]).
    fn rank_local(&mut self, l: usize) -> Result<(), MaliError> {
        let r = self.rank.get_mut(l).ok_or(MaliError::Defect(Defect::IrIndex))?;
        if *r == u32::MAX {
            *r = self.next_rank;
            self.next_rank += 1;
        }
        Ok(())
    }

    /// The sources a node reads through (an operation's, a phi's).
    fn srcs_of(&self, v: Val) -> Option<(usize, [Val; 3])> {
        let Val::Lane(n, _) = v else { return None };
        let nd = self.nir.nodes[n as usize];
        match nd.kind {
            Kind::Alu(op) => Some((op.arity(), nd.src)),
            Kind::Phi => Some((2, nd.src)),
            Kind::Discard => Some((1, nd.src)),
            /* a computed coordinate's lanes ([`TexCoord::Computed`], a
             * cube's three; the other forms leave them undefined) */
            Kind::Tex => Some((3, nd.src)),
            _ => None,
        }
    }

    /// The input node of varying location `loc` (or gl_PointCoord, or
    /// gl_FragCoord.z).
    fn input(&mut self, loc: u8, at: usize) -> Result<u16, MaliError> {
        if let Some(n) = *self.input_slot(loc)? {
            return Ok(n);
        }
        let n = self.nir.push_node(NirNode {
            kind: Kind::Input { loc, comps: 4 },
            src: [Val::Undef; 3],
            bits: 32,
            width: 1,
        })?;
        self.origin[n as usize] = at as u16;
        *self.input_slot(loc)? = Some(n);
        Ok(n)
    }

    fn input_slot(&mut self, loc: u8) -> Result<&mut Option<u16>, MaliError> {
        match loc {
            POINT_COORD => Ok(&mut self.pcoord),
            FRAG_Z => Ok(&mut self.fz),
            _ => self.inputs.get_mut(loc as usize).ok_or(MaliError::VaryingLocation),
        }
    }

    /// The node of uniform dword `d` (one fp16 lane, or an int uniform's
    /// 32-bit word: Mesa's fp16 lowering is of floats, uniidx_i's `fidx` is
    /// read whole, `LSHIFT_OR.i32 r2, u2.w0, ..`).
    fn uniform_lane(&mut self, d: u16, bits: u8, at: usize) -> Result<Val, MaliError> {
        let nd = NirNode { kind: Kind::Uniform { dword: d, comps: 1 }, src: [Val::Undef; 3], bits, width: 1 };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let id = self.nir.push_node(nd)?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// gl_FragCoord's four lanes in a highp shader, built once (IR
    /// instruction `at`, its first read) as Mesa 26.2.1's NIR has them on
    /// v10. `st_nir_lower_wpos_ytransform` (st_glsl_to_nir.cpp:395-435)
    /// rewrites the read first: panfrost's caps are an upper-left origin
    /// and integer centres (pan_screen.c:809-816), so
    /// `nir_lower_wpos_ytransform` inverts y and adds 0.5 to both
    /// (nir_lower_wpos_ytransform.c:159-204, `emit_wpos_adjustment`
    /// :60-118): `x + 0.5` and `ffma(y + 0.5, T.x, T.y)`, `T` the
    /// `gl_FbWposYTransform` state uniform ([`FB_WPOS_Y_TRANSFORM`]). Then
    /// `nir_lower_frag_coord_to_pixel_coord` (bifrost_nir.c:904-910,
    /// nir_lower_frag_coord_to_pixel_coord.c:20-24) makes the read
    /// `vec4(u2f32(load_pixel_coord), z, w)`: the 16-bit vec2 the hardware
    /// preloads (r59, [`Kind::PixelCoord`]), each lane a scalar `u2f32` of a
    /// 16-bit source after `nir_lower_alu_width` (bifrost_nir.c:176-179).
    /// Measured: fvg_fill_gradient_nodiscard_noscissor.expect.json's
    /// `U16_TO_F32` x2, `FADD_IMM` x2, `FMA.f32 r6, r6^, u9.w0, u9.w1`. z and
    /// w are [`Kind::FragCoordZw`], refused where the colour reads them
    /// ([`layout`]).
    fn frag_coord(&mut self, at: usize) -> Result<[Val; 4], MaliError> {
        let (xy, zw) = match (self.frag_xy, self.frag_zw) {
            (Some(xy), Some(zw)) => (xy, zw),
            _ => {
                let first = self.nir.n;
                let u = Val::Undef;
                let px = self.nir.push_node(NirNode { kind: Kind::PixelCoord, src: [u; 3], bits: 16, width: 2 })?;
                let x = self.nir.alu(Alu::U2f32, Val::Lane(px, 0), u, u)?;
                let y = self.nir.alu(Alu::U2f32, Val::Lane(px, 1), u, u)?;
                let x = self.nir.alu(Alu::Fadd, x, Val::Const(F32_HALF), u)?;
                let y = self.nir.alu(Alu::Fadd, y, Val::Const(F32_HALF), u)?;
                let t = self.nir.load(Kind::Uniform { dword: FB_WPOS_Y_TRANSFORM, comps: 2 })?;
                let y = self.nir.alu(Alu::Ffma, y, Val::Lane(t, 0), Val::Lane(t, 1))?;
                let zw = self.nir.push_node(NirNode { kind: Kind::FragCoordZw, src: [u; 3], bits: 32, width: 1 })?;
                for id in first..self.nir.n {
                    self.origin[id] = at as u16;
                }
                self.frag_xy = Some([x, y]);
                self.frag_zw = Some(zw);
                ([x, y], zw)
            }
        };
        Ok([xy[0], xy[1], Val::Lane(zw, 0), Val::Lane(zw, 1)])
    }

    /// gl_FragCoord's lanes in a mediump shader of ES 1.00, where the
    /// built-in is mediump (builtin_variables.cpp:1333-1335): x and y are
    /// built at fp32 as in a highp shader ([`Fs::frag_coord`]), each then
    /// narrowed by its own `f2f16` (the `f2fmp` lower_precision puts on the
    /// fp32 read, `FADD.f32 r0.h0, r0^, 0x0.neg`), and the two halves are
    /// one word by `MKVEC.v2i16` where an operation reads them as a pair
    /// (prec_84.expect.json; docs/notes/mali-fragcoord-mediump.md 3). z and
    /// w stay [`Kind::FragCoordZw`]'s.
    fn frag_coord16(&mut self, at: usize) -> Result<[Val; 4], MaliError> {
        let [x, y, z, w] = self.frag_coord(at)?;
        let xy = match self.frag16 {
            Some(xy) => xy,
            None => {
                let mut xy = [Val::Undef; 2];
                for (o, v) in xy.iter_mut().zip([x, y]) {
                    let nd =
                        NirNode { kind: Kind::Alu(Alu::F2f16), src: [v, Val::Undef, Val::Undef], bits: 16, width: 1 };
                    let n = self.nir.push_node(nd)?;
                    self.origin[n as usize] = at as u16;
                    *o = Val::Lane(n, 0);
                }
                self.frag16 = Some(xy);
                xy
            }
        };
        /* z is the special varying's fp32 lane ([`FRAG_Z`]), read as a
         * varying's lane is: narrowed by its own conversion where the
         * colour reads it (deqp_misc_fragcoord_xyz: `LD_VAR_SPECIAL.f32 ..
         * index:0x3`, then `FADD.f32 r2.h0, r2^, 0x0.neg`); w stays
         * refused */
        let _ = z;
        let z = Val::Lane(self.input(FRAG_Z, at)?, 0);
        Ok([xy[0], xy[1], z, w])
    }

    /// The lane of the narrowed position `v` is ([`Fs::frag_coord16`]): 0
    /// for x, 1 for y.
    fn frag16_lane(&self, v: Val) -> Option<u8> {
        self.frag16?.iter().position(|&l| l == v).map(|k| k as u8)
    }

    /// One lane of an fp32 operation of a highp shader: the vertex stage's
    /// folding, rules and CSE ([`Nir::alu`]) over operands that are fp32 (a
    /// varying of the shader's width, a constant, an fp32 result).
    fn op32(&mut self, op: Alu, s: [Val; 3], at: usize) -> Result<Val, MaliError> {
        let mut s = s;
        /* a texel read with a highp value (not a constant, which takes the
         * texel's precision): GLSL's lower_precision leaves the look-up
         * whose result such an expression takes at fp32, as one stored to
         * a highp local (femtovg's filter_image, `texture2D(tex, ..) *
         * gaussian_coeff.x`: `TEX_SINGLE .. .32.2d` in
         * fvg_filter_image.expect.json; hdrsample's `texture(..) * 0.25`
         * stays fp16, mixprec_hdrsample.expect.json) */
        let arity = op.arity();
        let highp = |f: &Self, v: Val| matches!(v, Val::Lane(..)) && f.texel(v).is_none() && !f.is_f16(v);
        if s[..arity].iter().any(|&v| highp(self, v)) {
            for &v in &s[..arity] {
                if let Some((j, _)) = self.texel(v) {
                    self.tex32(self.look_ups[j].node)?;
                }
            }
        }
        for v in s.iter_mut().take(op.arity()) {
            if *v == Val::Undef {
                return Err(MaliError::ColourForm);
            }
            self.mark_mixed(*v);
            *v = self.widen(*v, at)?;
        }
        let first = self.nir.n;
        let r = self.nir.alu(op, s[0], s[1], s[2])?;
        for id in first..self.nir.n {
            self.origin[id] = at as u16;
        }
        Ok(r)
    }

    /// `inot(c)` of a highp compare's mask `c`, folded into the inverse
    /// compare (nir_opt_algebraic.py: `inot(flt(a, b))` is `fge(a, b)`,
    /// `inot(ieq(a, b))` `ine(a, b)`), made where `c` was.
    fn inverse_compare(&mut self, c: Val) -> Result<Val, MaliError> {
        let Val::Lane(n, 0) = c else { return Err(MaliError::IfForm) };
        let nd = self.nir.nodes[n as usize];
        let inv = |x: Cmpf| match x {
            Cmpf::Lt => Cmpf::Ge,
            Cmpf::Ge => Cmpf::Lt,
            Cmpf::Le => Cmpf::Gt,
            Cmpf::Gt => Cmpf::Le,
            Cmpf::Eq => Cmpf::Ne,
            Cmpf::Ne => Cmpf::Eq,
        };
        let op = match nd.kind {
            Kind::Alu(Alu::Fcmp(x)) if nd.bits == 32 => Alu::Fcmp(inv(x)),
            Kind::Alu(Alu::Icmp(x)) if nd.bits == 32 => Alu::Icmp(inv(x)),
            _ => return Err(MaliError::IfForm),
        };
        let at = self.origin[n as usize] as usize;
        self.op32(op, nd.src, at)
    }

    /// `v` is a lane of an fp16 value of a highp shader: a mediump
    /// expression on gl_FragCoord ([`Fs::alu16`]), not a compare's mask.
    /// `v` is a 32-bit integer compare's mask (gl_FrontFacing's in a
    /// mediump shader, [`mask_colour`]).
    /// Also a boolean the walk folded (0 or ~0), its `inot`, and a 32-bit
    /// uniform word read as a boolean (case 112's `uniform bool`, whose
    /// word Mesa's `bcsel_pan` tests as it is, intbig_case112.stderr).
    fn mask_lane(&self, v: Val) -> bool {
        match v {
            Val::Const(c) => c == 0 || c == u32::MAX,
            Val::Lane(n, 0) => matches!(
                self.nir.nodes[n as usize],
                NirNode { kind: Kind::Alu(Alu::Icmp(_) | Alu::Inot) | Kind::Uniform { .. }, bits: 32, width: 1, .. }
            ),
            _ => false,
        }
    }

    /// `v` is a mediump shader's 32-bit select of two constants on a
    /// boolean ([`Fs::mask_lane`], [`Fs::mask16`]): a value of no precision
    /// (lower_precision lowers no operation of constants and booleans), so
    /// a sum of two stays fp32 and is narrowed at the store
    /// (intbool_76_boolany: `CSEL.u32.eq` of `u_b ? 0.5 : 0.0`, the
    /// `FADD.f32` of the three, then `FADD.f32 r0.h0`).
    fn select32(&self, v: Val) -> bool {
        matches!(self.alu_of(v), Some((Alu::Bcsel, [_, Val::Const(_), Val::Const(_)]) | (Alu::B2f, _)))
            && matches!(v, Val::Lane(n, _) if self.nir.nodes[n as usize].bits == 32)
            && !self.prec.highp
    }

    /// `v` is a 16-bit boolean of a mediump shader: an fp16 compare's mask,
    /// the `ior` / `iand` / `ixor` of such masks ([`Fs::mask_op`],
    /// [`Fs::int_lane`]), or the equality of two (`bvec == bvec`).
    fn mask16(&self, v: Val) -> bool {
        matches!(v, Val::Lane(n, 0) if matches!(
            self.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_) | Alu::Ior | Alu::Iand | Alu::Ixor), bits: 16, width: 1, .. }
        )) && !self.prec.highp
    }

    /// `v` is the `b2f` of a 16-bit mask ([`Fs::mask16`]) at either width
    /// (the walk's `bcsel(m, 1.0, 0.0)` at 32 bits, a product of two at
    /// 16): Some(the mask).
    fn cmp_select(&self, v: Val) -> Option<Val> {
        match self.alu_of(v) {
            Some((Alu::B2f, [m, ..])) if self.mask16(m) => Some(m),
            _ => None,
        }
    }

    /// [`Fs::cmp_select`] at 32 bits: a `?:` of a mediump compare.
    fn cmp_select32(&self, v: Val) -> bool {
        self.cmp_select(v).is_some() && matches!(v, Val::Lane(n, _) if self.nir.nodes[n as usize].bits == 32)
    }

    /// `iand` of two 16-bit masks: `iand(flt(a, c), flt(b, c))` is
    /// `flt(fmax(a, b), c)` (nir_opt_algebraic.py:1159), else the 16-bit
    /// `iand` ([`Fs::mask_op`]).
    fn and_masks(&mut self, a: Val, b: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        if let (Some((Alu::Fcmp(Cmpf::Lt), [x, c @ Val::Const(_), _])), Some((Alu::Fcmp(Cmpf::Lt), [y, d, _]))) =
            (self.alu_of(a), self.alu_of(b))
        {
            if c == d {
                let m = self.node(Alu::Fmax, [x, y, u], at)?;
                return self.node(Alu::Fcmp(Cmpf::Lt), [m, c, u], at);
            }
        }
        self.mask_op(Alu::Iand, a, b, at)
    }

    /// The `b2f` of the product of two compare selects `x` and `y`
    /// ([`Fs::cmp_select`]), its masks in Mesa's order. Measured with
    /// NIR_DEBUG=print_fs on the drm-shim (deqp_mixprec_cmpv4, Mesa
    /// 26.2.1): the product of two leaves is `b2f(iand(a, b))`
    /// (nir_opt_algebraic.py:1679) in source order, and so is the product
    /// of two products once both are `b2f`s (deqp_mixprec_cmpmat2, column 0
    /// first). A product of a product still a `fmul` and a leaf is first
    /// `bcsel(leaf, product, 0.0)` (`fmul(b2f(a), b)`), so in a chain
    /// `((l1 * l2) * l3) * l4` the leaves come first: `bcsel(l4, bcsel(l3,
    /// v, 0), 0)` is `bcsel(iand(l4, l3), v, 0)`, which is `iand(iand(l4,
    /// l3), iand(l1, l2))` (cmpv3: `fmax(z, fmax(x, y))`; cmpv4:
    /// `fmax(fmax(w, z), fmax(x, y))`). A chain past four leaves has no
    /// reference; it folds each new leaf in front of the ones before.
    fn cmp_product(&mut self, x: Val, y: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let (Some(a), Some(b)) = (self.cmp_select(x), self.cmp_select(y)) else {
            return Err(MaliError::Defect(Defect::IrIndex));
        };
        let entry = |f: &Self, v: Val| f.cmp_chain[..f.ncmp_chain].iter().find(|e| e.0 == v).copied();
        let (mask, leaves, base) = match (entry(self, x), entry(self, y)) {
            (Some((_, lx, bx)), None) | (None, Some((_, lx, bx))) => {
                let leaf = if entry(self, x).is_some() { b } else { a };
                let leaves = match lx {
                    Val::Undef => leaf,
                    l => self.and_masks(leaf, l, at)?,
                };
                (self.and_masks(leaves, bx, at)?, leaves, bx)
            }
            _ => {
                let m = self.and_masks(a, b, at)?;
                (m, u, m)
            }
        };
        let r = self.node(Alu::B2f, [mask, u, u], at)?;
        if self.ncmp_chain < MAX_CMP_CHAIN && entry(self, r).is_none() {
            self.cmp_chain[self.ncmp_chain] = (r, leaves, base);
            self.ncmp_chain += 1;
        }
        Ok(r)
    }

    /// `v` is a 32-bit boolean ([`Fs::mask_lane`]) or the `ior` / `iand`
    /// of two ([`Fs::mask_op`]).
    fn bool32(&self, v: Val) -> bool {
        match self.alu_of(v) {
            Some((Alu::Ior | Alu::Iand, [a, b, _])) if !self.mask16(v) => self.bool32(a) && self.bool32(b),
            _ => self.mask_lane(v) || self.fcmp32(v),
        }
    }

    /// `v` is the 32-bit mask of an fp32 compare of a mediump shader (an
    /// integer's float on one side, the walk's compare arm): a boolean of
    /// its compare's width (`pan_nir_lower_bool_to_bitsize`).
    fn fcmp32(&self, v: Val) -> bool {
        matches!(v, Val::Lane(n, 0) if matches!(
            self.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::Fcmp(_)), bits: 32, width: 1, .. }
        )) && !self.prec.highp
    }

    /// `iand(p, q)` of two fp32 compares' masks ([`Fs::fcmp32`]) that no
    /// rule of nir_opt_algebraic.py rewrites: the `&&` stays a `iand`,
    /// which `va_fuse_cmp` makes one `FCMP_AND.f32` (deqp_intval_reslib_
    /// int_vs's `float(b) <= atemp && atemp <= float(b + 1)`, `atemp` on
    /// either side). A constant operand (the rules of :918, :1162-1165,
    /// :1206-1214), an operand shared in the same place (:1158-1161) or
    /// the same compare with its sources swapped (:749-750) is another
    /// program in Mesa, with no reference: refused.
    fn and_fcmp32(&self, p: Val, q: Val) -> bool {
        let (Some((Alu::Fcmp(c), [a, b, _])), Some((Alu::Fcmp(d), [x, y, _]))) = (self.alu_of(p), self.alu_of(q))
        else {
            return false;
        };
        self.fcmp32(p)
            && self.fcmp32(q)
            && ![a, b, x, y].iter().any(|v| matches!(v, Val::Const(_)))
            && a != x
            && b != y
            && !(c == d && a == y && b == x)
    }

    /// `op` (`ior`, `iand`) of two booleans of `any` / `all`: two constants
    /// folded, two 32-bit masks a 32-bit node, two 16-bit masks a 16-bit
    /// one (`pan_nir_lower_bool_to_bitsize` gives a bool its compare's
    /// width). Another mix has no reference.
    fn mask_op(&mut self, op: Alu, a: Val, b: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        match (a, b) {
            (Val::Const(x), Val::Const(y)) => Ok(Val::Const(if op == Alu::Ior { x | y } else { x & y })),
            _ if self.bool32(a) && self.bool32(b) => self.int32(op, [a, b, u], at),
            _ if self.mask16(a) && self.mask16(b) => self.node(op, [a, b, u], at),
            _ => Err(MaliError::IntValue),
        }
    }

    /// A 32-bit integer node of a mediump shader ([`Fs::int_lane`]),
    /// CSE'd (`nir_opt_cse`), made by IR instruction `at`.
    fn int32(&mut self, op: Alu, src: [Val; 3], at: usize) -> Result<Val, MaliError> {
        let nd = NirNode { kind: Kind::Alu(op), src, bits: 32, width: 1 };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let id = self.nir.push_node(nd)?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// One lane of a mediump shader's integer or boolean operation `a`
    /// over the lane's sources `x` (Mesa's NIR keeps integers and booleans
    /// at 32 bits in a mediump shader: lower_precision lowers floats only,
    /// and `pan_nir_lower_bool_to_bitsize` gives a compare's bool its
    /// operands' width). Folded first ([`nir::fold_int`]), then the forms
    /// the references draw: `f2i32` of an fp16 value (`F16_TO_S32.rtz`),
    /// the integer compares (GLSL's `<=` swapped into `ige`), a boolean's
    /// `inot` folded into its users, `ieq(b, true)` as `b` and `ieq(b,
    /// false)` as `inot(b)` (nir_opt_algebraic.py), a boolean uniform's
    /// `i2b` as the word itself. Another has no reference.
    fn int_lane(
        &mut self,
        m: &Module<'_>,
        a: AluOp,
        srcs: &[ir::Src; 3],
        x: [Val; 3],
        at: usize,
    ) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let [p, q, _] = x;
        if let Some(v) = nir::fold_int(a, p, q) {
            return Ok(v);
        }
        let not = |f: &Self, v: Val| match f.alu_of(v) {
            Some((Alu::Inot, [inner, ..])) => Some(inner),
            _ => None,
        };
        let bool_dword = |d: u16| {
            m.uniforms.iter().any(|u| {
                d >= u.dword_off
                    && d < u.dword_off + u.dwords.max(1)
                    && matches!(u.gl_type as u32, GL_BOOL | GL_BOOL_VEC2 | GL_BOOL_VEC3 | GL_BOOL_VEC4)
            })
        };
        let bool_uniform = |k: usize| matches!(m.def(srcs[k].val), Some(ir::Op::LoadUniform { dword_off, .. }) if bool_dword(*dword_off));
        /* a bool uniform's word ([`Fs::uniform_lane`] at 32 bits), read
         * through a local too (a function's parameter, dEQP's `isOk(bvec3 a,
         * bvec3 b)`) */
        let bool_word = |f: &Self, v: Val| match v {
            Val::Lane(n, _) => matches!(f.nir.nodes[n as usize],
                NirNode { kind: Kind::Uniform { dword, .. }, bits: 32, .. } if bool_dword(dword)),
            _ => false,
        };
        /* an `int` / `ivec` uniform's word, a 32-bit integer and no mask
         * (`caps->int16` is false: it stays 32 bits in a mediump shader),
         * which [`Fs::mask_lane`] cannot tell from a bool's: dEQP's
         * `isOk(float a, int b)` reads `float(b)` and `float(b + 1)` of the
         * `ref_out0` word (deqp_intval_reslib_int_vs: `S32_TO_F32 r1,
         * u2.w0`, `IADD_IMM.i32 r2, u2.w0, #0x1`) */
        let int_word = |f: &Self, v: Val| match v {
            Val::Lane(n, _) => matches!(f.nir.nodes[n as usize],
                NirNode { kind: Kind::Uniform { dword, .. }, bits: 32, .. } if !bool_dword(dword)),
            _ => false,
        };
        let int = |f: &Self, v: Val| !f.mask_lane(v) || int_word(f, v);
        match a {
            AluOp::F2i if self.is_f16(p) || self.uniform_dword(p).is_some() => self.int32(Alu::F2i32, [p, u, u], at),
            /* of an fp32 value (case 42's `int(uv.x * float(sz.x))`,
             * `F32_TO_S32.rtz`), or of a varying's lane as loaded: the
             * conversion of its narrowing folds away (`f2i32(f2fmp(a))` is
             * `f2i32(a)`, nir_opt_algebraic.py), so dEQP's `int(v_in0.z)`
             * reads the fp32 word (deqp_intval_reslib_int_fs,
             * deqp_intval_opfs_add_int_mediump: `F32_TO_S32.rtz r0, r0^`
             * right after the load) */
            AluOp::F2i if self.fp32_lane(p) || self.input_lane(p).is_some() => self.int32(Alu::F2i, [p, u, u], at),
            AluOp::I2b if bool_uniform(0) => Ok(p),
            /* `inot(flt(a, b))` is `fge(a, b)` and the reverse
             * (nir_opt_algebraic.py), a mediump compare's 16-bit mask
             * (gm2_fsmisc_8's `step(0.5, x)`, `!(x < 0.5)`: `FCMP_OR.v2f16.ge.m1
             * r0, r0^, 0x38000000.h11, 0x0`) */
            AluOp::Bnot | AluOp::Inot if self.mask16(p) => match self.alu_of(p) {
                Some((Alu::Fcmp(Cmpf::Lt), [a, b, _])) => self.node(Alu::Fcmp(Cmpf::Ge), [a, b, u], at),
                Some((Alu::Fcmp(Cmpf::Ge), [a, b, _])) => self.node(Alu::Fcmp(Cmpf::Lt), [a, b, u], at),
                _ => Err(MaliError::IntValue),
            },
            /* `b2f` of such a mask: one 16-bit node, which a product makes a
             * select ([`Fs::op`]); any other reader has no reference */
            AluOp::B2f if self.mask16(p) => self.node(Alu::B2f, [p, u, u], at),
            /* `float(b)` of a 32-bit boolean (dEQP's `vec4(RES, RES, RES,
             * 1.0)` of an int `isOk`, deqp_intval_reslib_int_fs): the
             * `bcsel_pan(b, 1.0, 0.0)` the colour's store folds into its
             * `MUX.v2i16` ([`mask_colour`]), as the Bcsel arm of the walk
             * makes `b ? 1.0 : 0.0` */
            AluOp::B2f if self.bool32(p) => self.op32(Alu::B2f, [p, u, u], at),
            AluOp::Bnot | AluOp::Inot if self.mask_lane(p) => match not(self, p) {
                Some(inner) => Ok(inner),
                None => self.int32(Alu::Inot, [p, u, u], at),
            },
            /* `bool(x)` is `fneu(x, 0.0)` (glsl_to_nir.cpp's `f2b`): of a
             * mediump lane, the fp16 compare of its converted value
             * (deqp_intval_reslib_bvec_fs's `bvec3(v_in0)`, `FCMP_OR.v2f16
             * .ne.m1 r1, r1^.h00, 0x0, 0x0` of the narrowed lane,
             * [`convert_inputs16`]); of an fp32 value, the fp32 one */
            AluOp::F2b if self.input_lane(p).is_some() || self.is_f16(p) => {
                self.node(Alu::Fcmp(Cmpf::Ne), [p, Val::Const(0), u], at)
            }
            AluOp::F2b if self.fp32_lane(p) => self.op32(Alu::Fcmp(Cmpf::Ne), [p, Val::Const(0), u], at),
            /* `bvec == bvec` of two fp16 compares' masks: `ieq_pan` at 16
             * bits (deqp_intval_reslib_bvec_fs, deqp_intval_opfs_equal_bvec2:
             * `ICMP_OR.v2u16.eq.m1 r1, r1^.h00, u3.w0.h00, 0x0`); a bool
             * uniform's word takes the mask's width (`make_sources_canonical`,
             * pan_nir_lower_bool_to_bitsize.c: the first source that is no
             * constant), read through its low half */
            AluOp::Ieq | AluOp::Ine if self.mask16(p) && (self.mask16(q) || bool_word(self, q)) => {
                let c = if a == AluOp::Ieq { Cmpf::Eq } else { Cmpf::Ne };
                self.node(Alu::Icmp(c), [p, q, u], at)
            }
            /* `^^` of two masks: `LSHIFT_XOR.v2i16` of fp16 compares'
             * (deqp_intval_opfs_logical_xor), `LSHIFT_XOR.i32` of integer
             * ones */
            AluOp::Ixor if self.mask16(p) && self.mask16(q) => self.node(Alu::Ixor, [p, q, u], at),
            AluOp::Ixor if self.bool32(p) && self.bool32(q) => self.int32(Alu::Ixor, [p, q, u], at),
            /* `&&` of two fp32 compares' masks ([`Fs::and_fcmp32`]): a
             * 32-bit `iand`, `FCMP_AND.f32` once fused */
            AluOp::Iand if self.and_fcmp32(p, q) => self.mask_op(Alu::Iand, p, q, at),
            /* `int(b)` of an fp16 compare's mask: `b2i32` keeps its 16-bit
             * source (no arm of `lower_alu_instr` resizes it), `MUX.i32 0,
             * 1, b` reading the mask replicated into a word
             * (`bi_lower_swizzle`; deqp_intval_reslib_mixed_fs's `ivec3(in0,
             * ..)`: `IADD.v2u16 r0, r0^.h00, 0x0` then `CSEL.u32.eq r0, r0^,
             * 0x0, 0x0, u4.w0`, [`select_highp`]) */
            AluOp::B2i if self.mask16(p) || self.bool32(p) => self.int32(Alu::B2i, [p, u, u], at),
            /* `ieq(b, true)` is `b`, `ieq(b, false)` `inot(b)` */
            AluOp::Ieq | AluOp::Ine if self.mask_lane(p) && matches!(q, Val::Const(0 | u32::MAX)) => {
                let same = (q == Val::Const(u32::MAX)) == (a == AluOp::Ieq);
                if same {
                    Ok(p)
                } else {
                    self.int_lane(m, AluOp::Bnot, srcs, [p, u, u], at)
                }
            }
            AluOp::Ieq | AluOp::Ine if self.mask_lane(q) && matches!(p, Val::Const(0 | u32::MAX)) => {
                self.int_lane(m, a, srcs, [q, p, u], at)
            }
            AluOp::Ilt => self.int32(Alu::Icmp(Cmpf::Lt), [p, q, u], at),
            /* GLSL's `a <= b` is `ige(b, a)`, a constant on the left as it
             * comes (intloop_int_compare.expect.json: `ICMP_OR.s32.ge.m1 r3,
             * r0, r1` of `i <= j`, `ICMP_OR.s32.ge.m1 r1, 0x1000000.b3, r1^`
             * of `i <= 1`) */
            AluOp::Ile => self.int32(Alu::Icmp(Cmpf::Ge), [q, p, u], at),
            AluOp::Ieq => self.int32(Alu::Icmp(Cmpf::Eq), [p, q, u], at),
            /* case 79's `int(abs(u_d.z)) + 1` (`IADD_IMM.i32 r1, r1^, #0x2`
             * once fused), `float(q.x)` (an fp32 value: lower_precision
             * lowers no conversion from an integer, intbig_case79.stderr's
             * `i2f32` then `fmul` at 32 bits) and the division */
            /* `a - b` is the front end's `iadd(a, ineg(b))`, Mesa's `isub`
             * (`has_isub`; deqp_intval_opfs_sub_assign_ivec2_lowp's `res -=
             * in1`, `ISUB.s32 r0, r0^, r3^`) */
            AluOp::Iadd if int(self, p) && matches!(self.alu_of(q), Some((Alu::Ineg, _))) => {
                let Some((_, [b, ..])) = self.alu_of(q) else { return Err(MaliError::Defect(Defect::IrIndex)) };
                self.int32(Alu::Isub, [p, b, u], at)
            }
            AluOp::Iadd if int(self, p) && int(self, q) => self.int32(Alu::Iadd, [p, q, u], at),
            /* the arithmetic of the operator suite (deqp_intval_opfs_mul_
             * ivec4_highp: `IMUL.i32` per lane; a `-i` alone `ISUB.s32 0, a`) */
            AluOp::Imul if int(self, p) && int(self, q) => self.int32(Alu::Imul, [p, q, u], at),
            AluOp::Ineg if int(self, p) => self.int32(Alu::Ineg, [p, u, u], at),
            AluOp::I2f if int(self, p) => self.op32(Alu::I2f, [p, u, u], at),
            /* `float(v_idx)` of a flat `uint` varying (case 89), fp32 as
             * `i2f32` is; motion.rs moves it into the vertex shader */
            AluOp::U2f if int(self, p) => self.op32(Alu::U2f, [p, u, u], at),
            AluOp::Idiv if int(self, p) && int(self, q) => self.idiv(p, q, at),
            /* `!=` of run-time integers (tri_int's `ine`, whose fp16 operands
             * feed fp32 integer arithmetic, not lowered), the arithmetic,
             * and the mixes of a boolean and its negation (`iand(a,
             * inot(a))`): no reference compiles */
            _ => Err(MaliError::IntValue),
        }
    }

    /// One 32-bit integer node of [`Fs::idiv`], folded as NIR's constant
    /// folding and algebraic rules leave it: a constant's half
    /// (`extract_u16`), a product with 0, a sum with 0, and
    /// `extract_u16(extract_u16(a, 1), 0)` as its source (the half is all
    /// the word holds).
    fn iop(&mut self, op: Alu, a: Val, b: Val, at: usize) -> Result<Val, MaliError> {
        let c = |v: Val| match v {
            Val::Const(c) => Some(c),
            _ => None,
        };
        let k = |x: u32| Ok(Val::Const(x));
        match (op, c(a), c(b)) {
            (Alu::Imul, Some(p), Some(q)) => return k(p.wrapping_mul(q)),
            (Alu::Imul, Some(0), _) | (Alu::Imul, _, Some(0)) => return k(0),
            (Alu::Iadd, Some(p), Some(q)) => return k(p.wrapping_add(q)),
            (Alu::Iadd, Some(0), _) => return Ok(b),
            (Alu::Iadd, _, Some(0)) => return Ok(a),
            (Alu::Isub, Some(p), Some(q)) => return k(p.wrapping_sub(q)),
            (Alu::Ixor, Some(p), Some(q)) => return k(p ^ q),
            (Alu::Ineg, Some(p), _) => return k(p.wrapping_neg()),
            (Alu::Iabs, Some(p), _) => return k((p as i32).unsigned_abs()),
            (Alu::Extract16(h), Some(p), _) => return k(p >> (16 * u32::from(h)) & 0xffff),
            _ => {}
        }
        if let (Alu::Extract16(0), Some((Alu::Extract16(1), _))) = (op, self.alu_of(a)) {
            return Ok(a);
        }
        self.int32(op, [a, b, Val::Undef], at)
    }

    /// `umul_high(x, y)` as `nir_lower_idiv` builds it for a GPU without a
    /// high multiply (`nir_umul_high` lowered by `lower_umul_high` into the
    /// 16-bit halves' four products and their carries, the order of
    /// intbig_case79.stderr's `%23`..`%38`). With a constant `x` (`100 /
    /// ..`) the folds leave two products (`%160`..`%168`), and the high
    /// half of a half, `extract_u16(extract_u16(c, 1), 1)`, is the zero
    /// Mesa's BIR folds only after selection: its sum stays, `IADD_IMM.i32
    /// r4, r4^, #0x0`.
    fn umul_high(&mut self, x: Val, y: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let x0 = self.iop(Alu::Extract16(0), x, u, at)?;
        let y0 = self.iop(Alu::Extract16(0), y, u, at)?;
        let x1 = self.iop(Alu::Extract16(1), x, u, at)?;
        let y1 = self.iop(Alu::Extract16(1), y, u, at)?;
        let p00 = self.iop(Alu::Imul, x0, y0, at)?;
        let p01 = self.iop(Alu::Imul, x0, y1, at)?;
        let p10 = self.iop(Alu::Imul, x1, y0, at)?;
        let p11 = self.iop(Alu::Imul, x1, y1, at)?;
        let carry = self.iop(Alu::Extract16(1), p00, u, at)?;
        let s1 = self.iop(Alu::Iadd, p10, carry, at)?;
        let s1l = self.iop(Alu::Extract16(0), s1, u, at)?;
        let late = matches!(self.alu_of(s1), Some((Alu::Extract16(1), _)));
        let s1h = if late { Val::Const(0) } else { self.iop(Alu::Extract16(1), s1, u, at)? };
        let s2 = self.iop(Alu::Iadd, p01, s1l, at)?;
        let s2h = self.iop(Alu::Extract16(1), s2, u, at)?;
        let t = if late { self.int32(Alu::Iadd, [s2h, s1h, u], at)? } else { self.iop(Alu::Iadd, s2h, s1h, at)? };
        self.iop(Alu::Iadd, p11, t, at)
    }

    /// `n / d` of two 32-bit signed integers, `nir_lower_idiv`'s sequence
    /// (nir_lower_idiv.c, `emit_udiv` under `nir_lower_idiv_fast`'s
    /// absence: the reciprocal estimate by `frcp` scaled by `0x4f7ffffe`,
    /// refined once by `umul_high`, the quotient by another, then two
    /// corrections and the sign), node for node in the order of case 79's
    /// NIR (intbig_case79.stderr `%13`..`%68`): the first `b2i32` a
    /// `CSEL` (its compare also feeds the remainder's select), the second
    /// folded into its compare (`ICMP_OR.u32.ge.i1`, opt.rs).
    fn idiv(&mut self, n: Val, d: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let a = self.iop(Alu::Iabs, n, u, at)?;
        let b = self.iop(Alu::Iabs, d, u, at)?;
        let sign = self.iop(Alu::Ixor, n, d, at)?;
        let f = self.int32(Alu::U2f, [b, u, u], at)?;
        let r = self.op32(Alu::Frcp, [f, u, u], at)?;
        let m = self.op32(Alu::Fmul, [r, Val::Const(IDIV_RCP_SCALE), u], at)?;
        let e = self.int32(Alu::F2u, [m, u, u], at)?;
        let t = self.iop(Alu::Imul, b, e, at)?;
        let neg = self.iop(Alu::Ineg, t, u, at)?;
        let hi = self.umul_high(e, neg, at)?;
        let e = self.iop(Alu::Iadd, e, hi, at)?;
        let q = self.umul_high(a, e, at)?;
        let qd = self.iop(Alu::Imul, q, b, at)?;
        let rem = self.iop(Alu::Isub, a, qd, at)?;
        let c1 = self.int32(Alu::Ucmp(Cmpf::Ge), [rem, b, u], at)?;
        let i1 = self.int32(Alu::B2i, [c1, u, u], at)?;
        let q = self.iop(Alu::Iadd, q, i1, at)?;
        let rs = self.iop(Alu::Isub, rem, b, at)?;
        let rem = self.int32(Alu::Bcsel, [c1, rs, rem], at)?;
        let c2 = self.int32(Alu::Ucmp(Cmpf::Ge), [rem, b, u], at)?;
        let i2 = self.int32(Alu::B2i, [c2, u, u], at)?;
        let q = self.iop(Alu::Iadd, q, i2, at)?;
        let nq = self.iop(Alu::Ineg, q, u, at)?;
        let neg_sign = self.int32(Alu::Icmp(Cmpf::Lt), [sign, Val::Const(0), u], at)?;
        self.int32(Alu::Bcsel, [neg_sign, nq, q], at)
    }

    /// `v` is a value of the highp gl_FragCoord's island in a mediump ES
    /// 3.00 shader: one of the position's fp32 lanes ([`Fs::frag_coord`]),
    /// or a 32-bit operation reading one, through any depth of such
    /// operations. GLSL's lower_precision cannot lower a tree with a highp
    /// leaf (`handle_precision`, lower_precision.cpp:304-305: highp is
    /// `CANT_LOWER`, and a child that cannot be lowered makes its parent so,
    /// `stack_leave`), so every operation on the built-in or on a result of
    /// one stays fp32 until its value is assigned to a mediump variable
    /// ([`Fs::fc32_float`]). Measured: case 218's `step(32.0, gl_FragCoord.x)`,
    /// `(gl_FragCoord.x - 32.0 * right) / 8.0` and its `floor`, `floor(gl_
    /// FragCoord.y / 16.0)` are `FCMP_OR.f32`, `CSEL.u32`, `FADD.f32`,
    /// `FMA.f32 .., 0x0.neg` and `FROUND.f32.rtn` (fsarith_218_shadowcmp
    /// .expect.json; docs/notes/mali-fragcoord-fp16-ops.md).
    fn fc32(&self, v: Val) -> bool {
        /* an ES 1.00 mediump shader's read is the narrowed one
         * ([`Fs::frag_coord16`]): no island */
        if self.prec.highp || !(self.prec.frag_coord_highp || self.fc_copy) {
            return false;
        }
        let Some(xy) = self.frag_xy else { return false };
        self.reaches_fc(v, xy, 0)
    }

    fn reaches_fc(&self, v: Val, xy: [Val; 2], depth: u8) -> bool {
        if xy.contains(&v) {
            return true;
        }
        let Val::Lane(n, _) = v else { return false };
        let nd = self.nir.nodes[n as usize];
        let Kind::Alu(op) = nd.kind else { return false };
        nd.bits == 32 && depth < 24 && nd.src[..op.arity()].iter().any(|&s| self.reaches_fc(s, xy, depth + 1))
    }

    /// `v` is a float of the island ([`Fs::fc32`]): the position's lane or
    /// float arithmetic on it, which lower_precision narrows (`f2fmp`) where
    /// a mediump variable takes it; not a compare's mask, nor a boolean's
    /// float (`b2f`, a select of constants), which has no precision and
    /// is read as it is (NIR's `f2fmp(b2f32(a))` is `b2f16(a)`,
    /// nir_opt_algebraic.py:1980, and `f2f32` of that `b2f32(a)` again).
    fn fc32_float(&self, v: Val) -> bool {
        let float = |op: Alu| {
            matches!(
                op,
                Alu::Fadd
                    | Alu::Fmul
                    | Alu::Ffma
                    | Alu::Ffloor
                    | Alu::Fceil
                    | Alu::Ftrunc
                    | Alu::Fneg
                    | Alu::Fabs
                    | Alu::Frcp
                    | Alu::Frsq
                    | Alu::Fmin
                    | Alu::Fmax
                    | Alu::Fclamp(_)
                    | Alu::U2f32
            )
        };
        matches!(v, Val::Lane(n, _) if matches!(self.nir.nodes[n as usize], NirNode { kind: Kind::Alu(op), bits: 32, .. } if float(op)))
            && self.fc32(v)
    }

    /// `v` is the island's value narrowed to fp16 ([`Fs::fc32_float`] at a
    /// mediump local's store), or an fp16 operation of two lanes on such
    /// values ([`Fs::alu16`]'s vectors, the only two-lane nodes a mediump
    /// shader's walk makes).
    fn narrowed(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        match self.nir.nodes[n as usize] {
            /* or of an integer's float (`i2f32`, `u2f32`) stored to a mediump
             * variable: dEQP's `color.x = float(res)` then `color * 0.10 +
             * 0.50` (deqp_intval_opfs_add_int_mediump: `S32_TO_F32`, `FADD.f32
             * r0.h0`, then `U16_TO_U32` and `FMA.v2f16` of the pair;
             * [`Fs::narrow_stored`]). Not the narrowed position of an ES
             * 1.00 shader ([`Fs::frag_coord16`], fp32 arithmetic too): the
             * pair graph's leaf ([`Leaf::Frag`]) */
            NirNode { kind: Kind::Alu(Alu::F2f16), bits: 16, width: 1, src } => {
                self.fc32_float(src[0]) || self.int_float(src[0])
            }
            NirNode { kind: Kind::Alu(_), bits: 16, width: 2, .. } => !self.prec.highp,
            _ => false,
        }
    }

    /// `v` is one lane computed from the island's narrowed value in a
    /// mediump shader: the `f2f16` of an island float ([`Fs::fc32_float`]
    /// at a mediump local's store), or an fp16 operation of one lane on
    /// such values (`fract(3.0 * d)`, a select of two of them).
    fn island16(&self, v: Val, depth: u8) -> bool {
        /* no island, nothing to search (the walk asks of every operand of
         * a mediump shader) */
        if self.prec.highp || !(self.prec.frag_coord_highp || self.fc_copy) || self.frag_xy.is_none() {
            return false;
        }
        let Val::Lane(n, _) = v else { return false };
        let nd = self.nir.nodes[n as usize];
        match nd.kind {
            Kind::Alu(Alu::F2f16) if nd.width == 1 => self.fc32_float(nd.src[0]),
            Kind::Alu(op) if nd.bits == 16 && nd.width == 1 && depth < 32 => {
                nd.src[..op.arity()].iter().any(|&s| self.island16(s, depth + 1))
            }
            _ => false,
        }
    }

    /// The operands `p` and `q` (`Undef` for a unary operation) of a
    /// one-lane fp16 operation [`Fs::alu16`] takes in a mediump shader:
    /// each a constant (already an fp16 half) or a one-lane value of the
    /// island ([`Fs::island16`]), not both constants.
    fn lone16(&self, p: Val, q: Val) -> bool {
        let one = |v: Val| matches!(v, Val::Const(_)) || self.island16(v, 0);
        !self.prec.highp && (self.island16(p, 0) || self.island16(q, 0)) && one(p) && (q == Val::Undef || one(q))
    }

    /// `v` is an integer's float (`i2f32`, `u2f32`) of a mediump shader,
    /// fp32 as the conversion leaves it (lower_precision lowers no
    /// conversion from an integer).
    fn int_float(&self, v: Val) -> bool {
        matches!(self.alu_of(v), Some((Alu::I2f | Alu::U2f, _))) && !self.prec.highp
    }

    /// `v` is an fp32 value of a mediump shader: an integer's conversion
    /// or arithmetic on one ([`Fs::int_lane`], the walk's fp32 arm).
    fn fp32_lane(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        matches!(
            self.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::I2f | Alu::U2f | Alu::Fmul | Alu::Fadd | Alu::Ffma), bits: 32, .. }
        ) && !self.prec.highp
    }

    /// `v` is a lane of an fp16 operation, or of a phi of fp16 values (the
    /// join of an `if` Mesa keeps in a mediump shader, [`kept16`]).
    fn is_f16(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        match self.nir.nodes[n as usize] {
            NirNode { kind: Kind::Alu(op), bits: 16, .. } => !op.is_int(),
            NirNode { kind: Kind::Phi, bits: 16, .. } => true,
            _ => false,
        }
    }

    /// An fp16 lane where a highp value is read: lower_precision's
    /// up-conversion at the edge of the lowered tree (`f162f`,
    /// lower_precision.cpp:693-694; NIR's `f2f32`, `F16_TO_F32 rd, rs.hN`
    /// of the lane's half, bifrost_compile.c:3073). Any other lane is
    /// itself.
    fn widen(&mut self, v: Val, at: usize) -> Result<Val, MaliError> {
        if !self.is_f16(v) {
            return Ok(v);
        }
        /* fp16 products of texels read at fp32 other than by a highp
         * output's store ([`Fs::widen_store`]): no reference */
        self.mixed |= self.texel16_product(v, 0);
        self.widen_store(v, at)
    }

    /// `v` is fp16 arithmetic on the texels of an fp16 look-up of a highp
    /// shader ([`Fs::alu16`]; mixed_44_hdrsample's `* 0.25`).
    fn texel16_product(&self, v: Val, depth: u8) -> bool {
        let Val::Lane(n, _) = v else { return false };
        if !self.prec.highp {
            return false;
        }
        let nd = self.nir.nodes[n as usize];
        let Kind::Alu(op) = nd.kind else { return false };
        nd.bits == 16
            && depth < 8
            && nd.src[..op.arity()].iter().any(|&s| self.texel16(s) || self.texel16_product(s, depth + 1))
    }

    /// `v` is one fp16 texel by a constant, an operation of one lane
    /// ([`Fs::alu16`]'s `t.a * 0.5`, gm2_texswz_2).
    fn lone_texel_product(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        let nd = self.nir.nodes[n as usize];
        nd.kind == Kind::Alu(Alu::Fmul)
            && nd.bits == 16
            && nd.width == 1
            && matches!((self.texel16(nd.src[0]), nd.src[1]), (true, Val::Const(_)))
    }

    /// `v` is a lane of a sum of two fp16 look-ups' texels taken in pairs
    /// (`FADD.v2f16 r0, r0^, r2^` of each result's words: gm2_texcoord_4,
    /// `texture2D(s0, v.xy) + texture2D(s1, v.zw)` to a mediump colour).
    fn texel_pair_sum(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        let nd = self.nir.nodes[n as usize];
        let pair = |s: Val| match s {
            Val::Lane(p, 0) => matches!(self.nir.nodes[p as usize],
                NirNode { kind: Kind::Alu(Alu::Vec2), bits: 16, width: 2, src: [x, y, _] }
                    if self.texel16(x) && self.texel16(y)),
            _ => false,
        };
        nd.kind == Kind::Alu(Alu::Fadd) && nd.bits == 16 && nd.width == 2 && pair(nd.src[0]) && pair(nd.src[1])
    }

    /// [`Fs::widen`] at the store of a highp output: lower_precision's
    /// `f162f` where the lowered tree meets the assignment
    /// (mixed_44_hdrsample's four `F16_TO_F32` ahead of `BLEND.v4.f32`).
    fn widen_store(&mut self, v: Val, at: usize) -> Result<Val, MaliError> {
        if !self.is_f16(v) {
            return Ok(v);
        }
        let first = self.nir.n;
        let r = self.nir.alu(Alu::F2f32, v, Val::Undef, Val::Undef)?;
        for id in first..self.nir.n {
            self.origin[id] = at as u16;
        }
        Ok(r)
    }

    /// [`Fs::widen`] of an fp16 lane or an fp16 uniform's (`F16_TO_F32
    /// u2.w0.h0`, a 2D array look-up's layer, intbig_case42.expect.json).
    fn widen_any(&mut self, v: Val, at: usize) -> Result<Val, MaliError> {
        let uniform = matches!(v, Val::Lane(n, _)
            if matches!(self.nir.nodes[n as usize], NirNode { kind: Kind::Uniform { .. }, bits: 16, .. }));
        if !uniform {
            return self.widen(v, at);
        }
        let first = self.nir.n;
        let r = self.nir.alu(Alu::F2f32, v, Val::Undef, Val::Undef)?;
        for id in first..self.nir.n {
            self.origin[id] = at as u16;
        }
        Ok(r)
    }

    /// An fp16 node of a highp shader: `op` over `src` at `width` lanes
    /// (2: a vector of two halves, `nir_opt_vectorize`'s `vec2` of 16-bit
    /// lanes), CSE'd (`nir_opt_cse`), made by IR instruction `at`.
    fn node16(&mut self, op: Alu, src: [Val; 3], width: u8, at: usize) -> Result<Val, MaliError> {
        let nd = NirNode { kind: Kind::Alu(op), src, bits: 16, width };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let id = self.nir.push_node(nd)?;
        self.origin[id as usize] = at as u16;
        Ok(Val::Lane(id, 0))
    }

    /// The narrowing (`f2f16`) of the fp32 value `v` a mediump variable
    /// takes, made by IR instruction `at` (the store). Of an integer's
    /// conversion it stands right after the conversion: GLSL's `f2fmp(i2f32
    /// (a))` is one `i2fmp` (nir_opt_algebraic.py:1322) that lowers to
    /// `i2f16`, then to the two instructions in place (deqp_intval_opfs_
    /// mul_ivec4_highp: `S32_TO_F32 r1` then `FADD.f32 r1.h0` for each lane
    /// in turn, not the four conversions then the four narrowings).
    fn narrow_stored(&mut self, v: Val, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let Some((Alu::I2f | Alu::U2f, _)) = self.alu_of(v) else {
            return self.node16(Alu::F2f16, [v, u, u], 1, at);
        };
        let nd = NirNode { kind: Kind::Alu(Alu::F2f16), src: [v, u, u], bits: 16, width: 1 };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let Val::Lane(n, _) = v else { return Err(MaliError::Defect(Defect::IrIndex)) };
        let after = self.nir.order[..self.nir.len].iter().position(|&x| x == n).map(|p| p + 1);
        let id = match after {
            Some(k) => {
                let id = self.nir.insert_before(k, nd)?;
                self.nir.block[id as usize] = self.nir.block[n as usize];
                self.origin[id as usize] = self.origin[n as usize];
                id
            }
            None => {
                let id = self.nir.push_node(nd)?;
                self.origin[id as usize] = at as u16;
                id
            }
        };
        Ok(Val::Lane(id, 0))
    }

    /// The fp32 value of a mediump local's fp16 value `v` that an fp32
    /// operation reads: lower_precision's `f162f` of the assignment
    /// (`atemp = f162f(f2fmp(a) + 0.5)`; a mediump reader's `f2fmp` of it
    /// folds back to `v`), so the `f2f32` comes right after `v` in the
    /// order, CSE'd (deqp_intval_reslib_int_vs: `F16_TO_F32 r0, r0^.h0`
    /// right after the `FADD.v2f16` of `atemp`, before the `S32_TO_F32` of
    /// the compare that reads it). A `v` the order does not hold has no
    /// reference: refused.
    fn widen_local(&mut self, v: Val) -> Result<Val, MaliError> {
        let u = Val::Undef;
        let nd = NirNode { kind: Kind::Alu(Alu::F2f32), src: [v, u, u], bits: 32, width: 1 };
        if let Some(i) = self.nir.nodes[..self.nir.n].iter().position(|x| *x == nd) {
            return Ok(Val::Lane(i as u16, 0));
        }
        let Val::Lane(n, _) = v else { return Err(MaliError::Defect(Defect::IrIndex)) };
        let Some(k) = self.nir.order[..self.nir.len].iter().position(|&x| x == n) else {
            return Err(MaliError::IntValue);
        };
        let id = self.nir.insert_before(k + 1, nd)?;
        self.nir.block[id as usize] = self.nir.block[n as usize];
        self.origin[id as usize] = self.origin[n as usize];
        Ok(Val::Lane(id, 0))
    }

    /// The two halves `x` and `y` as one fp16 vector: the vector they are
    /// lanes 0 and 1 of, or their `vec2` (`MKVEC.v2i16 x.h0, y.h0`,
    /// prec_82.expect.json).
    fn vec16(&mut self, x: Val, y: Val, at: usize) -> Result<Val, MaliError> {
        self.vec16_of(x, y, false, at)
    }

    /// [`Fs::vec16`], taking two texels of an fp16 look-up as a pair when
    /// `texels` (an fp16 product's operand, [`Fs::alu16`]).
    fn vec16_of(&mut self, x: Val, y: Val, texels: bool, at: usize) -> Result<Val, MaliError> {
        if let (Val::Lane(n, 0), Val::Lane(m, 1)) = (x, y) {
            if n == m && self.nir.nodes[n as usize].width == 2 {
                return Ok(Val::Lane(n, 0));
            }
        }
        /* two texels of an fp16 look-up, lanes 2k and 2k + 1: the halves of
         * its result's word k, read whole (the `vec2` copy-propagated,
         * [`select_highp`]; mixed_44_hdrsample's `FMA.v2f16 r0, r0^, ..`) */
        let texels = texels
            && matches!((x, y), (Val::Lane(n, c), Val::Lane(m, d))
            if n == m && c % 2 == 0 && d == c + 1 && self.texel16(x));
        /* a lane and a constant half (`vec2(x, 0.0)`, `MKVEC.v2i16 x.h0, #0`
         * as `U16_TO_U32`; `vec2(0.0, x)`, `MKVEC.v2i16 0x0.h0, x.h0`;
         * `vec2(x, 1.0)`: deqp_intval_opfs_add_int_mediump and
         * opfs_sub_assign_ivec2_lowp's colour lanes beside the constant
         * lanes of `vec4(0.0, 0.0, 0.0, 1.0)`, the constant an fp16 half
         * already, [`Fs::alu16`]) */
        let lane_const = matches!((x, y), (Val::Lane(..), Val::Const(_)) | (Val::Const(_), Val::Lane(..)))
            && (self.is_f16(x) || self.is_f16(y));
        if !texels && !lane_const && (!self.is_f16(x) || !self.is_f16(y)) {
            return Err(MaliError::FragCoordMediump);
        }
        self.node16(Alu::Vec2, [x, y, Val::Undef], 2, at)
    }

    /// A highp shader's IR operation `a` that lower_precision computes at
    /// fp16 (lower_precision.cpp:295-312: every operand is mediump, the
    /// ES 1.00 gl_FragCoord read itself (builtin_variables.cpp:1333-1335)
    /// or an fp16 result of such an operation, or a constant, which takes
    /// the precision of the others; docs/notes/mali-fragcoord-mediump.md
    /// 3). The read is narrowed where the operation reads it
    /// ([`Fs::frag_coord16`]); a constant is converted to fp16 by
    /// `_mesa_float_to_half`, round to nearest even (lower_precision.cpp:
    /// 739-746). `fc` says which sources are the read. The forms the
    /// references draw: one lane's compare (fbload_predif_fs_coord,
    /// fbload_predlive_fs_coord: `FCMP_OR.v2f16.lt.m1 r0^.h00, u3.w0` of
    /// `x < 32.0`) and `floor`, `fract` of the position's x and y
    /// (prec_82: `FROUND.v2f16.rtn` and `FADD.v2f16` of the two halves,
    /// whose `vec2` the late `f2f32(op(f2fmp(a)))` rule, nir_opt_algebraic.py:
    /// 4173-4174, does not see through). Another has no reference.
    fn alu16(&mut self, a: AluOp, s: [[Val; 4]; 3], fc: [bool; 3], w: usize, at: usize) -> Result<[Val; 4], MaliError> {
        let mut x = [[Val::Undef; 4]; 3];
        for k in 0..ir_arity(a) {
            for i in 0..w {
                x[k][i] = match s[k][i] {
                    Val::Const(b) => Val::Const(u32::from(f16_rtne(b).ok_or(MaliError::Fp16Inexact)?)),
                    v if fc[k] => {
                        let [x32, y32, ..] = self.frag_coord(at)?;
                        let [x16, y16, ..] = self.frag_coord16(at)?;
                        if v == x32 {
                            x16
                        } else if v == y32 {
                            y16
                        } else {
                            return Err(MaliError::FragCoordMediump);
                        }
                    }
                    v => v,
                };
            }
        }
        /* the width is the front end's (a built-in records its call's,
         * lower.rs `Sink`), and a swizzle of a narrower value pads it with
         * its last lane: a lane whose operands are the lane before's is the
         * same scalar operation, one node (`nir_opt_cse` after
         * `nir_lower_alu_width`) */
        let same = |i: usize| (0..ir_arity(a)).all(|k| x[k][i] == x[k][i - 1]);
        let d = (1..w).find(|&i| same(i)).unwrap_or(w);
        /* a vector whose lanes merely coincide (dEQP's `vec4(x, 0.0, 0.0,
         * 1.0) * 0.1`: lanes y and z) is the vector it is, split in pairs
         * below */
        let d = if (d..w).any(|i| !same(i)) { w } else { d };
        let (lanes, w) = (w, d);
        let u = Val::Undef;
        let mut out = [u; 4];
        match a {
            /* against a constant, which lower_precision made a 16-bit one:
             * the late `flt(f2fmp(a), f2fmp(b))` -> `flt(a, b)`
             * (nir_opt_algebraic.py:4185-4186) takes a compare of two
             * narrowed values back to fp32, a form no reference draws */
            AluOp::Flt | AluOp::Fle | AluOp::Feq
                if w == 1 && matches!(x[0][0], Val::Const(_)) != matches!(x[1][0], Val::Const(_)) =>
            {
                let (p, q) = (x[0][0], x[1][0]);
                out[0] = match a {
                    AluOp::Flt => self.node16(Alu::Fcmp(Cmpf::Lt), [p, q, u], 1, at)?,
                    AluOp::Fle => self.node16(Alu::Fcmp(Cmpf::Ge), [q, p, u], 1, at)?,
                    _ => self.node16(Alu::Fcmp(Cmpf::Eq), [p, q, u], 1, at)?,
                };
            }
            /* two lanes, one vector operation of halves; `lower_ffract`
             * (nir_opt_algebraic.py:546) is `fadd(a, fneg(ffloor(a)))`, the
             * `ffloor` CSE'd with `floor()`'s. A constant operand has no
             * reference */
            AluOp::Ffloor | AluOp::Ffract | AluOp::Fneg if w == 2 => {
                let v = self.vec16(x[0][0], x[0][1], at)?;
                let r = match a {
                    AluOp::Ffloor => self.node16(Alu::Ffloor, [v, u, u], 2, at)?,
                    AluOp::Fneg => self.node16(Alu::Fneg, [v, u, u], 2, at)?,
                    _ => {
                        let fl = self.node16(Alu::Ffloor, [v, u, u], 2, at)?;
                        let ng = self.node16(Alu::Fneg, [fl, u, u], 2, at)?;
                        self.node16(Alu::Fadd, [v, ng, u], 2, at)?
                    }
                };
                let Val::Lane(n, _) = r else { return Err(MaliError::Defect(Defect::IrIndex)) };
                out[0] = Val::Lane(n, 0);
                out[1] = Val::Lane(n, 1);
            }
            /* a product or sum of the lanes in pairs, each one vector
             * operation of halves (`nir_lower_alu_width` with
             * `bi_vectorize_filter`'s width two): `FMA.v2f16 x, c, -0.0`,
             * `FADD.v2f16 x, c`, the constant one fp16 in both halves
             * (mixed_44_hdrsample's `* 0.25`, `0x34000000.h11`; case 218's
             * `(b + 0.5) / 4.0` of the island's vector, `FADD_IMM.v2f16 r1,
             * r0, #0x38003800`), or a sum of two vectors. A pair whose lanes
             * are a value and a constant is the `vec2` of the two
             * ([`Fs::vec16_of`]); a pair of constants folds, each step
             * rounded to fp16 (`nir_opt_constant_folding` of the `vec2` of
             * constants: deqp_intval_opfs_add_int_mediump's `vec4(float(res),
             * 0.0, 0.0, 1.0) * 0.1 + 0.5`, lanes z and w the constants 0x3800
             * and 0x38cd). Two different constants against a pair, a product
             * of two pairs, or an odd lane count have no reference */
            AluOp::Fmul | AluOp::Fadd if w % 2 == 0 => {
                /* the constant side second (both commute) */
                if matches!(x[0][0], Val::Const(_)) && !matches!(x[1][0], Val::Const(_)) {
                    x.swap(0, 1);
                }
                let konst = |v: Val| matches!(v, Val::Const(_));
                let op = if a == AluOp::Fmul { Alu::Fmul } else { Alu::Fadd };
                for k in (0..w).step_by(2) {
                    let (p, q) = (x[0][k], x[0][k + 1]);
                    let (c, d) = (x[1][k], x[1][k + 1]);
                    if konst(p) && konst(q) && konst(c) && konst(d) {
                        out[k] = Val::Const(fold16(a, p, c)?);
                        out[k + 1] = Val::Const(fold16(a, q, d)?);
                        continue;
                    }
                    let own = |v: Val| self.texel16(v) || self.narrowed(v);
                    let ok = |v: Val| own(v) || konst(v) || self.is_f16(v);
                    if !(own(p) || own(q)) || !ok(p) || !ok(q) {
                        return Err(MaliError::FragCoordMediump);
                    }
                    let v = self.vec16_of(p, q, true, at)?;
                    let kk = match (c, d) {
                        (Val::Const(a), Val::Const(b)) if a == b => Val::Const(a),
                        (Val::Const(_), _) | (_, Val::Const(_)) => return Err(MaliError::FragCoordMediump),
                        /* a second look-up's texels are its words too
                         * (gm2_texcoord_4) */
                        _ if a == AluOp::Fadd => self.vec16_of(c, d, true, at)?,
                        _ => return Err(MaliError::FragCoordMediump),
                    };
                    let Val::Lane(n, _) = self.node16(op, [v, kk, u], 2, at)? else {
                        return Err(MaliError::Defect(Defect::IrIndex));
                    };
                    out[k] = Val::Lane(n, 0);
                    out[k + 1] = Val::Lane(n, 1);
                }
            }
            /* one lane of fp16 arithmetic on the island's narrowed value in a
             * mediump shader (glmark2's `d = fract(3.0 * d)` after `float d =
             * fract(FragCoord.x * FragCoord.y * 0.0001)` of the highp copy):
             * an operation of one lane, its sources in the order written
             * (gm2_cf_call_fs.expect.json: `FMA.v2f16 r0, u3.w1, r0^.h00,
             * 0x0.neg`, the constant 3.0h in both halves of a FAU word;
             * `ffract` as `lower_ffract`'s `fadd(a, fneg(ffloor(a)))`,
             * `FROUND.v2f16.rtn r1, r0.h00`, `FADD.v2f16 r0, r0^.h00,
             * r1^.neg.h00`) */
            AluOp::Fmul | AluOp::Fadd if w == 1 && self.lone16(x[0][0], x[1][0]) => {
                let op = if a == AluOp::Fmul { Alu::Fmul } else { Alu::Fadd };
                out[0] = self.node16(op, [x[0][0], x[1][0], u], 1, at)?;
            }
            AluOp::Ffloor | AluOp::Ffract | AluOp::Fneg if w == 1 && self.lone16(x[0][0], Val::Undef) => {
                let v = x[0][0];
                out[0] = match a {
                    AluOp::Ffloor => self.node16(Alu::Ffloor, [v, u, u], 1, at)?,
                    AluOp::Fneg => self.node16(Alu::Fneg, [v, u, u], 1, at)?,
                    _ => {
                        let fl = self.node16(Alu::Ffloor, [v, u, u], 1, at)?;
                        let ng = self.node16(Alu::Fneg, [fl, u, u], 1, at)?;
                        self.node16(Alu::Fadd, [v, ng, u], 1, at)?
                    }
                };
            }
            /* one texel by a constant, an operation of one lane (gm2_texswz_2's
             * `t.a * 0.5`: `FMA.v2f16 r0, r0^.h00, 0x38000000.h11, 0x0.neg`
             * of the texel the write mask packs alone) */
            AluOp::Fmul if w == 1 => {
                if matches!(x[0][0], Val::Const(_)) {
                    x.swap(0, 1);
                }
                if !self.texel16(x[0][0]) || !matches!(x[1][0], Val::Const(_)) {
                    return Err(MaliError::FragCoordMediump);
                }
                out[0] = self.node16(Alu::Fmul, [x[0][0], x[1][0], u], 1, at)?;
            }
            _ => return Err(MaliError::FragCoordMediump),
        }
        for i in w..lanes {
            out[i] = out[w - 1];
        }
        Ok(out)
    }

    /// Mark a highp operand of the other width: gl_PointCoord is mediump,
    /// and so is a lowp sampler's texel read straight from the look-up (one
    /// stored to a highp local is fp32, [`Fs::look_up`]); a uniform of the
    /// other precision is marked at the compile (mod.rs, from its
    /// declaration).
    fn mark_mixed(&mut self, v: Val) {
        let Val::Lane(n, _) = v else { return };
        self.mixed |= match self.nir.nodes[n as usize].kind {
            Kind::Input { loc, .. } => loc == POINT_COORD,
            Kind::Tex => self.nir.nodes[n as usize].bits == 16,
            _ => false,
        };
    }

    /// `v` is a product or a negated one (`is_fmul`, nir_search_helpers.h).
    fn is_fmul(&self, v: Val) -> bool {
        match self.alu_of(v) {
            Some((Alu::Fmul, _)) => true,
            Some((Alu::Fneg, [inner, ..])) => self.is_fmul(inner),
            _ => false,
        }
    }

    /// `v` is never negative nor -0.0 (nir_range_analysis.c's `fp_class`,
    /// for the shapes atan2 makes): a constant without the sign bit, an
    /// `fabs`, a clamp from 0.0 (`fsat`, `fclamp_pos`), a product or a
    /// select of two such values, a reciprocal of one.
    fn not_negative(&self, v: Val) -> bool {
        match (v, self.alu_of(v)) {
            (Val::Const(c), _) => c & 0x8000_0000 == 0 && f32::from_bits(c) >= 0.0,
            (_, Some((Alu::Fabs | Alu::Fclamp(clamp::ZERO_ONE | clamp::ZERO_INF), ..))) => true,
            (_, Some((Alu::Fmul, [p, q, _]))) => self.not_negative(p) && self.not_negative(q),
            (_, Some((Alu::Bcsel, [_, p, q]))) => self.not_negative(p) && self.not_negative(q),
            (_, Some((Alu::Frcp, [p, ..]))) => self.not_negative(p),
            _ => false,
        }
    }

    /// The pixel position's 16-bit lane `v` is the centre of: `v` is
    /// `fadd(u2f32(p), b)`, `p` a lane of [`Kind::PixelCoord`] and `b` a
    /// constant in (0, 1) ([`Fs::frag_coord`]'s x).
    fn pixel_lane(&self, v: Val) -> Option<Val> {
        let (Alu::Fadd, [s, Val::Const(b), _]) = self.alu_of(v)? else { return None };
        let (Alu::U2f32, [p, ..]) = self.alu_of(s)? else { return None };
        let Val::Lane(n, _) = p else { return None };
        let half = f32::from_bits(b);
        (self.nir.nodes[n as usize].kind == Kind::PixelCoord && half > 0.0 && half < 1.0).then_some(p)
    }

    /// The sum `fadd(x, y)` (IR sources `srcs`) as nir_opt_algebraic.py:2419
    /// takes it: one operand a front-end `ffma(c0, x0, fmul(c1, y1))` read
    /// once, the other `c` no product; `(c0, x0, c1, y1, c)`.
    fn mat_sum(&self, m: &Module<'_>, srcs: &[ir::Src; 3], x: Val, y: Val) -> Option<(Val, Val, Val, Val, Val)> {
        [(0, x, y), (1, y, x)].into_iter().find_map(|(k, t, c)| {
            let (Alu::Ffma, [c0, x0, acc]) = self.alu_of(t)? else { return None };
            let (Alu::Fmul, [c1, y1, _]) = self.alu_of(acc)? else { return None };
            let konst = [c0, x0, c1, y1].iter().any(|v| matches!(v, Val::Const(_)));
            (ir_uses(m, srcs[k].val) == 1 && !konst && !self.is_fmul(c) && !matches!(c, Val::Const(_)))
                .then_some((c0, x0, c1, y1, c))
        })
    }

    /// `v` is a select of constants, or of such selects `depth` deep at
    /// most: a run-time index's read tree over constant elements (glsl.rs
    /// `ilt_trees`, case 80's `arr[i]`).
    fn const_select(&self, v: Val, depth: u8) -> bool {
        match (v, self.alu_of(v)) {
            (Val::Const(_), _) => true,
            (_, Some((Alu::Bcsel, [_, p, q]))) if depth > 0 => {
                self.const_select(p, depth - 1) && self.const_select(q, depth - 1)
            }
            _ => false,
        }
    }

    /// [`Fs::const_select`] `v` times the constant `k`: the selects' tree
    /// over the products of its constants (nir_opt_algebraic.py:1032,
    /// `op(bcsel(a, #b, #c), #d)` to `bcsel(a, op(b, d), op(c, d))`, from
    /// the innermost select out).
    fn scaled_select(&mut self, v: Val, k: u32, at: usize) -> Result<Val, MaliError> {
        match (v, self.alu_of(v)) {
            (Val::Const(c), _) => Ok(Val::Const((f32::from_bits(c) * f32::from_bits(k)).to_bits())),
            (_, Some((Alu::Bcsel, [c, p, q]))) => {
                let p = self.scaled_select(p, k, at)?;
                let q = self.scaled_select(q, k, at)?;
                self.op32(Alu::Bcsel, [c, p, q], at)
            }
            _ => Err(MaliError::Defect(Defect::IrIndex)),
        }
    }

    /// `v` is `fneg(b2f(c))`.
    fn is_neg_b2f(&self, v: Val) -> bool {
        matches!(self.alu_of(v), Some((Alu::Fneg, [inner, ..])) if matches!(self.alu_of(inner), Some((Alu::B2f, _))))
    }

    /// One lane of a highp shader's IR operation, as Mesa's NIR has it
    /// after the lowerings its options ask for (`bifrost_compile.h`
    /// `DEFINE_OPTIONS`, nir_opt_algebraic.py), each named at its arm; `s`
    /// are the lane's sources.
    fn lane32(
        &mut self,
        m: &Module<'_>,
        a: AluOp,
        srcs: &[ir::Src; 3],
        s: [Val; 3],
        at: usize,
    ) -> Result<Val, MaliError> {
        let [x, y, z] = s;
        let u = Val::Undef;
        const F32_ZERO: u32 = 0;
        const F32_ONE: u32 = 0x3f80_0000;
        /* `nir_opt_constant_folding` of the integer arithmetic an unrolled
         * loop's counter leaves (its step, its test, its conversion): the
         * walk folds them so the loop's test is a constant ([`walk_loop`]);
         * a test that also reads the counter (`top > 0.5 && i == 2`,
         * gl_test's databreak) by `iand(a, 0)` and `iand(a, -1)`
         * (nir_opt_algebraic.py:1716-1717) */
        if let Some(v) = nir::fold_int(a, x, y) {
            return Ok(v);
        }
        match a {
            /* `fadd(fadd(a, b), #c)`, the inner sum used once and of no
             * constant, is `fadd(fadd(a, c), b)`, a product kept for `b`
             * (nir_opt_algebraic.py:2399-2400, "Prefer moving out a
             * multiplication", a commutative match: the inner sum's own
             * order first, then swapped). Measured: femtovg's dither,
             * `fract(52.98 * t) - 0.5`, whose lowered `ffract` is `fadd(p,
             * fneg(ffloor(p)))`: `FADD -floor, -0.5` then `FADD .., p`
             * (fvg_fill_gradient*.expect.json) */
            /* a matrix product's lane, the front end's `ffma(c0, x0,
             * fmul(c1, y1))` (GLSL's `fadd(fmul(c0, x0), fmul(c1, y1))`,
             * lower_mat_op_to_vec.cpp:132-150), used once, plus a value that
             * is not a product: `fadd(fadd(a(is_fmul), b(is_fmul)), c)` is
             * `fadd(fadd(a, c), b)` (nir_opt_algebraic.py:2419), which the
             * late fusion makes `ffma(c1, y1, ffma(c0, x0, c))`, the inner
             * term first ([`Fs::alu32`]'s `mat3 * vec3(p, 1.0)` is the same
             * rule). Measured: case 80's `m[1][0] - m[0][0]` of `m *= m`,
             * `FMA.f32 r3, r9^, r6, r3^.neg` then `FMA.f32 r3, r6^, r9^, r3^`
             * (other_80_matdyn.expect.json words 29 and 31) */
            AluOp::Fadd if self.mat_sum(m, srcs, x, y).is_some() => match self.mat_sum(m, srcs, x, y) {
                Some((c0, x0, c1, y1, c)) => {
                    let inner = self.op32(Alu::Ffma, [c0, x0, c], at)?;
                    self.op32(Alu::Ffma, [c1, y1, inner], at)
                }
                None => Err(MaliError::Defect(Defect::IrIndex)),
            },
            AluOp::Fadd => {
                let once = |k: usize| ir_uses(m, srcs[k].val) == 1;
                let inner = match (x, y) {
                    (o, Val::Const(_)) if once(0) => Some((o, y)),
                    (Val::Const(_), o) if once(1) => Some((o, x)),
                    _ => None,
                };
                let sum = inner.and_then(|(o, c)| match self.alu_of(o) {
                    Some((Alu::Fadd, [p, q, _])) if !matches!(p, Val::Const(_)) && !matches!(q, Val::Const(_)) => {
                        Some((p, q, c))
                    }
                    _ => None,
                });
                match sum {
                    Some((p, q, c)) => {
                        let (a, b) = if !self.is_fmul(q) && self.is_fmul(p) { (q, p) } else { (p, q) };
                        let s = self.op32(Alu::Fadd, [a, c, u], at)?;
                        self.op32(Alu::Fadd, [s, b, u], at)
                    }
                    /* `fadd(a, a)` is `fmul(a, 2.0)` (nir_opt_algebraic.py:274),
                     * which fuses with its sum: case 80's `s[1][1]` of `s = m +
                     * m`, `FMA.f32 r3, u3.w1, 0x40000000, r3^`
                     * (other_80_matdyn.expect.json word 43). Not in an
                     * unrolled loop, whose passes' sums of products the
                     * reassociation above folds as they are (s6_discloop's
                     * byte-equal `fmul u_w, 10.0`) */
                    None if x == y && self.loops == 0 => self.op32(Alu::Fmul, [x, Val::Const(F32_TWO), u], at),
                    None => self.op32(Alu::Fadd, [x, y, u], at),
                }
            }
            /* `fmul(fneg(b2f(a)), b)` is `bcsel(a, fneg(b), 0.0)`
             * (nir_opt_algebraic.py:1696), a form no reference draws: refused
             * before the graph's `fmul(fneg(a), b)` rule rewrites it */
            AluOp::Fmul if [x, y].iter().any(|v| self.is_neg_b2f(*v)) => Err(MaliError::AlgebraicForm),
            /* a mediump shader's product of a constant and a boolean's float
             * (case 218's `32.0 * right`, `right = step(32.0, gl_FragCoord.x)`
             * a mediump local): both operands mediump, lower_precision lowers
             * the product to fp16 inside the fp32 tree of the highp
             * gl_FragCoord ([`Fs::fc32`]), where NIR's `fmul(b2f(a), #b)` is
             * `bcsel(a, b, 0.0)` (nir_opt_algebraic.py:1695: the product's one
             * user is the `f2f32` back into the tree, not an `fadd`), and the
             * `f2f32` of a select of two constants is the select of the
             * converted constants (:1022-1032). Measured: `CSEL.u32.eq r3,
             * r2, 0x0, u3.w1, r3^` (fsarith_218_shadowcmp.expect.json, with
             * the `fneg` folded in below). A highp shader's product stays
             * fp32 and, used by an `fadd`, fuses ([`b2f_products`]) */
            AluOp::Fmul
                if !self.prec.highp
                    && [(x, y), (y, x)]
                        .iter()
                        .any(|&(k, b)| matches!(k, Val::Const(_)) && matches!(self.alu_of(b), Some((Alu::B2f, _)))) =>
            {
                let (k, b) = if matches!(x, Val::Const(_)) { (x, y) } else { (y, x) };
                match self.alu_of(b) {
                    Some((Alu::B2f, [c, ..])) => self.op32(Alu::Bcsel, [c, k, Val::Const(0)], at),
                    _ => Err(MaliError::Defect(Defect::IrIndex)),
                }
            }
            /* a select of constants times a constant is the select of the
             * products (nir_opt_algebraic.py:1032, then each enclosing
             * select): case 80's `arr[i] * 0.5` over `(0.125, 0.25, 0.5)` is
             * `bcsel(ilt(i, 1), 0.0625, bcsel(ilt(i, 2), 0.125, 0.25))`,
             * other_80_matdyn.expect.json words 1-5 */
            AluOp::Fmul
                if [(x, y), (y, x)].iter().any(|&(t, k)| {
                    matches!(k, Val::Const(_)) && !matches!(t, Val::Const(_)) && self.const_select(t, 3)
                }) =>
            {
                let (t, k) = if matches!(y, Val::Const(_)) { (x, y) } else { (y, x) };
                match k {
                    Val::Const(k) => self.scaled_select(t, k, at),
                    _ => Err(MaliError::Defect(Defect::IrIndex)),
                }
            }
            /* a constant GLSL IR's `reassociate_constant` never saw, times a
             * product with a constant: `fmul(#a, fmul(b, #c))` is
             * `fmul(fmul(a, c), b)`, the folded constant first
             * (nir_opt_algebraic.py:2406, "Reassociate constants"). Two
             * shapes reach NIR so: in an unrolled loop the counter's
             * constant (computed; measured: s6_discloop's `acc +=
             * vec3(1.0, 2.0, 3.0) * u_w * float(i + 1)`, whose passes end as
             * Mesa's `fmul u_w, 10.0`), and a source constant times a local
             * `opt_tree_grafting` keeps, so the product was another tree
             * (measured: fbload_predlive_fs_coord's `a * 2.0` of `float a =
             * u_q * 3.0`, `fmul 6.0, u_q`). A constant on the right of one
             * tree ends the same when GLSL IR reassociates it
             * (opt_algebraic.cpp:175-217: `(b * #c) * #a` and `(#c * b) *
             * #a` are `(a * c) * b` and `(c * a) * b`) and when it does not
             * (a division, which GLSL IR keeps and NIR's `lower_fdiv` makes
             * a product; measured: prec_82's `p.x * 4.0 / 255.0`, `FMA.f32
             * r1, u3.w0, r1^, 0x0.neg` with `u3.w0` = 4/255). One on the
             * left of one tree GLSL IR makes `b * (a * c)`, the other
             * order: the vertex stage's refusal ([`Nir::alu`]) */
            AluOp::Fmul
                if self.loops > 0 || matches!(y, Val::Const(_)) || [0, 1].iter().any(|&k| kept_local(m, srcs[k])) =>
            {
                /* the counter's conversion (`float(i + 1)`), not a constant
                 * of the source (a constructor's reach through a temporary) */
                let computed = |k: usize| matches!(m.def(srcs[k].val), Some(ir::Op::Alu(..)));
                let local = |k: usize| kept_local(m, srcs[k]);
                let outer = match (x, y) {
                    (Val::Const(c), o) if computed(0) || local(1) => Some((c, o)),
                    (o, Val::Const(c)) => Some((c, o)),
                    _ => None,
                };
                let inner = outer.and_then(|(c2, o)| match (o, self.alu_of(o)) {
                    (
                        Val::Lane(n, _),
                        Some((Alu::Fmul, [Val::Const(c1), b, _]) | (Alu::Fmul, [b, Val::Const(c1), _])),
                    ) => Some((c2, c1, b, n)),
                    _ => None,
                });
                /* `fmul(fmul(a, b), #c)`, the inner product used once and of
                 * no constant, is `fmul(fmul(a, c), b)` (nir_opt_algebraic.py:
                 * 2394, "Reassociate constants"): glmark2's `FragCoord.x *
                 * FragCoord.y * 0.0001` of the island, `FMA.f32 r0, r0^,
                 * u3.w0, 0x0.neg` then `FMA.f32 r0, r0^, r1^, 0x0.neg`
                 * (gm2_cf_call_fs.expect.json). Only on the island
                 * ([`Fs::fc32`]): the highp shaders' products no reference
                 * of this rule draws keep their order */
                let plain =
                    outer.filter(|_| inner.is_none() && matches!(y, Val::Const(_))).and_then(|(c, o)| {
                        match self.alu_of(o) {
                            Some((Alu::Fmul, [p, q, _]))
                                if ir_uses(m, srcs[0].val) == 1
                                    && !matches!(p, Val::Const(_))
                                    && !matches!(q, Val::Const(_))
                                    && self.fc32(o) =>
                            {
                                Some((c, p, q))
                            }
                            _ => None,
                        }
                    });
                match (inner, plain) {
                    (Some((c2, c1, b, n)), _) => {
                        let saved = self.nir.nonuniform_const;
                        self.nir.nonuniform_const |= self.nir.nonuniform[n as usize];
                        let folded = self.op32(Alu::Fmul, [Val::Const(c2), Val::Const(c1), u], at);
                        let r = folded.and_then(|c| self.op32(Alu::Fmul, [c, b, u], at));
                        self.nir.nonuniform_const = saved;
                        r
                    }
                    (None, Some((c, p, q))) => {
                        let pc = self.op32(Alu::Fmul, [p, Val::Const(c), u], at)?;
                        self.op32(Alu::Fmul, [pc, q, u], at)
                    }
                    (None, None) => self.op32(Alu::Fmul, [x, y, u], at),
                }
            }
            AluOp::Fmul => self.op32(Alu::Fmul, [x, y, u], at),
            /* `fneg` of a select of two constants used once is the select of
             * the negated constants (nir_opt_algebraic.py:1126): case 218's
             * `x - 32.0 * right` is `fadd(x, bcsel(c, -32.0, -0.0))`, `CSEL
             * .u32.eq r3, r2, 0x0, u3.w1, r3^` of `0x80000000` and
             * `0xC2000000` then `FADD.f32 r0, r0^, r3^` */
            AluOp::Fneg
                if ir_uses(m, srcs[0].val) == 1
                    && matches!(self.alu_of(x), Some((Alu::Bcsel, [_, Val::Const(_), Val::Const(_)]))) =>
            {
                match self.alu_of(x) {
                    Some((Alu::Bcsel, [c, Val::Const(p), Val::Const(q)])) => {
                        self.op32(Alu::Bcsel, [c, Val::Const(p ^ F32_NEG_ZERO), Val::Const(q ^ F32_NEG_ZERO)], at)
                    }
                    _ => Err(MaliError::Defect(Defect::IrIndex)),
                }
            }
            AluOp::Fneg => self.op32(Alu::Fneg, [x, u, u], at),
            /* `fabs(a)` of a value range analysis finds not negative is `a`
             * (nir_opt_algebraic.py:2020; measured: atan2's `|tan|` and
             * `|u|`, a select of `1.0` and a product of two `fabs`, and its
             * reciprocal: `FCMP.ge 1.0, tan` and `FMA u, ..` with no `.abs`,
             * fvg_fill_gradient_conic*.expect.json) */
            AluOp::Fabs if self.not_negative(x) => Ok(x),
            AluOp::Fabs => self.op32(Alu::Fabs, [x, u, u], at),
            /* `fmax(a, 0.0)` is `fclamp_pos(a)` (bifrost_nir_algebraic.py:34),
             * `FCLAMP .clamp_0_inf`: measured on femtovg's `max(d, 0.0)`
             * (fvg_fill_color_nodiscard.expect.json, `FADD.f32.clamp_0_inf`
             * where the producer has another use) */
            AluOp::Fmax if y == Val::Const(F32_ZERO) => self.op32(Alu::Fclamp(clamp::ZERO_INF), [x, u, u], at),
            AluOp::Fmax if x == Val::Const(F32_ZERO) => self.op32(Alu::Fclamp(clamp::ZERO_INF), [y, u, u], at),
            AluOp::Fmax => self.op32(Alu::Fmax, [x, y, u], at),
            /* `fmin(fmax(a, 0.0), 1.0)` is `fsat(a)` (nir_opt_algebraic.py:1088;
             * both commutative, so in either order) */
            AluOp::Fmin => {
                /* the `fmax(a, 0.0)` is already the `fclamp_pos` node the
                 * Fmax arm made (NIR makes `fsat` first, in the loop, and
                 * `fclamp_pos` of what is left, late) */
                let sat = |f: &Self, p: Val, q: Val| match (f.alu_of(p), q) {
                    (Some((Alu::Fclamp(clamp::ZERO_INF), [a, ..])), Val::Const(F32_ONE)) => Some(a),
                    _ => None,
                };
                match sat(self, x, y).or_else(|| sat(self, y, x)) {
                    Some(a) => self.op32(Alu::Fclamp(clamp::ZERO_ONE), [a, u, u], at),
                    None => self.op32(Alu::Fmin, [x, y, u], at),
                }
            }
            /* `a / b` is `a * rcp(b)` from the front end */
            AluOp::Frcp => self.op32(Alu::Frcp, [x, u, u], at),
            /* `lower_fsqrt`: `frcp(frsq(a))` (nir_opt_algebraic.py:1908) */
            AluOp::Fsqrt => {
                let r = self.op32(Alu::Frsq, [x, u, u], at)?;
                self.op32(Alu::Frcp, [r, u, u], at)
            }
            AluOp::Frsq => self.op32(Alu::Frsq, [x, u, u], at),
            AluOp::Ffloor => self.op32(Alu::Ffloor, [x, u, u], at),
            AluOp::Fceil => self.op32(Alu::Fceil, [x, u, u], at),
            /* `lower_ffract` (arch < 11): `fsub(a, ffloor(a))` (:546) */
            AluOp::Ffract => {
                let fl = self.op32(Alu::Ffloor, [x, u, u], at)?;
                let n = self.op32(Alu::Fneg, [fl, u, u], at)?;
                self.op32(Alu::Fadd, [x, n, u], at)
            }
            /* `lower_fsign`: `fsub(b2f(flt(0.0, a)), b2f(flt(a, 0.0)))` (:2818) */
            AluOp::Fsign => {
                let p = self.op32(Alu::Fcmp(Cmpf::Lt), [Val::Const(F32_ZERO), x, u], at)?;
                let p = self.op32(Alu::B2f, [p, u, u], at)?;
                let q = self.op32(Alu::Fcmp(Cmpf::Lt), [x, Val::Const(F32_ZERO), u], at)?;
                let q = self.op32(Alu::B2f, [q, u, u], at)?;
                let n = self.op32(Alu::Fneg, [q, u, u], at)?;
                self.op32(Alu::Fadd, [p, n, u], at)
            }
            /* `flt(#a, b2f(c))` of a constant in (0, 1) is `c`: bifrost
             * lowers `b2f` to `bcsel_pan(c, 1.0, 0.0)` (bifrost_nir_algebraic.py:
             * 128), NIR moves the compare into the select's constants
             * (nir_opt_algebraic.py:3415-3432, `flt(bcsel(a, #b, #c), #d)`
             * to `bcsel(a, flt(b, d), flt(c, d))`, and the mirrored form),
             * folds them and makes `bcsel(c, true, false)` `c`. Measured:
             * case 218's `right > 0.5` of `right = step(32.0, gl_FragCoord.x)`
             * tests `step`'s own `FCMP_OR.f32.ge.m1` mask
             * (fsarith_218_shadowcmp.expect.json) */
            AluOp::Flt
                if matches!(x, Val::Const(k) if f32::from_bits(k) > 0.0 && f32::from_bits(k) < 1.0)
                    && matches!(self.alu_of(y), Some((Alu::B2f, _))) =>
            {
                match self.alu_of(y) {
                    Some((Alu::B2f, [c, ..])) => Ok(c),
                    _ => Err(MaliError::Defect(Defect::IrIndex)),
                }
            }
            AluOp::Flt => self.op32(Alu::Fcmp(Cmpf::Lt), [x, y, u], at),
            /* GLSL's `<=` is `fge` with its sources swapped (glsl_to_nir.cpp:2488) */
            AluOp::Fle => self.op32(Alu::Fcmp(Cmpf::Ge), [y, x, u], at),
            AluOp::Feq => self.op32(Alu::Fcmp(Cmpf::Eq), [x, y, u], at),
            AluOp::Ilt => self.op32(Alu::Icmp(Cmpf::Lt), [x, y, u], at),
            AluOp::Ile => self.op32(Alu::Icmp(Cmpf::Ge), [y, x, u], at),
            /* `ieq_pan` (femtovg's `texType == 1`); measured:
             * fvg_texture_copy_unclipped.expect.json's `ICMP_OR.u32.eq.m1` */
            /* `ieq(b, true)` of a boolean is `b` (nir_opt_algebraic.py),
             * the mediump arm's rule ([`Fs::int_lane`]): dEQP's `isOk(a,
             * b)` of a `true` constant and a `bool` uniform is the
             * uniform's word (deqp_misc_cmp_notequal_vec2, no compare) */
            AluOp::Ieq if x == Val::Const(u32::MAX) && bool_uniform32(m, srcs[1]) => Ok(y),
            AluOp::Ieq if y == Val::Const(u32::MAX) && bool_uniform32(m, srcs[0]) => Ok(x),
            AluOp::Ieq => self.op32(Alu::Icmp(Cmpf::Eq), [x, y, u], at),
            AluOp::B2f => self.op32(Alu::B2f, [x, u, u], at),
            /* a `bool` uniform's `i2b` is its word itself: GL stores true
             * as ~0 (`UniformBooleanTrue`), and Mesa's NIR reads the 32-bit
             * word as the boolean, no compare (deqp_misc_cmp_notequal_vec2:
             * `MOV.i32 r0, u2.w0`, then the `MUX.v2i16` of its `b2f16`),
             * as the mediump arm does ([`Fs::int_lane`]) */
            AluOp::I2b if bool_uniform32(m, srcs[0]) => Ok(x),
            /* `bcsel(a, 1.0, 0.0)` is `b2f(a)` (nir_opt_algebraic.py:2067),
             * so the colour's conversion is `b2f16` (measured: gl_test's
             * databreak, `top > 0.5 ? 1.0 : 0.0`, `MUX.v2i16 r0, 0x0,
             * 0x3C000000.h11, r0^.h00` in case87_databreak.expect.json) */
            AluOp::Bcsel if y == Val::Const(F32_ONE) && z == Val::Const(F32_ZERO) => self.op32(Alu::B2f, [x, u, u], at),
            /* `bcsel(a, b, b)` is `b` (nir_opt_algebraic.py): builtin_98's
             * alpha `cond ? 1.0 : 1.0` */
            AluOp::Bcsel if y == z => Ok(y),
            AluOp::Bcsel => self.op32(Alu::Bcsel, [x, y, z], at),
            /* `f2i(fadd(a, b))` of an integral, non-negative `a` and a `b` in
             * (0, 1) is `f2i(a)`, and `f2i32(u2f32(a@16))` is `u2u32(a)`
             * (nir_opt_algebraic.py:1955-1958): `int(gl_FragCoord.x)` is the
             * pixel's own x, `U16_TO_U32 r1, r59^.h0`
             * (fbload_fbload.expect.json) */
            AluOp::F2i => match self.pixel_lane(x) {
                Some(px) => self.op32(Alu::U2u32, [px, u, u], at),
                None => self.op32(Alu::F2i, [x, u, u], at),
            },
            /* the same rules of `f2u` (:1955, :1957): `uint(gl_FragCoord.x)`
             * is `U16_TO_U32 r3, r59^.h0` (intuni_intrt.expect.json) */
            AluOp::F2u => match self.pixel_lane(x) {
                Some(px) => self.op32(Alu::U2u32, [px, u, u], at),
                None => self.op32(Alu::F2u, [x, u, u], at),
            },
            /* `i2f32(iand(a, 0xff))` and `i2f32(iand(ishr(a, 8k), 0xff))`
             * are `u2f32(extract_u8(a, k))` (nir_opt_algebraic.py's
             * extract rules), `U8_TO_F32 a.bk` (intuni_ishr.expect.json) */
            AluOp::I2f => {
                let byte = match self.alu_of(x) {
                    Some((Alu::Iand, [a, Val::Const(0xff), _] | [Val::Const(0xff), a, _])) => match self.alu_of(a) {
                        Some((Alu::Ishr | Alu::Ushr, [b, Val::Const(s @ (8 | 16 | 24)), _])) => Some((b, s / 8)),
                        _ => Some((a, 0)),
                    },
                    _ => None,
                };
                match (byte, self.alu_of(x)) {
                    (Some((b, k)), _) => self.op32(Alu::U8f32(k as u8), [b, u, u], at),
                    /* `i2f32(f2i32(a))` is `ftrunc(a)` (nir_opt_algebraic.py:
                     * 1250): intloop_loop_exit's `float(n)` of `int n =
                     * int(x)`, `FROUND.f32.rtz` of the product */
                    (None, Some((Alu::F2i, [a, ..]))) => self.op32(Alu::Ftrunc, [a, u, u], at),
                    (None, _) => self.op32(Alu::I2f, [x, u, u], at),
                }
            }
            /* `u2f32` of a smooth lane: case 199's `float(u >> 2u)`, which
             * the link moves into the vertex shader (fsarith_199_es3trunc,
             * its varying program's `U32_TO_F32`) */
            AluOp::U2f => self.op32(Alu::U2f, [x, u, u], at),
            /* GLSL's `a - b` is `iadd(a, ineg(b))`, which is `isub(a, b)`
             * (nir_opt_algebraic.py, as the vertex stage's arm, nir.rs);
             * the `ineg` is left to the colour's reach: case 199's
             * `(i >> 1) - (i << 3)`, `ISUB.s32` (fsarith_199_es3trunc) */
            AluOp::Iadd => match (self.alu_of(x), self.alu_of(y)) {
                (_, Some((Alu::Ineg, [b, ..]))) => self.op32(Alu::Isub, [x, b, u], at),
                (Some((Alu::Ineg, [b, ..])), _) => self.op32(Alu::Isub, [y, b, u], at),
                _ => self.op32(Alu::Iadd, [x, y, u], at),
            },
            AluOp::Ineg => self.op32(Alu::Ineg, [x, u, u], at),
            AluOp::Iand => self.op32(Alu::Iand, [x, y, u], at),
            AluOp::Ishl => self.op32(Alu::Ishl, [x, y, u], at),
            AluOp::Ishr => self.op32(Alu::Ishr, [x, y, u], at),
            AluOp::Ushr => self.op32(Alu::Ushr, [x, y, u], at),
            /* `trunc()`, `FROUND.f32.rtz`; of `-v.x` the `fneg` node stays,
             * a source modifier (fsarith_199_es3trunc's `FROUND.f32.rtz r1,
             * r2^.neg`) */
            AluOp::Ftrunc => self.op32(Alu::Ftrunc, [x, u, u], at),
            /* every front-end ffma is `FMA.f32` with its sources in order: a
             * matrix product's term (the vertex stage's rule, here also of a
             * mat3 built from a vec4 array's elements, femtovg's
             * `scissorMat`; a last column scaled by the constant 1.0 is
             * [`Fs::alu32`]'s) and a built-in's, `mix()` (nir_lower_flrp)
             * and atan's polynomial (fvg_fill_gradient*.expect.json) */
            /* `ffma(k, fneg(b), c)` of a constant `k`: NIR moves the fneg
             * out of the product and folds it into the constant
             * (`fmul(a, fneg(b))` is `fneg(fmul(a, b))`, the late fusion's
             * `ffma(fneg(a), b, c)`): `mod(n, 16.0)`'s `FMA.f32 r4,
             * u3.w0, r4^, r3` of -16.0 (builtin_98.expect.json) */
            AluOp::Ffma => match (x, self.alu_of(y)) {
                (Val::Const(k), Some((Alu::Fneg, [b, ..]))) => {
                    self.op32(Alu::Ffma, [Val::Const(k ^ F32_NEG_ZERO), b, z], at)
                }
                _ => self.op32(Alu::Ffma, [x, y, z], at),
            },
            /* `!=`: `fneu_pan`/`ine_pan`, the operands in order as `Feq`/`Ieq`,
             * `FCMP_OR.f32.ne.m1` and `ICMP_OR.u32.ne.m1`
             * (deqp_misc_cmp_fne_hp, deqp_misc_cmp_ine0_hp) */
            AluOp::Fne => self.op32(Alu::Fcmp(Cmpf::Ne), [x, y, u], at),
            AluOp::Ine => self.op32(Alu::Icmp(Cmpf::Ne), [x, y, u], at),
            /* `inot` of a compare used once is the inverse compare
             * (nir_opt_algebraic.py: `inot(flt(a, b))` is `fge(a, b)`):
             * `step(e, x)`'s `!(x < e)` is `fge_pan x, e`, `FCMP_OR.f32.ge.m1`
             * (case 218, fsarith_218_shadowcmp.expect.json) */
            AluOp::Bnot | AluOp::Inot if ir_uses(m, srcs[0].val) == 1 => self.inverse_compare(x),
            /* the masks of a run-time index's store tree (glsl.rs
             * `ilt_trees`): `iand(d2, inot(d1))` and `ior(d1, d2)` of a
             * compare used more than once, which nir_opt_algebraic.py:742
             * leaves (other_80_matdyn.expect.json words 8-12) */
            AluOp::Inot => self.op32(Alu::Inot, [x, u, u], at),
            AluOp::Ior => self.op32(Alu::Ior, [x, y, u], at),
            /* `pow(a, #b)`, the front end's `exp2(b * log2(a))`
             * (nir_opt_algebraic.py:1872's `fpow`, `bi_fpow_32`; gm2_fsarith_2:
             * `FREXPE.f32.log` .. `FMA_RSCALE.f32 r0, u11.w1, r0^, 0x0.neg,
             * r2^`): the `log2` a node the `pow` reads, left to the dead
             * code; `log2` and `exp2` alone have no highp reference */
            AluOp::Flog2 => self.op32(Alu::Flog2, [x, u, u], at),
            AluOp::Fexp2 => match self.pow32_of(x) {
                Some((a, b)) if !matches!(f32::from_bits(b), 1.0 | 2.0 | 3.0 | 4.0) => {
                    self.op32(Alu::Fpow, [a, Val::Const(b), u], at)
                }
                /* `fpow(a, 2.0)` is `fmul(a, a)` (nir_opt_algebraic.py:1883;
                 * gm2_fclane_2's `pow(vd.r, 2.0)`, `FMA.f32 r0, r0^, r0^, r4^`
                 * with the sum it is read by, byte-equal to gm2_fclane_3's
                 * `vd.r * vd.r`); 1.0, 3.0 and 4.0 have no reference */
                Some((a, b)) if f32::from_bits(b) == 2.0 => self.op32(Alu::Fmul, [a, a, u], at),
                _ => Err(MaliError::Arithmetic),
            },
            _ => Err(MaliError::Arithmetic),
        }
    }

    /// `v` is the fp32 `fmul(flog2(a), #b)` (either order): Some((a, b))
    /// ([`Fs::pow_of`]'s at fp32).
    fn pow32_of(&self, v: Val) -> Option<(Val, u32)> {
        let Val::Lane(n, _) = v else { return None };
        let nd = self.nir.nodes[n as usize];
        let (Kind::Alu(Alu::Fmul), 32) = (nd.kind, nd.bits) else { return None };
        let (l, b) = match (nd.src[0], nd.src[1]) {
            (l, Val::Const(b)) | (Val::Const(b), l) => (l, b),
            _ => return None,
        };
        let Val::Lane(m, _) = l else { return None };
        match self.nir.nodes[m as usize] {
            NirNode { kind: Kind::Alu(Alu::Flog2), bits: 32, src: [a, ..], .. } => Some((a, b)),
            _ => None,
        }
    }

    /// A highp shader's IR operation `a` over its lanes (`nir_lower_alu_width`
    /// at width 1: one scalar node a lane; a dot product as the vertex
    /// stage's chain, nir.rs). A vector constant source whose lanes differ
    /// (`vec3(1.0, 2.0, 3.0) * u`) is marked for the rules on one value,
    /// which NIR matches against the vector whole
    /// ([`Nir::nonuniform_const`]).
    fn alu32(
        &mut self,
        m: &Module<'_>,
        a: AluOp,
        srcs: &[ir::Src; 3],
        vals: &[[Val; 4]],
        id: usize,
    ) -> Result<[Val; 4], MaliError> {
        let w = width_of(m, ir::ValueId(id as u32));
        /* a matrix times a constant vector: Mesa's sums of the scalar
         * terms, as its rules leave them ([`MatSum`]) */
        if matches!(a, AluOp::Fmul | AluOp::Ffma) {
            match const_mat_chain(m, vals, id) {
                ChainAt::Inner => return Ok([Val::Undef; 4]),
                ChainAt::Top(terms, n) => return self.const_mat_product(&terms[..n], vals, w, id),
                ChainAt::None => {}
            }
        }
        /* by the lanes' values: a constructor's constants reach the
         * operation through a temporary (`order_keys`), not a `ConstF32` */
        let lane = |k: usize, i: usize| -> Val {
            vals.get(srcs[k].val.0 as usize).map_or(Val::Undef, |v| v[srcs[k].swizzle[i] as usize & 3])
        };
        let nonuniform = (0..ir_arity(a)).any(|k| {
            let first = lane(k, 0);
            matches!(first, Val::Const(_))
                && (1..w.min(4)).any(|i| matches!(lane(k, i), Val::Const(_)) && lane(k, i) != first)
        });
        /* atan2's last select, `bcsel(flt(fmin(y, rcp), 0.0), fneg(arc),
         * arc)`: Mesa builds its arguments right to left, the `fneg` before
         * the `fmin` (nir_builtin_builder.c `nir_atan2`; measured:
         * fvg_fill_gradient_conic*.expect.json, `FADD -arc` then `FMIN`),
         * the front end the other way round. The `fneg` is made here, and
         * found again (CSE) where its IR instruction stands */
        if let Some((neg, arc)) = atan2_fneg(m, a, id) {
            let wn = width_of(m, neg);
            for i in 0..wn.min(4) {
                let v = vals.get(arc.val.0 as usize).map_or(Val::Undef, |v| v[arc.swizzle[i] as usize & 3]);
                self.op32(Alu::Fneg, [v, Val::Undef, Val::Undef], id)?;
            }
        }
        let saved = self.nir.nonuniform_const;
        self.nir.nonuniform_const = nonuniform;
        let r = self.alu32_lanes(m, a, srcs, vals, id, w);
        self.nir.nonuniform_const = saved;
        r
    }

    /// `m * v` by a constant `v` ([`const_mat_chain`]'s `terms`, column j
    /// and its scale): each lane's sum of terms simplified as NIR's rules
    /// leave it ([`MatSum`]), then built in its instructions' order, lane
    /// after lane, for the late fusion to fuse (algfold_189_half: `fmul(0.5,
    /// fadd(b, fadd(q, a)))`, `ffma(fneg(q), 0.5, a)`, `fmul(0.5, fadd(q,
    /// x))`; algfold_189_mulone: `ffma(a, 0.5, ffma(q, 0.5, b))`,
    /// `fadd(fneg(q), a)`, `ffma(q, 0.5, x)`).
    fn const_mat_product(
        &mut self,
        terms: &[(ir::Src, u32)],
        vals: &[[Val; 4]],
        w: usize,
        id: usize,
    ) -> Result<[Val; 4], MaliError> {
        let col_lane = |col: ir::Src, i: usize| {
            vals.get(col.val.0 as usize).map_or(Val::Undef, |v| v[col.swizzle[i] as usize & 3])
        };
        /* the columns' lanes (the front end's width may be the vector's) */
        let rows = (0..w.min(4)).take_while(|&i| terms.iter().all(|&(c, _)| col_lane(c, i) != Val::Undef)).count();
        let mut out = [Val::Undef; 4];
        for (i, o) in out.iter_mut().enumerate().take(rows) {
            let mut s = MatSum::new();
            let mut acc: Option<u8> = None;
            for &(col, k) in terms {
                let c = col_lane(col, i);
                let leaf = match self.alu_of(c) {
                    Some((Alu::Fneg, [x, ..])) => {
                        let fm = matches!(self.alu_of(x), Some((Alu::Fmul, _)));
                        let l = s.leaf(x, fm)?;
                        s.mk(Term::Neg(l))?
                    }
                    found => s.leaf(c, matches!(found, Some((Alu::Fmul, _))))?,
                };
                /* a term by 1.0 is the column (GLSL's `is_vec_one`) */
                let t = if k == F32_ONE {
                    leaf
                } else {
                    let kk = s.leaf(Val::Const(k), false)?;
                    s.mk(Term::Mul(leaf, kk))?
                };
                acc = Some(match acc {
                    None => t,
                    Some(x) => s.mk(Term::Add(x, t))?,
                });
            }
            let root = acc.ok_or(MaliError::Defect(Defect::IrIndex))?;
            let root = s.simplify(root)?;
            *o = self.emit_term(&s, root, id)?;
        }
        Ok(out)
    }

    /// Term `i` of `s` as nodes, its sources first.
    fn emit_term(&mut self, s: &MatSum, i: u8, at: usize) -> Result<Val, MaliError> {
        let u = Val::Undef;
        match s.t[i as usize] {
            Term::Leaf(v) => Ok(v),
            Term::Neg(x) => {
                let x = self.emit_term(s, x, at)?;
                self.op32(Alu::Fneg, [x, u, u], at)
            }
            Term::Add(x, y) | Term::Mul(x, y) => {
                let op = if matches!(s.t[i as usize], Term::Add(..)) { Alu::Fadd } else { Alu::Fmul };
                let x = self.emit_term(s, x, at)?;
                let y = self.emit_term(s, y, at)?;
                self.op32(op, [x, y, u], at)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn alu32_lanes(
        &mut self,
        m: &Module<'_>,
        a: AluOp,
        srcs: &[ir::Src; 3],
        vals: &[[Val; 4]],
        id: usize,
        w: usize,
    ) -> Result<[Val; 4], MaliError> {
        let lane = |k: usize, i: usize| -> Val {
            vals.get(srcs[k].val.0 as usize).map_or(Val::Undef, |v| v[srcs[k].swizzle[i] as usize & 3])
        };
        let mut out = [Val::Undef; 4];
        if matches!(a, AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4) {
            let n = match a {
                AluOp::Fdot2 => 2,
                AluOp::Fdot3 => 3,
                _ => 4,
            };
            let u = Val::Undef;
            let mut acc = self.op32(Alu::Fmul, [lane(0, n - 1), lane(1, n - 1), u], id)?;
            for i in (0..n - 1).rev() {
                acc = self.op32(Alu::Ffma, [lane(0, i), lane(1, i), acc], id)?;
            }
            return Ok([acc; 4]);
        }
        /* `mat3 * vec3(p, 1.0)` (femtovg's `scissorMat`, `paintMat`): the
         * last column's product by 1.0 folds in GLSL's front end, so NIR
         * holds `fadd(fadd(fmul(m0, x), fmul(m1, y)), m2)`, which
         * nir_opt_algebraic reassociates to `fadd(fadd(fmul(m0, x), m2),
         * fmul(m1, y))` (nir_opt_algebraic.py:2419: a single-use sum of two
         * products plus a non-product) and the late fusion makes
         * `ffma(m1, y, ffma(m0, x, m2))`, the inner term of every lane
         * before the outer ones (`nir_lower_alu_width` splits one vector
         * operation at a time). Measured: fvg_fill_color_nodiscard.expect.json
         * (`FMA m0.x, p.x, m2.x`, `FMA m0.y, p.y, m2.y`, then `FMA m1.x,
         * p.y, ..`). The front end's chain is `ffma(m2, 1.0, ffma(m0, x,
         * fmul(m1, y)))`; any other shape under a constant scale has no
         * reference ([`MaliError::ArithConstant`] by `Nir::alu`). */
        if a == AluOp::Ffma && is_mat_term_fs(m, srcs) && lane(1, 0) == Val::Const(F32_ONE) {
            /* `mat4 * vec4(v, 1.0)`: `fadd(fadd(fadd(fmul(m0, x), fmul(m1,
             * y)), fmul(m2, z)), m3)`, which `reassoc_fma_optimizations`
             * (nir_opt_algebraic.py:4355-4356) makes `fadd(m2 z, fadd(m0 x,
             * fadd(m1 y, m3)))`, fused: `ffma(m2, z, ffma(m0, x, ffma(m1, y,
             * m3)))` (gm2_fsarith_2: `FMA.f32 r3, u2.w0, r1, u2.w1`, then
             * `u8.w0, r0, r3^` and `u9.w1, r2, r3^`; gm2_fsarith_1 the same
             * at fp16). The front end's chain is `ffma(m3, 1.0, ffma(m2, z,
             * ffma(m0, x, fmul(m1, y))))` */
            let three = |f: &Self, v: Val| match f.alu_of(v) {
                Some((Alu::Ffma, [c2, z2, acc])) => match f.alu_of(acc) {
                    Some((Alu::Ffma, [c0, x0, acc])) => match f.alu_of(acc) {
                        Some((Alu::Fmul, [c1, y1, _])) => Some([(c1, y1), (c0, x0), (c2, z2)]),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };
            if (0..w).all(|i| three(self, lane(2, i)).is_some()) {
                let mut terms = [[(Val::Undef, Val::Undef); 3]; 4];
                for (i, t) in terms.iter_mut().enumerate().take(w) {
                    *t = three(self, lane(2, i)).ok_or(MaliError::Defect(Defect::IrIndex))?;
                }
                let mut acc = [Val::Undef; 4];
                for (i, a) in acc.iter_mut().enumerate().take(w) {
                    *a = lane(0, i);
                }
                /* each lane's chain whole, x first (the order Mesa's program
                 * has; the scheduler keeps it) */
                for i in 0..w {
                    for t in terms[i] {
                        acc[i] = self.op32(Alu::Ffma, [t.0, t.1, acc[i]], id)?;
                    }
                }
                return Ok(acc);
            }
            let mut inner = [Val::Undef; 4];
            let mut outer = [(Val::Undef, Val::Undef); 4];
            for i in 0..w {
                let chain = match self.alu_of(lane(2, i)) {
                    Some((Alu::Ffma, [c0, x0, acc])) => match self.alu_of(acc) {
                        Some((Alu::Fmul, [c1, y1, _])) => Some((c0, x0, c1, y1)),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((c0, x0, c1, y1)) = chain else { return Err(MaliError::ArithConstant) };
                inner[i] = self.op32(Alu::Ffma, [c0, x0, lane(0, i)], id)?;
                outer[i] = (c1, y1);
            }
            for i in 0..w {
                out[i] = self.op32(Alu::Ffma, [outer[i].0, outer[i].1, inner[i]], id)?;
            }
            return Ok(out);
        }
        /* any other front-end `ffma` (a matrix product's term) by a
         * constant 1.0 or -1.0: a product by a constant vector is
         * [`Fs::const_mat_product`]'s (algfold_189_mulone.expect.json); a
         * scale of -1.0, or 1.0 beside a scale that is not a constant, in
         * any other shape has no reference. A dot product's own `ffma`s are
         * made here and fold (`Nir::alu`) */
        if a == AluOp::Ffma
            && (0..w).any(|i| [lane(0, i), lane(1, i)].iter().any(|v| matches!(v, Val::Const(F32_ONE | F32_MINUS_ONE))))
        {
            return Err(MaliError::ArithConstant);
        }
        /* atan's sign fix-up: the front end's `fabs(t) * fsign(x)`
         * (builtin_call.rs `atan`, whose `t` is `ffma(fabs(u), res,
         * bias)`) is Mesa's `nir_copysign(t, x)` (nir_builtin_builder.h:
         * `ior(iand(t, 0x7fffffff), iand(x, 0x80000000))`, its arguments
         * built right to left: the sign's mask first). Measured:
         * fvg_fill_gradient_conic*.expect.json, `LSHIFT_AND x, 0x80000000`,
         * `LSHIFT_AND t, 0x7FFFFFFF`, `LSHIFT_OR` */
        if let Some((t, x)) = atan_sign(m, a, srcs) {
            let u = Val::Undef;
            let at = |s: ir::Src, c: u8| {
                vals.get(s.val.0 as usize).map_or(Val::Undef, |v| v[s.swizzle[c as usize & 3] as usize & 3])
            };
            for i in 0..w {
                let sign = self.op32(Alu::Iand, [at(x, srcs[1].swizzle[i]), Val::Const(0x8000_0000), u], id)?;
                let mag = self.op32(Alu::Iand, [at(t, srcs[0].swizzle[i]), Val::Const(0x7fff_ffff), u], id)?;
                out[i] = self.op32(Alu::Ior, [mag, sign, u], id)?;
            }
            return Ok(out);
        }
        /* a signed divide by a positive constant power of two `2^k`:
         * `imul(isign(a), ushr(iabs(a), k))` (nir_opt_algebraic.py:208,
         * the constant `find_lsb(b)` folded), whose `isign` is `imin(imax(a,
         * -1), 1)` (:2812, `lower_isign`, bifrost_compile.h:89). The rule
         * rewrites the vector operation, building its replacement source
         * first, and `nir_lower_alu_width` splits each vector operation in
         * place: every lane's `imax`, then every lane's `imin`, and so on
         * (fbload_isamp.expect.json: `CSEL.s32.gt` x2, `CSEL.s32.lt` x2,
         * `IABS.s32` x2, `RSHIFT_OR.i32` x2, `IMUL.i32` x2). Another
         * divisor has no reference */
        if a == AluOp::Idiv {
            let k = match lane(1, 0) {
                Val::Const(d) if d > 1 && d.is_power_of_two() && d < 1 << 31 => d.trailing_zeros(),
                _ => return Err(MaliError::Arithmetic),
            };
            let n = (0..w).take_while(|&i| lane(0, i) != Val::Undef).count();
            if (0..n).any(|i| lane(1, i) != lane(1, 0)) {
                return Err(MaliError::Arithmetic);
            }
            let u = Val::Undef;
            let mut sign = [u; 4];
            let mut mag = [u; 4];
            for (i, s) in sign.iter_mut().enumerate().take(n) {
                *s = self.op32(Alu::Imax, [lane(0, i), Val::Const(u32::MAX), u], id)?;
            }
            for s in sign.iter_mut().take(n) {
                *s = self.op32(Alu::Imin, [*s, Val::Const(1), u], id)?;
            }
            for (i, m) in mag.iter_mut().enumerate().take(n) {
                *m = self.op32(Alu::Iabs, [lane(0, i), u, u], id)?;
            }
            for m in mag.iter_mut().take(n) {
                *m = self.op32(Alu::Ushr, [*m, Val::Const(k), u], id)?;
            }
            for i in 0..n {
                out[i] = self.op32(Alu::Imul, [sign[i], mag[i], u], id)?;
            }
            return Ok(out);
        }
        /* the recorded width can pass the lanes the sources have (a
         * module with no width table answers 4, and a source may be read
         * through a narrower value's swizzle): a lane past the sources'
         * ends is not one of the value's */
        let arity = ir_arity(a);
        for (i, o) in out.iter_mut().enumerate().take(w) {
            let s = [lane(0, i), lane(1, i), lane(2, i)];
            if i > 0 && s[..arity].contains(&Val::Undef) {
                break;
            }
            *o = self.lane32(m, a, srcs, s, id)?;
        }
        Ok(out)
    }

    /// A mediump operation's operand: one Mesa computes at fp32 (a highp
    /// varying's lane) makes the shader mixed.
    fn fp16_operand(&mut self, v: Val) -> Val {
        let generic = |loc: u8| (loc as usize) < MAX_VARYINGS;
        if self.input_lane(v).is_some_and(|(loc, _)| generic(loc) && self.prec.other >> (loc & 31) & 1 != 0) {
            self.mixed = true;
        }
        v
    }

    /// The input a lane is (location, component), if it is one.
    fn input_lane(&self, v: Val) -> Option<(u8, u8)> {
        match v {
            Val::Lane(n, c) => match self.nir.nodes[n as usize].kind {
                Kind::Input { loc, .. } if self.nir.nodes[n as usize].bits == 32 => Some((loc, c)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The texture look-ups the shader makes.
    pub fn look_ups(&self) -> &[TexLookup] {
        &self.look_ups[..self.nlook]
    }

    /// The look-ups the colour reads ([`layout`]), with their place in
    /// [`Fs::look_ups`].
    pub fn live_look_ups(&self) -> impl Iterator<Item = (usize, &TexLookup)> + '_ {
        self.look_ups().iter().enumerate().filter(|(_, t)| t.live())
    }

    /// `v` is a texel of an fp16 look-up (`TEX_SINGLE ... .16`).
    fn texel16(&self, v: Val) -> bool {
        self.texel(v).is_some_and(|(j, _)| self.tex_f16(j))
    }

    /// Look-up node `n` is taken at fp32 (a highp value reads it): refused
    /// when an fp16 operation already reads its halves ([`Fs::alu16`]),
    /// which would then read words of the other width (no reference).
    fn tex32(&mut self, n: u16) -> Result<(), MaliError> {
        let read16 = self.nir.nodes[..self.nir.n].iter().any(|x| {
            x.bits == 16
                && matches!(x.kind, Kind::Alu(_))
                && x.src.iter().any(|v| matches!(v, Val::Lane(m, _) if *m == n))
        });
        if read16 && self.nir.nodes[n as usize].bits == 16 {
            return Err(MaliError::MixedPrecision);
        }
        self.nir.nodes[n as usize].bits = 32;
        Ok(())
    }

    /// The look-up whose node is `n`, if one's is.
    fn look_up_of(&self, n: u16) -> Option<usize> {
        self.look_ups().iter().position(|t| t.node == n)
    }

    /// The texel a lane is, if it is one: its look-up's place in
    /// [`Fs::look_ups`] and the lane.
    fn texel(&self, v: Val) -> Option<(usize, u8)> {
        match v {
            Val::Lane(n, c) => self.look_up_of(n).map(|j| (j, c)),
            _ => None,
        }
    }

    /// Look-up `j`'s result is fp16 words (`TEX_SINGLE ... .16`).
    fn tex_f16(&self, j: usize) -> bool {
        self.nir.nodes[self.look_ups[j].node as usize].bits == 16
    }

    /// A look-up the walk met (past [`MAX_LOOK_UPS`]: no reference draws
    /// so many).
    fn push_look_up(&mut self, t: TexLookup) -> Result<(), MaliError> {
        let slot = self.look_ups.get_mut(self.nlook).ok_or(MaliError::TexCount)?;
        *slot = t;
        self.nlook += 1;
        Ok(())
    }

    /// `textureSize(s, 0)` of a 2D, 2D array or 3D texture at IR
    /// instruction `at`, as `va_lower_tex_query` lowers `txs` on v10
    /// (`pan_nir_load_va_tex_size`, pan_nir.h:105-140): the descriptor's
    /// first eight bytes loaded through the table handle ([`Kind::TexDesc`]
    /// at byte 0), then `if (!is_null)` on its first half, the then side
    /// `u2u32` of the width and height halves (`extract_u16` of the second
    /// word) and, for a 3D texture or an array, of the depth or layer count
    /// loaded at byte 28 or 24 (that load sunk to just before its `u2u32`,
    /// `nir_opt_sink` of `load_ssbo`), each plus one; the else side
    /// nothing; a phi per lane with the constant 0 (texmore_46.stderr's
    /// `%15`-`%24`, texmore_48.stderr's `%8`-`%15`). The `if` is built on
    /// `inot(ieq)` ([`nir::IfRegion::inot`]): bifrost's peephole flattens it
    /// to `bcsel(ieq, 0, size)` when its then side holds no load (the 2D
    /// texture of texmore_46, case 42's array whose layer count is not
    /// read: `CSEL.u32.eq`), and keeps its branch when it does (the array
    /// of texmore_46, the volume of texmore_48). The compare stands after
    /// the side's nodes in the order: where `nir_move_comparisons` leaves
    /// it, before its first select, once flattened; a kept `if` tests it at
    /// its block's end. A cube's size has no reference.
    fn tex_size(&mut self, sampler: u8, dim: ir::TexDim, at: usize, out: &mut [Val; 4]) -> Result<(), MaliError> {
        let u = Val::Undef;
        if self.nir.nifs >= nir::MAX_IFS {
            return Err(MaliError::BlockTableFull);
        }
        if self.nsizes >= MAX_SIZE_QUERIES {
            return Err(MaliError::TexCount);
        }
        let extra = match dim {
            ir::TexDim::D2 => None,
            ir::TexDim::D3 => Some(7 * 4),
            ir::TexDim::D2Array => Some(6 * 4),
            ir::TexDim::Cube => return Err(MaliError::TexKind),
        };
        let first = self.nir.n;
        let before = self.nir.cur;
        let d = NirNode { kind: Kind::TexDesc { sampler, byte: 0 }, src: [u; 3], bits: 32, width: 2 };
        let d = self.nir.push_node(d)?;
        let then_first = self.nir.new_block(before)?;
        self.nir.cur = then_first;
        let w = self.int32(Alu::Extract16(0), [Val::Lane(d, 1), u, u], at)?;
        let h = self.int32(Alu::Extract16(1), [Val::Lane(d, 1), u, u], at)?;
        let e = match extra {
            Some(byte) => {
                let l = NirNode { kind: Kind::TexDesc { sampler, byte }, src: [u; 3], bits: 16, width: 1 };
                let l = self.nir.push_node(l)?;
                self.int32(Alu::U2u32, [Val::Lane(l, 0), u, u], at)?
            }
            None => u,
        };
        let w = self.int32(Alu::Iadd, [w, Val::Const(1), u], at)?;
        let h = self.int32(Alu::Iadd, [h, Val::Const(1), u], at)?;
        let e = match e {
            Val::Undef => u,
            e => self.int32(Alu::Iadd, [e, Val::Const(1), u], at)?,
        };
        let then_last = self.nir.cur;
        let else_first = self.nir.new_block(before)?;
        self.nir.cur = before;
        let cond = NirNode {
            kind: Kind::Alu(Alu::Icmp(Cmpf::Ne)),
            src: [Val::Lane(d, 0), Val::Const(0), u],
            bits: 16,
            width: 1,
        };
        let cond = Val::Lane(self.nir.push_node(cond)?, 0);
        let after = self.nir.new_block(before)?;
        self.nir.cur = after;
        let mut outs = [u; 3];
        for (o, v) in outs.iter_mut().zip([w, h, e]) {
            if v != Val::Undef {
                *o = self.phi(v, Val::Const(0), at)?;
            }
        }
        for id in first..self.nir.n {
            self.origin[id] = at as u16;
        }
        let seq = self.nir.nseq;
        self.nir.nseq += 1;
        self.nir.ifs[self.nir.nifs] = nir::IfRegion {
            cond,
            before,
            then_first,
            then_last,
            else_first,
            else_last: else_first,
            after,
            live: true,
            guard: false,
            brk: None,
            seq,
            single: false,
            inot: true,
        };
        self.nir.nifs += 1;
        self.sizes[self.nsizes] = SizeQuery { sampler, node: d, out: outs, index: 0, live: false };
        self.nsizes += 1;
        out[..3].copy_from_slice(&outs);
        Ok(())
    }

    /// The size query whose first-eight-bytes node is `n`, if one's is.
    fn size_of(&self, n: u16) -> Option<usize> {
        self.sizes[..self.nsizes].iter().position(|q| q.node == n)
    }

    /// The sampler bindings the program reads, each with its descriptor
    /// index ([`layout`] numbers them: a live look-up's or size query's
    /// sampler, by binding), in index order.
    pub fn bound_samplers(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        let looks = self.live_look_ups().map(|(_, t)| (t.index, t.sampler));
        let sizes = self.sizes[..self.nsizes].iter().filter(|q| q.live).map(|q| (q.index, q.sampler));
        looks.chain(sizes)
    }

    /// The descriptor index of `sampler` ([`Fs::bound_samplers`]).
    fn sampler_index(&self, sampler: u8) -> Result<u8, MaliError> {
        self.bound_samplers().find(|&(_, s)| s == sampler).map(|(i, _)| i).ok_or(MaliError::TexKind)
    }

    /// The component of the run-time indexed vec4 element a lane is, if it
    /// is one.
    fn ubo_lane(&self, v: Val) -> Option<u8> {
        match v {
            Val::Lane(n, c) if self.nubo == 1 && n == self.ubo_idx[0] && self.ubo_comps(n) == 4 => Some(c),
            _ => None,
        }
    }

    /// The run-time indexed read of a float array a lane is, if it is one:
    /// its place in [`Fs::ubo_idx`].
    fn ubo_read(&self, v: Val) -> Option<u8> {
        match v {
            Val::Lane(n, 0) if self.ubo_comps(n) == 1 => {
                self.ubo_idx[..self.nubo].iter().position(|&t| t == n).map(|k| k as u8)
            }
            _ => None,
        }
    }

    /// Word `h` of a vec2, vec3 or vec4 array read `k` that a shader reads
    /// more than once ([`Leaf::UboWord`]), if lanes `a` and `b` are its two
    /// lanes in order, or `a` alone a vec3's z (`b` None).
    fn ubo_word(&self, a: Val, b: Option<Val>) -> Option<(u8, u8)> {
        let Val::Lane(n, c) = a else { return None };
        let comps = self.ubo_comps(n);
        if comps < 2 || c % 2 != 0 || (self.nubo == 1 && comps == 4) {
            return None;
        }
        let k = self.ubo_idx[..self.nubo].iter().position(|&t| t == n)? as u8;
        match b {
            Some(b) if b == Val::Lane(n, c + 1) => Some((k, c / 2)),
            None if c + 1 == comps => Some((k, c / 2)),
            _ => None,
        }
    }

    /// Lane `v` is fadds of a vec3 array's z lanes ([`Fs::ubo_word`]) alone.
    fn ubo_word_tree(&self, v: Val) -> bool {
        match self.alu_of(v) {
            Some((Alu::Fadd, s)) => s[..2].iter().all(|&x| self.ubo_word_tree(x)),
            Some(_) => false,
            None => self.ubo_word(v, None).is_some(),
        }
    }

    fn ubo_comps(&self, n: u16) -> u8 {
        match self.nir.nodes[n as usize].kind {
            Kind::UniformIdx { comps, .. } => comps,
            _ => 0,
        }
    }

    /// The fp16 uniform dword a lane is, if it is one (an int uniform's
    /// 32-bit word is none: it indexes, it is no colour).
    fn uniform_dword(&self, v: Val) -> Option<u16> {
        match v {
            Val::Lane(n, _) => match self.nir.nodes[n as usize] {
                NirNode { kind: Kind::Uniform { dword, .. }, bits: 16, .. } => Some(dword),
                _ => None,
            },
            _ => None,
        }
    }

    /// `v` is the `f2fmp` of a 32-bit uniform word in a mediump shader (the
    /// walk's read of a float uniform Mesa keeps fp32): its front-end dword.
    fn wide_uniform(&self, v: Val) -> Option<u16> {
        let (Alu::F2f16, [Val::Lane(n, _), ..]) = self.alu_of(v)? else { return None };
        match self.nir.nodes[n as usize] {
            NirNode { kind: Kind::Uniform { dword, comps: 1 }, bits: 32, .. } => Some(dword),
            _ => None,
        }
    }
}

/// Walk the body: loads of varyings, uniforms and constants, local copies,
/// `+`, `*`, an fneg, an fabs and the colour's store. Anything else is
/// refused by name. `prec` is the shader's float precision: a highp
/// shader's arithmetic is built as fp32 nodes ([`Fs::op32`]).
pub fn walk<'s>(m: &Module<'_>, scratch: &Arena<'s>, prec: FsPrecision) -> Result<Fs<'s>, MaliError> {
    if m.instrs.len() > MAX_VALUES {
        return Err(MaliError::TooManyValues);
    }
    if m.locals.len() > MAX_LOCALS {
        return Err(MaliError::TooManyLocals);
    }
    let oom = |_| MaliError::OutOfScratch;
    let mut f = Fs {
        nir: Nir::new(scratch)?,
        prec,
        /* a precision statement or declaration the walk does not see */
        mixed: prec.mixed,
        inputs16: false,
        origin: scratch.alloc_filled(MAX_NODES, 0u16).map_err(oom)?,
        pass: scratch.alloc_filled(MAX_NODES, 0u8).map_err(oom)?,
        keys: scratch.alloc_filled(MAX_VALUES, u32::MAX).map_err(oom)?,
        ir_comps: scratch.alloc_filled(MAX_VALUES, 0u8).map_err(oom)?,
        ir_vals: &[],
        live: [0; MAX_NODES / 64],
        colour: None,
        targets: [None; MAX_TARGETS - 1],
        fragcolor: m.outputs.iter().any(|d| matches!(d.kind, IoKind::Colour) && d.name == "gl_FragColor"),
        moved: [Moved::NONE; MAX_MOVED],
        nmoved: 0,
        var_key: [u32::MAX; MAX_VARYINGS],
        folded: [0; MAX_VARYINGS],
        pcoord_key: u32::MAX,
        fz_key: u32::MAX,
        inputs: [None; MAX_VARYINGS],
        pcoord: None,
        fz: None,
        varyings: [FsVarying::NONE; MAX_VARYINGS],
        nvar: 0,
        reads: [FsRead { location: 0, mask: 0, offset: 0 }; MAX_VARYINGS],
        nreads: 0,
        lane_of: [[None; 4]; MAX_VARYINGS],
        compacted: false,
        alpha: None,
        ubo_idx: [0; MAX_UBO_READS],
        ubo: UboLayout::NONE,
        nubo: 0,
        cmp_chain: [(Val::Undef, Val::Undef, Val::Undef); MAX_CMP_CHAIN],
        ncmp_chain: 0,
        prods: 0,
        look_ups: [TexLookup::NONE; MAX_LOOK_UPS],
        nlook: 0,
        sizes: [SizeQuery::NONE; MAX_SIZE_QUERIES],
        nsizes: 0,
        rows: 0,
        npairs: 0,
        frag_xy: None,
        frag_zw: None,
        frag16: None,
        fc_copy: false,
        rank: [u32::MAX; MAX_LOCALS],
        colour_rank: u32::MAX,
        next_rank: 0,
        discards: 0,
        loops: 0,
        guard: u32::MAX,
        kept: None,
        kept_body: (0, 0),
        cap: None,
        breaks: [BreakEdge::NONE; MAX_BREAKS],
        nbreaks: 0,
        nl: m.locals.len(),
        pool: scratch.alloc_filled((MAX_BREAKS + POOL_IF_TABLES) * m.locals.len(), [Val::Undef; 4]).map_err(oom)?,
        ptop: MAX_BREAKS,
        reg: nir::registration_order(m),
    };
    let vals = scratch.alloc_filled(MAX_VALUES, [Val::Undef; 4]).map_err(oom)?;
    let mut locals = [[Val::Undef; 4]; MAX_LOCALS];
    /* an `if` that ends `main` joins at the end of the program */
    let end = m.instrs.len().min(MAX_VALUES - 1);
    if walk_nodes(&mut f, m, m.nodes_of(m.body), vals, &mut locals, prec, true, end, None)? != Flow::Next {
        return Err(MaliError::ControlFlow);
    }
    /* the highp locals copy gl_FragCoord alone; a read beside the copy
     * that the walk narrowed ([`Fs::frag_coord16`]) may meet the fp32 copy
     * in a highp tree, where Mesa reads it at fp32: no reference */
    f.mixed |= !highp_locals_copy(m, &prec) || (f.fc_copy && f.frag16.is_some());
    f.ir_vals = vals;
    f.mark_live();
    /* a highp `log2` read by anything but a `pow` ([`Fs::lane32`]'s
     * `Flog2`, made for the `pow` that reads it): no reference */
    let lone_log = (0..f.nir.n).any(|k| {
        f.live[k / 64] >> (k % 64) & 1 != 0
            && matches!(f.nir.nodes[k], NirNode { kind: Kind::Alu(Alu::Flog2), bits: 32, .. })
    });
    if lone_log {
        return Err(MaliError::Arithmetic);
    }
    /* an output past location 0 is stored where its value is the last
     * store's: the joins and the loops carry the colour alone */
    let branches = f.nir.nifs != 0 || f.nir.nloops != 0 || f.discards != 0 || f.nlook != 0;
    if f.targets.iter().any(Option::is_some) && branches {
        return Err(MaliError::FragOutput);
    }
    /* fp16 texel products read by anything but fp16 arithmetic and the
     * highp output's conversions ([`Fs::texel16_product`]): no reference */
    for id in 0..f.nir.n {
        let nd = f.nir.nodes[id];
        let arity = match nd.kind {
            Kind::Alu(Alu::F2f32) => 0,
            Kind::Alu(_) if nd.bits == 16 => 0,
            Kind::Alu(op) => op.arity(),
            Kind::Phi => 2,
            Kind::Tex => 3,
            Kind::Discard => 1,
            _ => 0,
        };
        if nd.src[..arity].iter().any(|&v| f.texel16_product(v, 0)) {
            f.mixed = true;
        }
    }
    order_keys(m, f.keys);
    /* a 32-bit operation CSE'd with one of a later IR instruction whose
     * place comes first ([`swapped_compares`]): `nir_opt_cse` keeps the
     * first in program order, so it stands at its first reader's place
     * (deqp_constop_ce_asin_fs: `abs(b)` of `eps * abs(b) + eps`, made by
     * the `abs(a - b)` of `a` 0.0 that the compare emits after it). Users
     * first, so an operand follows a user that moved */
    for c in (0..f.nir.n).rev() {
        if !matches!(f.nir.nodes[c], NirNode { kind: Kind::Alu(_), bits: 32, .. }) {
            continue;
        }
        let first = f.nir.uses(c as u16).map(|(u, _)| f.origin[u as usize]).min_by_key(|&o| f.keys[o as usize]);
        if let Some(o) = first.filter(|&o| f.keys[o as usize] < f.keys[f.origin[c] as usize]) {
            f.origin[c] = o;
        }
    }
    /* a store makes no vector: an unrolled loop's nodes take the loop's
     * last instruction for their origin ([`walk_loop`]), often the
     * counter's store, which is no operation of several lanes
     * ([`Fs::wide`]; gm2_fcoord16_6's `d = fract(d * 1.7)` passes, one
     * lane each) */
    for (i, c) in f.ir_comps.iter_mut().enumerate().take(m.instrs.len()) {
        *c = match m.instrs[i] {
            ir::Op::StoreLocal { .. } | ir::Op::StoreOutput { .. } | ir::Op::Discard => 1,
            _ => width_of(m, ir::ValueId(i as u32)) as u8,
        };
    }
    /* a 2D array look-up's coordinate conversion CSE'd with one a later
     * statement made first (`nir_opt_cse` keeps the first in program
     * order): it stands at its first reader (case 42's `F16_TO_F32` of
     * `uv.x`, made for the look-up grafted into the colour's store, read
     * first by the fetch's coordinate: intbig_case42.stderr's `%19`) */
    for t in f.look_ups[..f.nlook].iter().filter(|t| t.array) {
        for v in f.nir.nodes[t.node as usize].src {
            let Val::Lane(c, _) = v else { continue };
            if !matches!(f.nir.nodes[c as usize].kind, Kind::Alu(Alu::F2f32)) {
                continue;
            }
            let first = f.nir.uses(c).map(|(u, _)| f.origin[u as usize]).min_by_key(|&o| f.keys[o as usize]);
            if let Some(o) = first.filter(|&o| f.keys[o as usize] < f.keys[f.origin[c as usize] as usize]) {
                f.origin[c as usize] = o;
            }
        }
    }
    /* each read of gl_FragCoord is its own lowered load, and `nir_opt_cse`
     * keeps the first in program order: the position's nodes stand at the
     * read whose statement comes first (fbload_fbload_int.stderr: the y
     * pair's `ffma` before `int(gl_FragCoord.x)`'s `u2u32`, the y read
     * being grafted into the later lane of the constructor) */
    let is_fc = |i: usize| matches!(m.instrs.get(i), Some(ir::Op::LoadInput { slot: IoSlot::FragCoord, .. }));
    if let Some(first) = (0..m.instrs.len().min(MAX_VALUES)).filter(|&i| is_fc(i)).min_by_key(|&i| f.keys[i]) {
        for o in f.origin[..f.nir.n].iter_mut() {
            if is_fc(*o as usize) {
                *o = first as u16;
            }
        }
    }
    for (i, op) in m.instrs.iter().enumerate().take(MAX_VALUES) {
        match *op {
            ir::Op::LoadInput { slot: IoSlot::Varying(v), .. } => {
                let k = f.var_key.get_mut(v as usize).ok_or(MaliError::VaryingLocation)?;
                *k = (*k).min(f.keys[i]);
            }
            ir::Op::LoadInput { slot: IoSlot::PointCoord, .. } => f.pcoord_key = f.pcoord_key.min(f.keys[i]),
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } => f.fz_key = f.fz_key.min(f.keys[i]),
            _ => {}
        }
    }
    Ok(f)
}

/// An `if` of a highp shader (`Node::If`), as `nir_lower_vars_to_ssa` and
/// `emit_cf_list` have it: each side walked in its own block (a side with
/// a nested `if` ends in a later one), the locals and the colour restored
/// between them, and at the join a phi per lane the sides left different
/// ([`Fs::phi`]). The condition is a comparison's mask (its low half is
/// what the branch tests, `emit_if`). A mediump shader's `if` has no
/// reference (the pair path, [`select`]). `exit` is what the sides may end
/// in ([`IfExit`]); the `if`'s own end is returned. The tables of the
/// locals' lanes it keeps ([`Fs::pool`]) are free again when it returns.
#[allow(clippy::too_many_arguments)]
fn walk_if(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    cond: Val,
    then_: &[Node],
    else_: &[Node],
    exit: IfExit<'_>,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
    join: usize,
) -> Result<Flow, MaliError> {
    let top = f.ptop;
    let r = walk_if_sides(f, m, cond, then_, else_, exit, vals, locals, prec, join);
    f.ptop = top;
    r
}

/// [`walk_if`]'s work.
#[allow(clippy::too_many_arguments)]
fn walk_if_sides(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    cond: Val,
    then_: &[Node],
    else_: &[Node],
    exit: IfExit<'_>,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
    join: usize,
) -> Result<Flow, MaliError> {
    /* a mediump pass's guard on the loop's one terminator ([`merged_bound`])
     * is a region as a highp one's: the ladder the optimisations flatten
     * afterwards ([`select_highp`]; cflow_71_deadloop), and so is an `if`
     * with a look-up under it, or a pass's guard in a loop with one: the
     * peephole takes no `tex` (`block_check_for_allowed_instrs`,
     * nir_opt_peephole_select.c:319), so the `if` keeps its blocks and its
     * branch ([`kept16`]; cflow_148_texbranch). Another mediump `if` is the
     * pair path's ([`walk_if16`]) */
    let merged = matches!(exit, IfExit::Guard(Unroll { merged: true, .. }));
    let kept = match exit {
        IfExit::Guard(lp) => holds_look_up(m, lp.nodes),
        _ => holds_look_up(m, then_) || holds_look_up(m, else_),
    };
    if !prec.highp && !merged && !kept {
        return walk_if16(f, m, cond, then_, else_, exit, vals, locals, prec, join);
    }
    let Val::Lane(cn, _) = cond else { return Err(MaliError::IfForm) };
    let kind = |v: Val| match v {
        Val::Lane(n, _) => Some(f.nir.nodes[n as usize].kind),
        _ => None,
    };
    let compare = |v: Val| matches!(kind(v), Some(Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_))));
    /* or a kept loop's head test with its counter's compare OR-ed in
     * ([`loop_cap_limit`]) */
    let src = f.nir.nodes[cn as usize].src;
    let capped = kind(cond) == Some(Kind::Alu(Alu::Ior))
        && compare(src[0])
        && kind(src[1]) == Some(Kind::Alu(Alu::Icmp(Cmpf::Lt)));
    if !compare(cond) && !capped {
        return Err(MaliError::IfForm);
    }
    if f.nir.nifs >= nir::MAX_IFS {
        return Err(MaliError::BlockTableFull);
    }
    let before = f.nir.cur;
    let (saved_locals, saved_colour) = (f.push_locals(locals)?, f.colour);
    /* a nested `if` that ends a side joins where the side's successor
     * begins: the else side's first instruction after a then side, this
     * `if`'s own join after the else side (its phis stand before the
     * outer join's, nir_opt_peephole_select.c:595 keeping the place) */
    let first_instr = |r: &[Node]| {
        r.iter().find_map(|n| match *n {
            Node::Instr(id) => Some(id.0 as usize),
            _ => None,
        })
    };
    let then_first = f.nir.new_block(before)?;
    f.nir.cur = then_first;
    let then_start = f.nir.n;
    let then_flow = walk_nodes(f, m, then_, vals, locals, prec, false, first_instr(else_).unwrap_or(join), None)?;
    let then_end = f.nir.n;
    let then_last = f.nir.cur;
    let (then_locals, then_colour) = (f.push_locals(locals)?, f.colour);
    f.load_locals(saved_locals, locals);
    f.colour = saved_colour;
    let else_first = f.nir.new_block(before)?;
    f.nir.cur = else_first;
    let else_start = f.nir.n;
    let rest = match exit {
        IfExit::Guard(lp) => Some(lp),
        _ => None,
    };
    let mut else_flow = walk_nodes(f, m, else_, vals, locals, prec, false, join, rest)?;
    /* the guard's continue side: the rest of the pass, then the passes
     * after it, each nested in the one before (`complex_unroll_loop_body`,
     * nir_opt_loop_unroll.c:237-286) */
    if let (Some(lp), Flow::Next | Flow::Continue) = (rest, else_flow) {
        walk_passes(f, m, lp, vals, locals, prec)?;
        else_flow = Flow::Break;
    }
    let else_end = f.nir.n;
    let else_last = f.nir.cur;
    /* a `break` or `continue` behind a condition the walk could not fold,
     * in a form `complex_unroll` does not take ([`walk_loop`]) */
    /* `if (c) break;` in a loop Mesa keeps: the then side's last block
     * jumps to the loop's exit, carrying the locals as they stand there
     * ([`walk_kept_loop`]); the join is entered from the else side alone */
    let brk = match (exit, then_flow, else_flow, f.kept) {
        (IfExit::Plain, Flow::Break, Flow::Next, Some(lp)) if f.loops == 0 => Some(lp),
        _ => None,
    };
    if let Some(lp) = brk {
        if f.nbreaks == MAX_BREAKS {
            return Err(MaliError::BlockTableFull);
        }
        let slot = f.nbreaks;
        let nl = f.nl;
        f.pool.copy_within(then_locals * nl..(then_locals + 1) * nl, slot * nl);
        f.breaks[slot] = BreakEdge { lp, block: then_last, slot: slot as u8, colour: then_colour };
        f.nbreaks += 1;
    }
    let flow = match exit {
        IfExit::Plain if brk.is_some() => Flow::Next,
        IfExit::Plain if then_flow == Flow::Next && else_flow == Flow::Next => Flow::Next,
        IfExit::Guard(_) if then_flow == Flow::Break && else_flow == Flow::Break => Flow::Break,
        IfExit::Continue if then_flow != Flow::Break && else_flow != Flow::Break => Flow::Next,
        _ => return Err(MaliError::LoopForm),
    };
    /* a side of nothing but discards (their compares computed before the
     * `if`), the other side empty: the GL front end's third
     * `nir_opt_peephole_select` (limit 0, `discard_ok`, gl_nir_linker.c:
     * 107-111) folds it into a `demote_if` of the two conditions' `iand`
     * (`rewrite_intrinsic_conds`, nir_opt_peephole_select.c:350-376), a
     * form no reference draws */
    let only_discards =
        |start: usize, end: usize| start < end && (start..end).all(|id| f.nir.nodes[id].kind == Kind::Discard);
    if (only_discards(then_start, then_end) && else_start == else_end && else_.is_empty())
        || (only_discards(else_start, else_end) && then_start == then_end && then_.is_empty())
    {
        return Err(MaliError::DiscardForm);
    }
    let after = f.nir.new_block(before)?;
    f.nir.cur = after;
    /* a phi stands at the join: after both sides, where the instruction
     * after the `if` is (the `bcsel` the peephole makes of it keeps the
     * place, nir_opt_peephole_select.c:595); the variables' phis in the
     * reverse of their first-reference order ([`Fs::rank`]), each
     * variable's lanes in order (`nir_lower_phis_to_scalar`, one scalar
     * phi per lane inserted before the vector one). The colour is an
     * output temporary (`nir_lower_io_to_temporaries`): a side that leaves
     * it unwritten leaves it undefined, so the written side's value is the
     * join's (`nir_opt_undef`) */
    let at = join;
    let colour = match (then_colour, f.colour) {
        _ if brk.is_some() => None,
        (Some(a), Some(b)) if a != b => Some((a, b)),
        _ => None,
    };
    /* the variables a side changed and the colour (`MAX_LOCALS`), by
     * (rank, local) from the largest down: a scan per phi set, not a table
     * of every local on this frame ([`Fs::pool`]) */
    let mut prev = None;
    loop {
        let changed = |l: usize| brk.is_none() && f.table(then_locals)[l] != locals[l];
        let colour_key = colour.map(|_| (f.colour_rank, MAX_LOCALS));
        let next = (0..f.nl)
            .filter(|&l| changed(l))
            .map(|l| (f.rank[l], l))
            .chain(colour_key)
            .filter(|k| prev.is_none_or(|p| *k < p))
            .max();
        let Some(key) = next else { break };
        prev = Some(key);
        let l = key.1;
        if l == MAX_LOCALS {
            let Some((a, b)) = colour else { continue };
            let mut c = [Val::Undef; 4];
            for (k, v) in c.iter_mut().enumerate() {
                *v = f.phi(a[k], b[k], at)?;
            }
            f.colour = Some(c);
            continue;
        }
        for c in 0..4 {
            let (a, b) = (f.table(then_locals)[l][c], locals[l][c]);
            if a == b || a == Val::Undef || b == Val::Undef {
                locals[l][c] = f.phi(a, b, at)?;
                continue;
            }
            /* a side's value that is a lane of a vector is copied at the
             * side's end for the phi (`nir_lower_phis_to_scalar`; a texel,
             * cflow_148_texbranch) */
            let a = f.phi_src(a, then_last, at)?;
            let b = f.phi_src(b, else_last, at)?;
            locals[l][c] = f.phi(a, b, at)?;
        }
    }
    if colour.is_none() && brk.is_none() {
        f.colour = then_colour.or(f.colour);
    }
    let guard = matches!(exit, IfExit::Guard(_));
    let seq = f.nir.nseq;
    f.nir.nseq += 1;
    f.nir.ifs[f.nir.nifs] = nir::IfRegion {
        cond,
        before,
        then_first,
        then_last,
        else_first,
        else_last,
        after,
        live: true,
        guard,
        brk,
        seq,
        single: matches!(exit, IfExit::Guard(Unroll { merged: true, .. })),
        inot: false,
    };
    f.nir.nifs += 1;
    Ok(flow)
}

/// An `if` of a mediump shader whose sides only assign booleans (case 112:
/// `u_false == sideEffectA && funcB()`, the inlined call's two flags set
/// under the condition): `nir_opt_peephole_select` makes each variable a
/// `bcsel` of the condition at the join, which the boolean rules fold
/// (`bcsel(a, true, false)` is `a`, `bcsel(a, false, true)` `inot(a)`,
/// `bcsel(a, true, a)` `ior(a, a)`, `bcsel(a, a, false)` `iand(a, a)`,
/// nir_opt_algebraic.py), so no select is left (intbig_case112.stderr's
/// final NIR has none). A side that writes the colour or anything but a
/// boolean, or ends in a `break` or `continue`, has no reference.
#[allow(clippy::too_many_arguments)]
fn walk_if16(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    cond: Val,
    then_: &[Node],
    else_: &[Node],
    exit: IfExit<'_>,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
    join: usize,
) -> Result<Flow, MaliError> {
    /* a pass's run-time `break` or `continue` (intloop_contloops' `for (i <
     * 8)`): Mesa's fp16 ladder, no reference lowered */
    if !matches!(exit, IfExit::Plain) {
        return Err(MaliError::GuardMediump);
    }
    /* a float compare's 16-bit mask (`pan_nir_lower_bool_to_bitsize`):
     * the flattened `if`s below */
    let cmp16 = matches!(cond, Val::Lane(n, 0) if matches!(
        f.nir.nodes[n as usize],
        NirNode { kind: Kind::Alu(Alu::Fcmp(_)), bits: 16, width: 1, .. }
    ));
    if !f.mask_lane(cond) && !cmp16 {
        return Err(MaliError::ControlFlow);
    }
    let (saved_locals, saved_colour) = (f.push_locals(locals)?, f.colour);
    let (looks, discards) = (f.nlook, f.discards);
    let then_start = f.nir.n;
    let then_flow = walk_nodes(f, m, then_, vals, locals, prec, false, join, None)?;
    let then_locals = f.push_locals(locals)?;
    f.load_locals(saved_locals, locals);
    let then_colour = core::mem::replace(&mut f.colour, saved_colour);
    let else_start = f.nir.n;
    let else_flow = walk_nodes(f, m, else_, vals, locals, prec, false, join, None)?;
    if then_flow != Flow::Next || else_flow != Flow::Next || (then_colour != f.colour && !cmp16) {
        return Err(MaliError::ControlFlow);
    }
    /* `nir_opt_peephole_select` takes sides of ALU work alone, at most
     * [`nir::PEEPHOLE_LIMIT`] operations each (nir_opt_peephole_select.c:
     * 543-549): a look-up, a discard or a longer side keeps its branch, a
     * form no mediump reference draws (cflow_148_texbranch's) */
    let alu_only = |a: usize, b: usize| {
        b - a <= nir::PEEPHOLE_LIMIT as usize
            && f.nir.nodes[a..b].iter().all(|nd| matches!(nd.kind, Kind::Alu(_) | Kind::Uniform { .. }))
    };
    if cmp16
        && (f.nlook != looks
            || f.discards != discards
            || !alu_only(then_start, else_start)
            || !alu_only(else_start, f.nir.n))
    {
        return Err(MaliError::ControlFlow);
    }
    let not = |f: &mut Fs<'_>, v: Val| f.int_lane(m, AluOp::Bnot, &[ir::Src::id(ir::ValueId(0)); 3], [v; 3], join);
    /* a lane each side defines its own way: the peephole's `bcsel_pan` of
     * the compare at the join (cflow_68_armselect.stderr's `%8 = bcsel_pan
     * %7.xx, %3.xy, %2.xy`), which [`vectorise`] pairs */
    let sel = |f: &mut Fs<'_>, t: Val, e: Val| match (t, e) {
        _ if t == e => Ok(t),
        (t, Val::Undef) => Ok(t),
        (Val::Undef, e) => Ok(e),
        /* a boolean's 32-bit select under a float compare: no reference */
        _ if cmp16
            && ![t, e].iter().any(|&v| v == Val::Const(u32::MAX) || (f.mask_lane(v) && !f.is_const(v, &[0]))) =>
        {
            f.node(Alu::Bcsel, [cond, t, e], join)
        }
        _ => Err(MaliError::ControlFlow),
    };
    for (l, lanes) in locals.iter_mut().enumerate().take(f.nl) {
        for (c, e) in lanes.iter_mut().enumerate() {
            let t = f.table(then_locals)[l][c];
            *e = match (t, *e) {
                _ if t == *e => t,
                /* a side that leaves it undefined (`nir_opt_undef`) */
                (t, Val::Undef) => t,
                (Val::Undef, e) => e,
                (Val::Const(u32::MAX), Val::Const(0)) if !cmp16 => cond,
                (Val::Const(0), Val::Const(u32::MAX)) if !cmp16 => not(f, cond)?,
                (Val::Const(u32::MAX), e) if e == cond && !cmp16 => cond,
                (t, Val::Const(0)) if t == cond && !cmp16 => cond,
                (t, e) => sel(f, t, e)?,
            };
        }
    }
    /* the colour is an output temporary: written on one side alone it is
     * undefined on the other ([`walk_if`]), else a select per lane
     * (cflow_74_ifoutput: `%6 = bcsel_pan %4, 0x3c00, 0x0`) */
    f.colour = match (then_colour, f.colour) {
        (Some(a), Some(b)) => {
            let mut c = [Val::Undef; 4];
            for (k, v) in c.iter_mut().enumerate() {
                *v = sel(f, a[k], b[k])?;
            }
            Some(c)
        }
        (a, b) => a.or(b),
    };
    Ok(Flow::Next)
}

/// What the sides of an `if` may end in ([`walk_if`]).
#[derive(Clone, Copy)]
enum IfExit<'a> {
    /// Both fall through: an `if` of the source.
    Plain,
    /// The guard of a pass `complex_unroll` unrolls ([`walk_guard`]): the
    /// then side is the `break`, the else side the rest of the pass and
    /// then the loop's later passes; the `if` ends the loop.
    Guard(Unroll<'a>),
    /// `if (c) { ...; continue; }` in a pass: the else side the rest of the
    /// pass, either side ending in the `continue` or falling through; the
    /// `if` ends the pass.
    Continue,
}

/// A loop being unrolled ([`walk_loop`]): its body, the IR instruction its
/// nodes stand at, and the pass being walked (from 1).
#[derive(Clone, Copy)]
struct Unroll<'a> {
    nodes: &'a [Node],
    last: usize,
    pass: usize,
    /// The guard's `if` is the loop's one terminator ([`merged_bound`]).
    merged: bool,
}

/// The passes of loop `lp` after its pass `lp.pass`, each walked until a
/// `break` ends the loop: a folded one, or a guard that took the passes
/// after it into its continue side ([`walk_guard`]). A loop that runs past
/// [`MAX_UNROLL_ITERATIONS`] passes has no limiting terminator Mesa
/// unrolls by ([`MaliError::LoopForm`]).
fn walk_passes(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    lp: Unroll<'_>,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
) -> Result<(), MaliError> {
    let mut pass = lp.pass;
    loop {
        if pass >= MAX_UNROLL_ITERATIONS {
            return Err(MaliError::LoopForm);
        }
        pass += 1;
        let next = Unroll { pass, ..lp };
        let first = f.nir.n;
        let flow = walk_nodes(f, m, lp.nodes, vals, locals, prec, false, lp.last, Some(next))?;
        /* a later pass a guard nests in this one ([`walk_guard`]) keeps its own */
        for id in first..f.nir.n {
            if f.pass[id] == 0 {
                f.pass[id] = pass as u8;
            }
        }
        if flow == Flow::Break {
            return Ok(());
        }
    }
}

/// `if (c) break;` in pass `lp.pass` of a loop, `c` a run-time value: the
/// loop's other terminator, the one `complex_unroll` (nir_opt_loop_unroll.c:
/// 338-455) takes besides the limiting one, its counter's test folded in
/// every pass ([`walk_passes`]). Each pass's copy is nested in the continue
/// side of the previous one's guard (`get_complex_unroll_insert_location`,
/// :222): an `if` whose then side is the `break` (the locals as they stand,
/// at the loop's exit) and whose else side is `rest`, the rest of the pass,
/// then the later passes; the join's phis are the values the loop leaves.
/// Mesa takes a loop with exactly two terminators, one of them limiting
/// (:1129-1150): a second run-time `break` keeps the loop, which no
/// reference draws. What the optimisations make of the nest
/// ([`nir::Nir::inverted_guards`], `peephole_select`): measured on gl_test's
/// databreak, case 87 (docs/notes/mali-loop-break.md).
#[allow(clippy::too_many_arguments)]
fn walk_guard(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    c: Val,
    at: ir::ValueId,
    rest: &[Node],
    lp: Unroll<'_>,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
) -> Result<Flow, MaliError> {
    if f.guard != u32::MAX && f.guard != at.0 {
        return Err(MaliError::LoopForm);
    }
    f.guard = at.0;
    /* a mediump float counter's test and the guard on it are one
     * terminator, `fge(i, fmin(x, C))` (see [`merged_bound`]) */
    let (c, merged) = match (prec.highp, merged_bound(m, lp.nodes, at), f.alu_of(c)) {
        (false, Some(bound), Some((Alu::Fcmp(Cmpf::Ge), [i @ Val::Const(_), x, _]))) => {
            let fm = f.node(Alu::Fmin, [x, Val::Const(bound), Val::Undef], at.0 as usize)?;
            (f.node(Alu::Fcmp(Cmpf::Ge), [i, fm, Val::Undef], at.0 as usize)?, true)
        }
        _ => (c, false),
    };
    let exit = IfExit::Guard(Unroll { merged, ..lp });
    walk_if(f, m, c, &[Node::Break], rest, exit, vals, locals, prec, lp.last)
}

/// Run `nodes` holds a texture look-up, itself or in a nested region: the
/// `if` around it keeps its blocks ([`walk_if`]).
fn holds_look_up(m: &Module<'_>, nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match *n {
        Node::Instr(id) => matches!(m.instr(id), Some(ir::Op::Tex { .. })),
        Node::If { then_, else_, .. } => holds_look_up(m, m.nodes_of(then_)) || holds_look_up(m, m.nodes_of(else_)),
        Node::Loop { body } => holds_look_up(m, m.nodes_of(body)),
        Node::Break | Node::Continue | Node::Return => false,
    })
}

/// The bound `C` of a loop whose body `nodes` begins with its test on a
/// float counter, `if (i < C) {} else break;` (`Lower::break_unless`), and
/// whose next control-flow node is the guard `if (i >= x) break;` at IR
/// value `at`, nothing but instructions between them. NIR inverts the
/// test (`opt_if_simplification`, nir_opt_if.c:741-782; `inot(flt(i, C))`
/// is `fge(i, C)`), `opt_loop_merge_terminators` makes the two one
/// terminator of `ior(fge(i, x), fge(i, C))` (nir_opt_loop.c:554-602,
/// 648-712: the second `if`'s condition first) and nir_opt_algebraic.py:
/// 1152 makes that `fge(i, fmin(x, C))`, the one condition each unrolled
/// pass tests (`complex_unroll_single_terminator`,
/// nir_opt_loop_unroll.c:466-527, its last clone's `if` the test past the
/// bound, whose sides the lcssa copies make equal). Measured:
/// cflow_71_deadloop's `%6 = fmin %4, %5 (8.000000)` and the compares of
/// the passes with it (docs/notes/mali-control-flow.md).
fn merged_bound(m: &Module<'_>, nodes: &[Node], at: ir::ValueId) -> Option<u32> {
    let mut ctl = nodes.iter().filter(|n| !matches!(n, Node::Instr(_)));
    let (Some(&Node::If { cond: test, then_, else_ }), Some(&Node::If { cond: guard, then_: brk, else_: none })) =
        (ctl.next(), ctl.next())
    else {
        return None;
    };
    if then_.len != 0 || m.nodes_of(else_) != [Node::Break] || m.nodes_of(brk) != [Node::Break] || none.len != 0 {
        return None;
    }
    if guard.val != at {
        return None;
    }
    let counter = |s: ir::Src| match m.def(s.val) {
        Some(&ir::Op::LoadLocal { local, index: None, .. }) => Some(local),
        _ => None,
    };
    let (Some(&ir::Op::Alu(AluOp::Flt, [i, k, _])), Some(&ir::Op::Alu(AluOp::Fle, [_, j, _]))) =
        (m.def(test.val), m.def(guard.val))
    else {
        return None;
    };
    let Some(ir::Op::ConstF32(bits)) = m.def(k.val) else { return None };
    (counter(i).is_some() && counter(i) == counter(j)).then_some(bits[k.swizzle[0] as usize & 3])
}

/// A loop (`Node::Loop`; a mediump shader's too, whose passes must fold to
/// constants and selects: case 72's `for (int i = 0; i < 3; ..) r +=
/// 0.25`, the constant 0.75), unrolled as `nir_opt_loop_unroll`
/// unrolls one whose trip count a constant bound decides (`simple_unroll`,
/// nir_opt_loop_unroll.c:105, within [`MAX_UNROLL_ITERATIONS`] and
/// [`LOOP_UNROLL_LIMIT`]): the body walked pass after pass, its counter's
/// arithmetic folded as it goes ([`Fs::lane32`]) so its test is an `if` on
/// a constant whose side runs in place ([`walk_nodes`]), until a `break` is
/// taken. The nodes of every pass stand where the body's last instruction
/// does: one straight run in NIR's order, as the unrolled copies are. A
/// `break` behind a run-time condition is the loop's second terminator,
/// which `complex_unroll` takes with the counter's test as the limiting one
/// ([`walk_guard`]: gl_test's databreak, case 87). Guarded passes too large for
/// the peephole stay branches (femtovg's `filter_image`: `if (i >= sampleCount)
/// break;`, 24 guarded copies with two look-ups each). A loop that runs past
/// the bound or has two run-time `break`s has no reference ([`MaliError::LoopForm`]).
fn walk_loop(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    body: ir::Range,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
) -> Result<(), MaliError> {
    let nodes = m.nodes_of(body);
    let last = nodes.iter().rev().find_map(|n| match *n {
        Node::Instr(id) => Some(id.0 as usize),
        _ => None,
    });
    let Some(last) = last else { return Err(MaliError::LoopForm) };
    let first = f.nir.n;
    f.loops += 1;
    let outer = f.guard;
    f.guard = u32::MAX;
    let r = walk_passes(f, m, Unroll { nodes, last, pass: 0, merged: false }, vals, locals, prec);
    f.guard = outer;
    f.loops -= 1;
    r?;
    if unroll_cost(f, first) > LOOP_UNROLL_LIMIT * MAX_UNROLL_ITERATIONS {
        return Err(MaliError::LoopForm);
    }
    for id in first..f.nir.n {
        f.origin[id] = last as u16;
    }
    Ok(())
}

/// The cost of the passes walked from node `first` on, as loop analysis
/// counts its body's (`instr_cost`, nir_loop_analyze.c:79-130) times the
/// trip count (nir_opt_loop_unroll.c:924-925): every node one, but a phi
/// (no ALU, intrinsic or look-up), a constant the passes folded (no node
/// here) and an `fmul` only `fadd`s read (`is_only_used_by_fadd`: fused
/// later). Femtovg's filter_image makes 1030 nodes in its 24 passes, more
/// than 26 * 32; this count is within it, and Mesa unrolls the loop
/// (fvg_filter_image.expect.json).
fn unroll_cost(f: &Fs<'_>, first: usize) -> usize {
    let nodes = &f.nir.nodes[first..f.nir.n];
    let only_fadd = |id: usize| {
        nodes.iter().all(|u| {
            let reads = u.src.iter().any(|s| matches!(*s, Val::Lane(n, _) if n as usize == id));
            !reads || u.kind == Kind::Alu(Alu::Fadd)
        })
    };
    (first..f.nir.n)
        .filter(|&id| match f.nir.nodes[id].kind {
            Kind::Phi => false,
            Kind::Alu(Alu::Fmul) => !only_fadd(id),
            _ => true,
        })
        .count()
}

/// Whether Mesa keeps loop `body`: its test (`Lower::break_unless`, the
/// body's first control-flow node) is on a value the walk cannot fold at
/// the first pass (a varying, a uniform), so loop analysis finds no
/// limiting terminator (`nir_loop_analyze.c`) and `nir_opt_loop_unroll`
/// leaves it (nir_opt_loop_unroll.c:1112-1114). A loop whose test the
/// walk folds is unrolled ([`walk_loop`]), as is one with no test at its
/// head (a `do .. while`, whose terminator is the walk's to find).
fn kept_loop(f: &Fs<'_>, m: &Module<'_>, body: ir::Range, locals: &[[Val; 4]; MAX_LOCALS]) -> bool {
    let nodes = m.nodes_of(body);
    let Some(head) = nodes.iter().find(|n| !matches!(n, Node::Instr(_))) else { return false };
    let Node::If { cond, then_, else_ } = *head else { return false };
    if then_.len != 0 || m.nodes_of(else_) != [Node::Break] {
        return false;
    }
    let _ = f;
    !ir_folds(m, cond.val, locals, 0)
}

/// IR value `v` folds to a constant from the constants and the locals'
/// constant lanes alone.
pub(super) fn ir_folds(m: &Module<'_>, v: ir::ValueId, locals: &[[Val; 4]; MAX_LOCALS], depth: u32) -> bool {
    if depth > 32 {
        return false;
    }
    match m.def(v) {
        Some(ir::Op::ConstF32(_) | ir::Op::ConstI32(_) | ir::Op::ConstBool(_)) => true,
        Some(&ir::Op::LoadLocal { local, index: None, comps }) => {
            locals.get(local.0 as usize).is_some_and(|l| l[..comps as usize].iter().all(|x| matches!(x, Val::Const(_))))
        }
        Some(&ir::Op::Alu(a, s)) => s[..ir_arity(a)].iter().all(|x| ir_folds(m, x.val, locals, depth + 1)),
        _ => false,
    }
}

/// Lane `c` of local `l` is stored by an instruction of run `nodes` itself
/// before anything in the run reads the local: each pass sets it before
/// use, so no value of it is carried around a loop whose body is `nodes`.
pub(super) fn declared_in(m: &Module<'_>, nodes: &[Node], l: usize, c: usize) -> bool {
    for n in nodes {
        match *n {
            Node::Instr(id) => match m.instr(id) {
                Some(&ir::Op::StoreLocal { local, index: None, write_mask, .. }) if local.0 as usize == l => {
                    if write_mask & (1 << c) != 0 {
                        return true;
                    }
                }
                Some(ir::Op::StoreLocal { local, .. } | ir::Op::LoadLocal { local, .. }) if local.0 as usize == l => {
                    return false
                }
                _ => {}
            },
            Node::If { then_, else_, .. } => {
                if refers_to(m, then_, l) || refers_to(m, else_, l) {
                    return false;
                }
            }
            Node::Loop { body } => {
                if refers_to(m, body, l) {
                    return false;
                }
            }
            Node::Break | Node::Continue | Node::Return => return false,
        }
    }
    false
}

/// Region `r` loads or stores local `l`.
fn refers_to(m: &Module<'_>, r: ir::Range, l: usize) -> bool {
    m.nodes_of(r).iter().any(|n| match *n {
        Node::Instr(id) => matches!(
            m.instr(id),
            Some(ir::Op::StoreLocal { local, .. } | ir::Op::LoadLocal { local, .. }) if local.0 as usize == l
        ),
        Node::If { then_, else_, .. } => refers_to(m, then_, l) || refers_to(m, else_, l),
        Node::Loop { body } => refers_to(m, body, l),
        _ => false,
    })
}

/// Runs `a` and `b` are the same instructions (the front end's copies of
/// one `for` step): local loads and stores, constants and ALU operations,
/// equal but for the values each defines, which they read in the same
/// places.
fn same_instrs(m: &Module<'_>, a: &[Node], b: &[Node]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let ids = |r: &[Node], k: usize| match r[k] {
        Node::Instr(id) => Some(id),
        _ => None,
    };
    /* a source: the same value outside the runs, or the values the two
     * runs define at one place */
    let same_src = |x: ir::Src, y: ir::Src| {
        let pos = |r: &[Node], v: ir::ValueId| (0..r.len()).find(|&k| ids(r, k).is_some_and(|id| id.0 == v.0));
        x.swizzle == y.swizzle
            && match (pos(a, x.val), pos(b, y.val)) {
                (Some(p), Some(q)) => p == q,
                (None, None) => x.val == y.val,
                _ => false,
            }
    };
    (0..a.len()).all(|k| {
        let (Some(x), Some(y)) = (ids(a, k), ids(b, k)) else { return false };
        match (m.instr(x), m.instr(y)) {
            (
                Some(&ir::Op::LoadLocal { local: p, index: None, comps: c }),
                Some(&ir::Op::LoadLocal { local: q, index: None, comps: d }),
            ) => p == q && c == d,
            (
                Some(&ir::Op::StoreLocal { local: p, index: None, src: s, write_mask: w }),
                Some(&ir::Op::StoreLocal { local: q, index: None, src: t, write_mask: v }),
            ) => p == q && w == v && same_src(s, t),
            (Some(ir::Op::ConstI32(p)), Some(ir::Op::ConstI32(q))) => p == q,
            (Some(ir::Op::ConstF32(p)), Some(ir::Op::ConstF32(q))) => p == q,
            (Some(&ir::Op::Alu(p, s)), Some(&ir::Op::Alu(q, t))) => {
                p == q && (0..ir_arity(p)).all(|i| same_src(s[i], t[i]))
            }
            _ => false,
        }
    })
}

/// The locals' lanes loop `body` stores (`nir_lower_vars_to_ssa` places a
/// phi for each at the loop's header), each local's in a mask.
fn stored_lanes(m: &Module<'_>, body: ir::Range, masks: &mut [u8; MAX_LOCALS]) -> Result<bool, MaliError> {
    let mut colour = false;
    for n in m.nodes_of(body) {
        match *n {
            Node::Instr(id) => match m.instr(id) {
                Some(&ir::Op::StoreLocal { local, index: None, write_mask, .. }) => {
                    *masks.get_mut(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))? |= write_mask;
                }
                Some(ir::Op::StoreLocal { .. }) => return Err(MaliError::IndexedLocal),
                Some(ir::Op::StoreOutput { .. }) => colour = true,
                _ => {}
            },
            Node::If { then_, else_, .. } => {
                colour |= stored_lanes(m, then_, masks)?;
                colour |= stored_lanes(m, else_, masks)?;
            }
            Node::Loop { body } => colour |= stored_lanes(m, body, masks)?,
            Node::Break | Node::Continue | Node::Return => {}
        }
    }
    Ok(colour)
}

/// The passes kept loop `body` may run before its hidden counter ends it,
/// the rule of the other back ends (select.rs `loop_convert`): a constant
/// `for`'s own count of head entries (`loop_count::constant_loop_entries`,
/// the index's value before the loop read from `locals`), so the counter
/// never ends it, or [`LOOP_CAP`] (4096) for a loop that cannot be counted.
///
/// The counter is a DELIBERATE deviation from Mesa 26.2.1, which keeps such
/// a loop with the GLSL test as its only exit: a bound read at run time can
/// keep a pass running past the GPU's job timeout (case 104's band 7, a
/// 1e9 bound per fragment), and gl_test wants the capped 4096 passes (case
/// 98's `count = 4097`). The counter counts the head entries down, so the
/// limit is the pre-header's immediate and the compare is with the inline
/// zero (no constant in the loop, no FAU word): the pre-header's `IADD_IMM
/// rc, 0x0, #limit` (the phi's first source), the header's `IADD_IMM rc,
/// rc^, #0xFFFFFFFF` and `ICMP_OR.s32.lt.m1 rs, rc, 0x0`, OR-ed into the
/// head test's mask before its `BRANCHZ` (an integer test takes it as its
/// third source, `va_fuse_cmp`; a float test keeps a `LSHIFT_OR`). Entry
/// `k` leaves when `limit - k < 0`, so the body runs at most `limit` times
/// (docs/notes/mali-loop-board-failures.md).
fn loop_cap_limit(m: &Module<'_>, body: ir::Range, locals: &[[Val; 4]; MAX_LOCALS]) -> u32 {
    let init = |l: ir::LocalId, _float: bool| match locals.get(l.0 as usize).map(|v| v[0]) {
        Some(Val::Const(bits)) => Some(bits),
        _ => None,
    };
    crate::shaderc::online::loop_count::constant_loop_entries(m, body, &init).unwrap_or(LOOP_CAP)
}

/// A loop Mesa keeps ([`kept_loop`]) as `emit_loop` emits it
/// (bifrost_compile.c:3738-3770; [`nir::LoopRegion`]): a header block
/// holding one phi per local lane the body stores, in the order of the
/// locals' first stores (measured: intloop_loop_exit's `s` then `i`,
/// builtin_98's `n`, `last`, `i`, builtin_45's `c` then `i`;
/// docs/notes/mali-loop-break.md, Part 2), each taking the value before
/// the loop and the one at the end of the body; the body walked once from
/// the header, its `break`s jumps to the exit block ([`walk_if`]), where
/// the locals are those of the `break` (one) or their phis (two), its
/// `continue`s branches over the rest of the body to the latch
/// ([`walk_nodes`]). A kept loop may nest in another (intloop_divnest).
fn walk_kept_loop(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    body: ir::Range,
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
) -> Result<(), MaliError> {
    /* a mediump shader's kept loop: Mesa carries its fp16 lanes across the
     * back edge as 16-bit vectors, and the mediump selection has no blocks
     * (intloop_loop_nested; docs/notes/mali-loop-break.md, Part 2) */
    if !prec.highp {
        return Err(MaliError::LoopMediump);
    }
    let nodes = m.nodes_of(body);
    let last = nodes.iter().rev().find_map(|n| match *n {
        Node::Instr(id) => Some(id.0 as usize),
        _ => None,
    });
    let Some(last) = last else { return Err(MaliError::LoopForm) };
    if f.nir.nloops == nir::MAX_LOOPS {
        return Err(MaliError::BlockTableFull);
    }
    let mut masks = [0u8; MAX_LOCALS];
    /* the colour written in the loop: no reference */
    if stored_lanes(m, body, &mut masks)? {
        return Err(MaliError::LoopForm);
    }
    let limit = loop_cap_limit(m, body, locals);
    let lp = f.nir.nloops as u8;
    f.nir.nloops += 1;
    let pre = f.nir.cur;
    let header = f.nir.new_block(pre)?;
    f.nir.cur = header;
    /* the header's phis: the locals in the reverse of the order
     * `nir_lower_vars_to_ssa` registers them ([`nir::registration_order`]:
     * the constant-initialised ones first, so they come last), each one's
     * lanes in order */
    /* compact entries: this frame nests in the walk ([`Fs::pool`]) */
    let mut order = [(u32::MAX, 0u16); MAX_LOCALS];
    let mut nl = 0;
    for (l, &mask) in masks.iter().enumerate() {
        if mask != 0 {
            order[nl] = (u32::MAX - u32::from(f.reg[l]), l as u16);
            nl += 1;
        }
    }
    order[..nl].sort_unstable();
    let mut phis = [(0u16, 0u16, 0u8); MAX_LOCALS * 4];
    let mut nphi = 0;
    for &(_, l) in &order[..nl] {
        let l = usize::from(l);
        /* a mediump shader's float local is fp16 (lower_precision), its
         * int or bool 32-bit; a highp shader's lanes are their nodes' */
        let float = m.locals.get(l).is_some_and(|d| d.ty.scalar == ir::ScalarType::F32);
        for c in 0..4 {
            if masks[l] & (1 << c) == 0 {
                continue;
            }
            let v = locals[l][c];
            /* a local the body declares (divnest's inner `j`, stored before
             * any read in each pass): no value reaches the header, so no
             * phi (`nir_lower_vars_to_ssa` places none where the variable
             * is not live) */
            if v == Val::Undef {
                if !declared_in(m, nodes, l, c) {
                    return Err(MaliError::LoopForm);
                }
                continue;
            }
            let bits = match v {
                _ if !prec.highp && float => 16,
                Val::Lane(n, _) => f.nir.nodes[n as usize].bits,
                _ => 32,
            };
            let id = f.nir.push_node(NirNode { kind: Kind::Phi, src: [v, Val::Undef, Val::Undef], bits, width: 1 })?;
            f.origin[id as usize] = last as u16;
            locals[l][c] = Val::Lane(id, 0);
            phis[nphi] = (id, l as u16, c as u8);
            nphi += 1;
        }
    }
    /* the pass counter's phi, after the locals' (a DELIBERATE deviation from
     * Mesa, which has none: [`loop_cap_limit`]) */
    let nd = NirNode { kind: Kind::Phi, src: [Val::Const(limit), Val::Undef, Val::Undef], bits: 32, width: 1 };
    let cap_phi = f.nir.push_node(nd)?;
    f.origin[cap_phi as usize] = last as u16;
    let cap = LoopCap { phi: cap_phi, next: Val::Undef };
    let (outer_kept, outer_body, outer_loops, outer_guard, outer_cap) = (f.kept, f.kept_body, f.loops, f.guard, f.cap);
    let first_break = f.nbreaks;
    (f.kept, f.kept_body, f.loops, f.guard, f.cap) =
        (Some(lp), (nodes.as_ptr() as usize, nodes.len()), 0, u32::MAX, Some(cap));
    let flow = walk_nodes(f, m, nodes, vals, locals, prec, false, last, None);
    let cap = f.cap;
    (f.kept, f.kept_body, f.loops, f.guard, f.cap) = (outer_kept, outer_body, outer_loops, outer_guard, outer_cap);
    /* a body that always breaks runs once: no reference */
    if flow? == Flow::Break {
        return Err(MaliError::LoopForm);
    }
    /* the head test took the counter ([`walk_nodes`]) */
    let Some(LoopCap { next: next @ Val::Lane(..), .. }) = cap else { return Err(MaliError::LoopForm) };
    f.nir.nodes[cap_phi as usize].src[1] = next;
    let latch = f.nir.cur;
    for &(id, l, c) in &phis[..nphi] {
        let v = locals[usize::from(l)][usize::from(c)];
        /* a lane the body leaves as it found it (`nir_opt_remove_phis`) */
        if v == Val::Lane(id, 0) {
            return Err(MaliError::LoopForm);
        }
        f.nir.nodes[id as usize].src[1] = v;
    }
    /* header phis of equal sources are one (`nir_opt_cse` of phis, the
     * first kept): divnest's `outer` and `i`, both `phi(0, i + 1)` once the
     * step's `i + 1` is the one of `outer = i + 1` */
    for j in 1..nphi {
        let (pj, ..) = phis[j];
        let src = |id: u16| {
            let s = f.nir.nodes[id as usize].src;
            (s[0], s[1])
        };
        let Some(&(pi, ..)) = phis[..j].iter().find(|p| src(p.0) == src(pj)) else { continue };
        let (from, to) = (Val::Lane(pj, 0), Val::Lane(pi, 0));
        for nd in f.nir.nodes[..f.nir.n].iter_mut() {
            for s in nd.src.iter_mut().filter(|s| **s == from) {
                *s = to;
            }
        }
        for v in locals.iter_mut().flatten().filter(|v| **v == from) {
            *v = to;
        }
        for k in first_break..f.nbreaks {
            let slot = usize::from(f.breaks[k].slot);
            for v in f.table_mut(slot).iter_mut().flatten().filter(|v| **v == from) {
                *v = to;
            }
        }
    }
    /* the exit is entered by the `break`s alone: two, or one (no `break`
     * is a loop that never ends; more have no reference) */
    let mut edges = [BreakEdge::NONE; 2];
    let mut dom: Option<u8> = None;
    let mut nedges = 0;
    for b in f.breaks[first_break..f.nbreaks].iter().filter(|b| b.lp == lp) {
        if nedges == edges.len() {
            return Err(MaliError::LoopForm);
        }
        dom = Some(dom.map_or(b.block, |d| f.nir.lca(d, b.block)));
        edges[nedges] = *b;
        nedges += 1;
    }
    let Some(dom) = dom else { return Err(MaliError::LoopForm) };
    f.nbreaks = first_break;
    let exit = f.nir.new_block(dom)?;
    f.nir.cur = exit;
    let (e0, e1) = (usize::from(edges[0].slot), usize::from(edges[1].slot));
    f.load_locals(e0, locals);
    f.colour = edges[0].colour;
    if nedges == 2 {
        /* the exit's phis, a lane the edges leave different, in the order
         * an `if`'s join has them ([`walk_if`]), each taking the edges'
         * values in the order their jumps were added (divnest's outer exit:
         * `n` and `outer`, whose copies stand before each edge's `JUMP`) */
        if edges[1].colour != f.colour {
            return Err(MaliError::LoopForm);
        }
        /* the lanes the edges leave different, by (rank, local) from the
         * smallest up: a scan, not a table on this frame */
        let mut prev = None;
        loop {
            let next = (0..f.nl)
                .filter(|&l| f.table(e0)[l] != f.table(e1)[l])
                .map(|l| (f.rank[l], l))
                .filter(|k| prev.is_none_or(|p| *k > p))
                .min();
            let Some(key) = next else { break };
            prev = Some(key);
            let l = key.1;
            for c in 0..4 {
                let (a, b) = (f.table(e0)[l][c], f.table(e1)[l][c]);
                locals[l][c] = f.phi(a, b, last)?;
            }
        }
    }
    let seq = f.nir.nseq;
    f.nir.nseq += 1;
    f.nir.loops[lp as usize] = nir::LoopRegion { pre, header, latch, exit, seq };
    Ok(())
}

/// The nodes of one run: `main`'s body (`top`), a side of an `if`, or a
/// pass of a loop being unrolled. `after` is where an `if` that ends the
/// run joins: the instruction the run's successor begins with (the
/// program's end for `main`, [`walk_if`] for a side). The run ends by
/// falling through or by the `break` or `continue` it met ([`Flow`]).
/// `lp` is the loop when the run is (the rest of) one of its passes: there
/// an `if` with a run-time condition may `break` ([`walk_guard`]) or
/// `continue` ([`IfExit::Continue`]).
#[allow(clippy::too_many_arguments)]
fn walk_nodes(
    f: &mut Fs<'_>,
    m: &Module<'_>,
    nodes: &[Node],
    vals: &mut [[Val; 4]],
    locals: &mut [[Val; 4]; MAX_LOCALS],
    prec: FsPrecision,
    top: bool,
    after: usize,
    lp: Option<Unroll<'_>>,
) -> Result<Flow, MaliError> {
    for (k, n) in nodes.iter().enumerate() {
        let id = match *n {
            Node::Instr(id) => id,
            /* a `return` that ends `main` is the end of the program */
            Node::Return if top && k + 1 == nodes.len() => continue,
            Node::If { cond, then_, else_ } => {
                let c = vals.get(cond.val.0 as usize).map_or(Val::Undef, |v| v[cond.swizzle[0] as usize & 3]);
                /* the join's place: the instruction after the `if`, or the
                 * run's successor */
                let join = match nodes.get(k + 1) {
                    Some(Node::Instr(id)) => id.0 as usize,
                    _ => after,
                };
                /* `if (c) discard;` with no else is one `demote_if`
                 * ([`Fs::discard`]) */
                let lone_discard = m.nodes_of(then_).iter().find_map(|n| match *n {
                    Node::Instr(id) if matches!(m.instr(id), Some(ir::Op::Discard)) => Some(id.0 as usize),
                    _ => None,
                });
                if let (Some(at), 1, true) = (lone_discard, then_.len, else_.is_empty()) {
                    f.discard(c, at)?;
                    continue;
                }
                /* a condition the walk folded (an unrolled loop's test on
                 * its counter): the side it picks runs in place, the other
                 * is dead (`nir_opt_dead_cf` after the folding) */
                if let Val::Const(mask) = c {
                    let side = if mask != 0 { then_ } else { else_ };
                    match walk_nodes(f, m, m.nodes_of(side), vals, locals, prec, false, join, None)? {
                        Flow::Next => continue,
                        flow => return Ok(flow),
                    }
                }
                let (t, e) = (m.nodes_of(then_), m.nodes_of(else_));
                if let (Some(lp), true) = (lp, e.is_empty()) {
                    /* `if (c) break;`: the guard the rest of the loop
                     * nests in */
                    if t == [Node::Break] {
                        return walk_guard(f, m, c, cond.val, &nodes[k + 1..], lp, vals, locals, prec);
                    }
                    /* `if (c) { ...; continue; }`: the rest of the pass is
                     * its else side (`opt_loop_last_block`, nir_opt_loop.c:
                     * 208-300), the join the pass's end */
                    if t.last() == Some(&Node::Continue) {
                        let rest = &nodes[k + 1..];
                        return walk_if(f, m, c, t, rest, IfExit::Continue, vals, locals, prec, after);
                    }
                }
                /* `if (c) {} else break;` (the front end's test of a loop,
                 * `Lower::break_unless`) in a loop Mesa keeps:
                 * `opt_if_simplification` (nir_opt_if.c:741-782) swaps the
                 * sides under `inot(c)`, which `nir_opt_algebraic` folds into
                 * the inverse compare (`inot(ilt(a, b))` is `ige(a, b)`) */
                if f.kept.is_some() && f.loops == 0 && t.is_empty() && e == [Node::Break] {
                    let mut c = f.inverse_compare(c)?;
                    /* the loop's head test (the body's first `if`,
                     * [`kept_loop`]) also leaves once the hidden counter
                     * runs out ([`loop_cap_limit`]): `count - 1`, its
                     * compare with 0, OR-ed into the mask */
                    let head = f.kept_body == (nodes.as_ptr() as usize, nodes.len());
                    if let Some(cap) = f.cap.filter(|cap| head && cap.next == Val::Undef) {
                        let at = cond.val.0 as usize;
                        let next = f.op32(Alu::Iadd, [Val::Lane(cap.phi, 0), Val::Const(u32::MAX), Val::Undef], at)?;
                        let stop = f.op32(Alu::Icmp(Cmpf::Lt), [next, Val::Const(0), Val::Undef], at)?;
                        c = f.op32(Alu::Ior, [c, stop, Val::Undef], at)?;
                        f.cap = Some(LoopCap { next, ..cap });
                    }
                    walk_if(f, m, c, e, &[], IfExit::Plain, vals, locals, prec, join)?;
                    continue;
                }
                /* `if (c) { step; continue; }` at the top of a kept loop's
                 * body, the front end's copy of the `for` step before the
                 * `continue` (`Lower::loop_body`): Mesa's NIR has the step
                 * once, in the loop's continue construct, which
                 * `nir_lower_continue_constructs` inlines at the body's end
                 * after moving the code that follows the `continue`'s `if`
                 * into its other side (`lower_jumps_in_if`,
                 * nir_lower_continue_constructs.c:91-140); the then side left
                 * empty, `opt_if_simplification` swaps the sides under the
                 * inverse compare. So: `if (!c) { rest } else {}`, then the
                 * step, in the latch (intloop_divnest.expect.json: `i != 2`
                 * branches over the inner loop to the outer latch, `j != 1`
                 * over the inner body to the `j++` of the inner latch). A
                 * then side with more than the step has no reference */
                if f.kept.is_some() && f.loops == 0 && e.is_empty() && t.last() == Some(&Node::Continue) {
                    if f.kept_body != (nodes.as_ptr() as usize, nodes.len()) {
                        return Err(MaliError::LoopForm);
                    }
                    let rest = &nodes[k + 1..];
                    let s = &t[..t.len() - 1];
                    if rest.len() < s.len() || !same_instrs(m, s, &rest[rest.len() - s.len()..]) {
                        return Err(MaliError::LoopForm);
                    }
                    let (moved, step) = rest.split_at(rest.len() - s.len());
                    let join = match step.first() {
                        Some(Node::Instr(id)) => id.0 as usize,
                        _ => after,
                    };
                    let c = f.inverse_compare(c)?;
                    if walk_if(f, m, c, moved, &[], IfExit::Plain, vals, locals, prec, join)? != Flow::Next {
                        return Err(MaliError::LoopForm);
                    }
                    return walk_nodes(f, m, step, vals, locals, prec, false, after, None);
                }
                walk_if(f, m, c, t, e, IfExit::Plain, vals, locals, prec, join)?;
                continue;
            }
            Node::Loop { body } if kept_loop(f, m, body, locals) => {
                walk_kept_loop(f, m, body, vals, locals, prec)?;
                continue;
            }
            Node::Loop { body } => {
                walk_loop(f, m, body, vals, locals, prec)?;
                continue;
            }
            Node::Break if f.loops > 0 || f.kept.is_some() => return Ok(Flow::Break),
            Node::Continue if f.loops > 0 => return Ok(Flow::Continue),
            _ => return Err(MaliError::ControlFlow),
        };
        let op = m.instr(id).ok_or(MaliError::Defect(Defect::IrIndex))?;
        let lane = |vals: &[[Val; 4]], s: ir::Src, i: usize| -> Val {
            vals.get(s.val.0 as usize).map_or(Val::Undef, |v| v[s.swizzle[i] as usize & 3])
        };
        let at = id.0 as usize;
        let mut out = [Val::Undef; 4];
        /* a uniform array read at an index the walk folded to a constant
         * (an unrolled loop's counter): Mesa's unrolling makes the deref's
         * index a constant before `nir_lower_uniforms_to_ubo`, so the read
         * is the constant-index one (deqp_uniarray_loop_*_fs, byte-equal
         * to their `*_const_fs`); the index clamped as the IR reads it */
        let folded;
        let op = match *op {
            ir::Op::LoadUniformIdx { base_dw, stride_dw, len, index, comps } if len > 0 => match lane(vals, index, 0) {
                Val::Const(k) => {
                    let k = (k as i32).clamp(0, i32::from(len) - 1) as u16;
                    folded = ir::Op::LoadUniform { dword_off: base_dw + k * stride_dw, comps };
                    &folded
                }
                _ => op,
            },
            _ => op,
        };
        match *op {
            ir::Op::ConstF32(bits) => out = bits.map(Val::Const),
            /* a 32-bit integer constant is its bits (NIR's `load_const`
             * has no type; femtovg's `texType == 1`), a boolean NIR's
             * 32-bit 0 / ~0 (`pan_nir_lower_bool_to_bitsize`): values the
             * walk folds ([`nir::fold_int`]; case 72's `ivec2(1, 2)` and
             * `bvec2(1, 0)`, case 112's side-effect flags) */
            ir::Op::ConstI32(bits) => out = bits.map(|b| Val::Const(b as u32)),
            ir::Op::ConstBool(bits) => out = bits.map(|b| Val::Const(if b { u32::MAX } else { 0 })),
            ir::Op::LoadInput { slot: IoSlot::Varying(v), comps } => {
                /* a mediump varying of a highp shader is an fp16 load
                 * (nir_lower_mediump_io, bifrost_nir.c:914-916) */
                f.mixed |= prec.highp && prec.other >> (v & 31) & 1 != 0;
                /* a lane the vertex shader stores as a constant is that
                 * constant ([`super::link::FsConsts`]): the load is of
                 * the other lanes alone, when one is left */
                let konst = |c: usize| prec.consts.get(v, c as u8);
                let n = if (0..comps as usize).all(|c| konst(c).is_some()) { None } else { Some(f.input(v, at)?) };
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = match (konst(c), n) {
                        (Some(bits), _) => Val::Const(bits),
                        (None, Some(n)) => Val::Lane(n, c as u8),
                        (None, None) => return Err(MaliError::Defect(Defect::IrIndex)),
                    };
                }
            }
            ir::Op::LoadInput { slot: IoSlot::PointCoord, comps } => {
                let n = f.input(POINT_COORD, at)?;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* gl_FragCoord is highp in ES 3.00 and mediump in ES 1.00: a
             * mediump ES 1.00 shader's read is narrowed to fp16
             * ([`Fs::frag_coord16`]); a mediump ES 3.00 shader's is the
             * fp32 lanes, an island of fp32 in the fp16 shader ([`Fs::fc32`];
             * case 218 shadowcmp, fsarith_218_shadowcmp.expect.json,
             * docs/notes/mali-fragcoord-fp16-ops.md), its z and w
             * [`Kind::FragCoordZw`]'s */
            /* a highp shader's z is the special varying's fp32 lane
             * ([`FRAG_Z`], loaded only when read, [`layout`]): Mesa reads it
             * at fp32 where a highp operation does, as the position's x and
             * y (`load_var_special_pan` is 32-bit, bifrost_compile.c:1428) */
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } if prec.highp => {
                out = f.frag_coord(at)?;
                out[2] = Val::Lane(f.input(FRAG_Z, at)?, 0);
            }
            /* an ES 1.00 read a highp local takes whole ([`highp_copy`]):
             * lower_precision narrows no bare dereference
             * (lower_precision.cpp:835-843), so the local holds the fp32
             * lanes and every operation on them is fp32 until a mediump
             * variable takes the result (gm2_cf_ifelse_fs.expect.json:
             * `FMA.f32` x2, `FROUND.f32.rtn`, `FADD.f32`, then `FADD.f32
             * r0.h0, r0^, 0x0.neg`) */
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } if !prec.frag_coord_highp && highp_copy(m, id.0) => {
                f.fc_copy = true;
                out = f.frag_coord(at)?;
            }
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } if !prec.frag_coord_highp => out = f.frag_coord16(at)?,
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } => out = f.frag_coord(at)?,
            /* `load_front_face`: `ICMP_OR.u32.eq.m1` of the primitive
             * flags and 0, the word whole on arch 10 (bifrost_compile.c:
             * 2017-2029; builtin_49.expect.json's `ICMP_OR.u32.eq.m1 r0,
             * r58^, 0x0, 0x0`), a 32-bit mask in either precision */
            ir::Op::LoadInput { slot: IoSlot::FrontFacing, .. } => {
                let before = f.nir.n;
                let n = f.nir.load(Kind::Preload { reg: super::ra::PRIMITIVE_FLAGS })?;
                if f.nir.n > before {
                    f.origin[n as usize] = at as u16;
                }
                let u = Val::Undef;
                out[0] = f.op32(Alu::Icmp(Cmpf::Eq), [Val::Lane(n, 0), Val::Const(0), u], at)?;
            }
            ir::Op::LoadInput { .. } => return Err(MaliError::SpecialInput),
            ir::Op::LoadLocal { local, index: None, comps } => {
                f.rank_local(local.0 as usize)?;
                let s = locals.get(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))?;
                out[..comps as usize].copy_from_slice(&s[..comps as usize]);
            }
            ir::Op::StoreLocal { local, index: None, src, write_mask } => {
                f.rank_local(local.0 as usize)?;
                let mut lanes = [Val::Undef; 4];
                for (i, t) in lanes.iter_mut().enumerate() {
                    *t = lane(vals, src, i);
                }
                /* an fp16 value stored to a highp local is converted at the
                 * assignment ([`Fs::widen`]; prec_82's `vec2 p =
                 * floor(gl_FragCoord.xy)`, `F16_TO_F32` of each half). A
                 * local of a highp shader is highp: one declared at the
                 * other width is refused ([`FsPrecision::mixed`]) */
                if prec.highp {
                    for (i, t) in lanes.iter_mut().enumerate() {
                        if write_mask & (1 << i) != 0 {
                            *t = f.widen(*t, at)?;
                        }
                    }
                }
                /* the other way in a mediump shader: an fp32 value of the
                 * highp gl_FragCoord's island ([`Fs::fc32_float`]) stored to a
                 * declared mediump variable is narrowed at the assignment
                 * (lower_precision retypes the variable, lower_precision.cpp:
                 * 1023-1060, and converts at the edge of the tree it cannot
                 * lower). A constructor's temporary (the front end's unnamed
                 * local) takes its arguments' precision (GLSL ES 3.00 4.5.2)
                 * and keeps the fp32 lanes until they are copied to the
                 * variable: case 218's `vec2 b = vec2(floor(..), floor(..))`
                 * is `FADD.f32 rN.h0, rN^, 0x0.neg` twice after both floors
                 * (fsarith_218_shadowcmp.expect.json). A boolean's float
                 * (`b2f`, a select of constants) has no precision and is
                 * read where it is */
                /* So is an integer's float ([`Fs::fp32_lane`]: `float(res)`
                 * stays fp32, lower_precision lowers no conversion from an
                 * int) stored to a declared mediump variable (dEQP's
                 * `color.x = float(res)`, deqp_intval_opfs_add_int_mediump:
                 * `S32_TO_F32 r0, r0^` then `FADD.f32 r0.h0, r0^, 0x0.neg`;
                 * [`Fs::narrowed`], [`int_colour16`]) */
                let temporary = m.locals.get(local.0 as usize).is_some_and(|d| d.name.is_empty());
                /* a local declared highp keeps the fp32 lanes (glmark2's
                 * `highp vec2 FragCoord`, [`highp_copy`]) */
                let highp_local = m.local_prec_of(local) == ir::Prec::High;
                if !prec.highp && !temporary && !highp_local {
                    for (i, t) in lanes.iter_mut().enumerate() {
                        if write_mask & (1 << i) != 0 && (f.fc32_float(*t) || f.int_float(*t)) {
                            *t = f.narrow_stored(*t, at)?;
                        }
                    }
                }
                /* a texel stored to a (highp) local: the look-up returns
                 * fp32 ([`Fs::look_up`]) */
                if prec.highp {
                    for v in lanes {
                        if let Some((j, _)) = f.texel(v) {
                            f.tex32(f.look_ups[j].node)?;
                        }
                    }
                }
                let s = locals.get_mut(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))?;
                for (i, t) in lanes.iter().enumerate() {
                    if write_mask & (1 << i) != 0 {
                        s[i] = *t;
                    }
                }
            }
            ir::Op::StoreOutput { slot, src } => {
                let mut lanes = [Val::Undef; 4];
                for (i, t) in lanes.iter_mut().enumerate() {
                    *t = lane(vals, src, i);
                }
                /* an fp16 value stored to a highp output is converted at the
                 * assignment, lane by lane (lower_precision's `f162f`;
                 * mixed_44_hdrsample's four `F16_TO_F32 rN, rK.h0/.h1`), the
                 * blend the fp32 one */
                if prec.highp && prec.colour_f32 {
                    for t in lanes.iter_mut() {
                        *t = f.widen_store(*t, at)?;
                    }
                } else if prec.highp {
                    /* stored to a mediump colour as they are: no reference
                     * but one texel's product by a constant, an operation
                     * of one lane ([`Fs::alu16`], gm2_texswz_2) */
                    f.mixed |= lanes
                        .iter()
                        .any(|&t| f.texel16_product(t, 0) && !f.lone_texel_product(t) && !f.texel_pair_sum(t));
                }
                match slot {
                    IoSlot::FragColor(0) => {
                        if f.colour_rank == u32::MAX {
                            f.colour_rank = f.next_rank;
                            f.next_rank += 1;
                        }
                        f.colour = Some(lanes)
                    }
                    IoSlot::FragColor(n) if (n as usize) < MAX_TARGETS => f.targets[n as usize - 1] = Some(lanes),
                    _ => return Err(MaliError::FragOutput),
                }
            }
            ir::Op::LoadLocal { .. } | ir::Op::StoreLocal { .. } => return Err(MaliError::IndexedLocal),
            /* fp32: one scalar node a lane, as the vertex stage builds it */
            ir::Op::Alu(a, s) if prec.highp => {
                /* an operation on the mediump built-in itself is mediump
                 * (lower_precision.cpp:295-312: its precision is its
                 * operands'), so Mesa computes it in fp16 when every other
                 * operand is a constant or such an fp16 result
                 * ([`Fs::alu16`]); with a highp operand it is fp32, the
                 * read taken at fp32 as a highp copy's (a highp local or
                 * parameter) is, and an fp16 operand widened ([`Fs::widen`]) */
                let arity = ir_arity(a);
                let mut fc = [false; 3];
                for (k, x) in fc.iter_mut().enumerate().take(arity) {
                    *x = !prec.frag_coord_highp
                        && matches!(m.def(s[k].val), Some(ir::Op::LoadInput { slot: IoSlot::FragCoord, .. }));
                }
                let w = width_of(m, ir::ValueId(id.0));
                let mut src = [[Val::Undef; 4]; 3];
                for (k, l) in src.iter_mut().enumerate().take(arity) {
                    for (i, v) in l.iter_mut().enumerate().take(w) {
                        *v = lane(vals, s[k], i);
                    }
                }
                /* the lanes an operand has (a compare's and a conversion's
                 * width is its sources', [`Fs::alu32_lanes`]) */
                let n = (0..w).take_while(|&i| src[0][i] != Val::Undef).count();
                /* a lowp or mediump sampler's texel is fp16 (lower_precision's
                 * `visit_enter(ir_texture)`: the look-up takes its sampler's
                 * precision; hdrsample's `texture(hdr, ..) * 0.25`) */
                /* and a sum of two such look-ups' texels (gm2_texcoord_4's
                 * `texture2D(s0, v.xy) + texture2D(s1, v.zw)`: `FADD.v2f16`
                 * of the two results' words); a texel plus a constant or a
                 * highp value has no reference */
                let texels = (0..arity).all(|k| (0..n).all(|i| f.texel16(src[k][i])));
                let texel = |f: &Fs<'_>, v: Val| (a == AluOp::Fmul || (a == AluOp::Fadd && texels)) && f.texel16(v);
                let low = |f: &Fs<'_>, k: usize| fc[k] || (0..n).any(|i| f.is_f16(src[k][i]) || texel(f, src[k][i]));
                let konst = |k: usize| (0..n).all(|i| matches!(src[k][i], Val::Const(_)));
                if (0..arity).any(|k| low(f, k)) && (0..arity).all(|k| low(f, k) || konst(k)) {
                    /* a form [`Fs::alu16`] does not take on no gl_FragCoord
                     * read: fp16 arithmetic of a mixed shader */
                    let frag = fc.iter().any(|&b| b);
                    out = f.alu16(a, src, fc, n, at).map_err(|e| match e {
                        MaliError::FragCoordMediump if !frag => MaliError::MixedPrecision,
                        e => e,
                    })?;
                } else {
                    out = f.alu32(m, a, &s, vals, at)?
                }
            }
            /* a select of two fp16 values on the island's mask (case 218's
             * `right > 0.5 ? l : n`, whose compare NIR folds into `step`'s
             * own ([`Fs::lane32`]), the texels its sides): the mask's low
             * half (`i2i16`, `pan_nir_lower_bool_to_bitsize`) tests a
             * 16-bit `bcsel_pan`, `MUX.v2i16 r0, r1^.h00, r0^.h00, r2^.h00`
             * (fsarith_218_shadowcmp.expect.json) */
            ir::Op::Alu(AluOp::Bcsel, s)
                if !prec.highp
                    && f.fc32(lane(vals, s[0], 0))
                    && [1, 2].iter().all(|&k| {
                        let v = lane(vals, s[k], 0);
                        f.is_f16(v) || f.texel16(v)
                    }) =>
            {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (c, y, z) = (lane(vals, s[0], i), lane(vals, s[1], i), lane(vals, s[2], i));
                    if c == Val::Undef || y == Val::Undef || z == Val::Undef {
                        if i > 0 {
                            break;
                        }
                        return Err(MaliError::ColourForm);
                    }
                    *o = f.node(Alu::Bcsel, [c, y, z], at)?;
                }
            }
            /* a mediump ES 3.00 shader's operation on the highp gl_FragCoord
             * or on a value computed from it ([`Fs::fc32`]): lower_precision
             * leaves the whole tree at fp32, so it is a highp shader's
             * operation ([`Fs::alu32`]: NIR's rules on fp32 values), its fp16
             * operands widened ([`Fs::op32`]) */
            ir::Op::Alu(a, s)
                if !prec.highp && (0..ir_arity(a)).any(|k| (0..4).any(|i| f.fc32(lane(vals, s[k], i)))) =>
            {
                out = f.alu32(m, a, &s, vals, at)?;
            }
            /* fp16 arithmetic on the island's narrowed values ([`Fs::narrowed`];
             * case 218's `(b + 0.5) / 4.0` and `(b + 1.0) / 4.0`): the two
             * halves one vector (`vec2`, `MKVEC.v2i16 r0, r0^.h0, r1^.h0`), the
             * operation one of two lanes as a highp shader's fp16 island
             * ([`Fs::alu16`]: `FADD_IMM.v2f16 r1, r0, #0x38003800`, `FMA.v2f16
             * r1, r1^, 0x34000000.h11, 0x0.neg`) */
            /* and on fp16 results of one lane of those (glmark2's `d =
             * fract(3.0 * d)` steps after the narrowed head, [`Fs::island16`]) */
            ir::Op::Alu(a, s)
                if !prec.highp
                    && (0..ir_arity(a)).any(|k| {
                        (0..4).any(|i| {
                            let v = lane(vals, s[k], i);
                            f.narrowed(v) || f.island16(v, 0)
                        })
                    })
                    && (0..ir_arity(a)).all(|k| {
                        (0..4).all(|i| {
                            let v = lane(vals, s[k], i);
                            v == Val::Undef || matches!(v, Val::Const(_)) || f.narrowed(v) || f.is_f16(v)
                        })
                    }) =>
            {
                let w = width_of(m, ir::ValueId(id.0));
                let mut src = [[Val::Undef; 4]; 3];
                for (k, l) in src.iter_mut().enumerate().take(ir_arity(a)) {
                    for (i, v) in l.iter_mut().enumerate().take(w) {
                        *v = lane(vals, s[k], i);
                    }
                }
                out = f.alu16(a, src, [false; 3], w, at)?;
            }
            /* a product of compare-function results of a mediump shader
             * (dEQP uniform_api's `compare_vecN`, `compare_float(..) *
             * compare_float(..)` of `abs(a - b) < 0.05 ? 1.0 : 0.0`): the
             * mediump return temporary narrows each select (`f2fmp(b2f32(a))`
             * is `b2f16(a)`, nir_opt_algebraic.py:1980), `fmul(b2f(a),
             * b2f(b))` is `b2f(iand(a, b))` (:1679) and `iand(flt(a, c),
             * flt(b, c))` is `flt(fmax(a, b), c)` (:1159): one 16-bit compare
             * of an `FMAX` tree (deqp_mixprec_cmpv4). The product with another
             * mediump value is `fmul(b2f16(a), b)` ([`Fs::op`]); with an fp32
             * value it is the next arm's */
            ir::Op::Alu(AluOp::Fmul, s)
                if (0..m.comps_of(ir::ValueId(id.0)) as usize).all(|i| {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    let fp32 = |v: Val| f.fp32_lane(v) || (f.select32(v) && f.cmp_select(v).is_none());
                    let both = f.cmp_select(x).is_some() && f.cmp_select(y).is_some();
                    (f.cmp_select32(x) || f.cmp_select32(y) || both) && !fp32(x) && !fp32(y)
                }) =>
            {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                let u = Val::Undef;
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    *o = match (f.cmp_select(x), f.cmp_select(y)) {
                        (Some(_), Some(_)) => f.cmp_product(x, y, at)?,
                        (Some(a), None) | (None, Some(a)) => {
                            let other = if f.cmp_select(x).is_some() { y } else { x };
                            let b = f.node(Alu::B2f, [a, u, u], at)?;
                            match other {
                                Val::Const(F32_ONE) => b,
                                _ => {
                                    let other = f.fp16_operand(other);
                                    f.op(AluOp::Fmul, other, b, at, Rule::None, None)?
                                }
                            }
                        }
                        (None, None) => return Err(MaliError::Defect(Defect::IrIndex)),
                    };
                }
            }
            /* a product or sum on an fp32 value of a mediump shader (an
             * integer's conversion, case 79's `float(q.x) / 4.0`): lower_
             * precision leaves it at 32 bits, an operation the vertex stage
             * builds (intbig_case79.stderr: `fmul %124, 0.25` at 32 bits,
             * narrowed at the store) */
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s)
                if (0..width_of(m, ir::ValueId(id.0))).any(|i| {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    f.fp32_lane(x) || f.fp32_lane(y) || (f.select32(x) && f.select32(y))
                }) =>
            {
                let op = if a == AluOp::Fmul { Alu::Fmul } else { Alu::Fadd };
                let w = width_of(m, ir::ValueId(id.0));
                /* an fp16 operand is widened (lower_precision's `f162f` at
                 * the edge of the lowered tree: case 42's `uv.x *
                 * float(sz.x)`, `F16_TO_F32` then `FMA.f32`) */
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    let ok = |v: Val| f.fp32_lane(v) || f.select32(v) || f.is_f16(v) || matches!(v, Val::Const(_));
                    if !ok(x) || !ok(y) {
                        return Err(MaliError::MixedPrecision);
                    }
                    *o = f.op32(op, [x, y, Val::Undef], at)?;
                }
            }
            /* a sum or product of constants (an unrolled loop's `r +=
             * 0.25`, case 72): `nir_opt_constant_folding` at fp16, each
             * operand rounded to fp16 first (lower_precision's constants)
             * and the result rounded (gm2_fsmisc_6's `mix(FILL, LINE, I)`,
             * `1.0 - 0.8` the `0x3268` of `u3.w0`: 0.8 is `0x3A66` first) */
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s)
                if (0..width_of(m, ir::ValueId(id.0)))
                    .all(|i| matches!((lane(vals, s[0], i), lane(vals, s[1], i)), (Val::Const(_), Val::Const(_)))) =>
            {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (Val::Const(p), Val::Const(q)) = (lane(vals, s[0], i), lane(vals, s[1], i)) else {
                        return Err(MaliError::Defect(Defect::IrIndex));
                    };
                    *o = Val::Const(fold_const16(a, p, q)?);
                }
            }
            /* a one-lane product of a negated literal the GLSL passes left
             * (glsl.rs `neg_products`: the literal first, `-2.0 * d`):
             * `fneg(fmul(2.0, d))`, the constant first, as that pass rewrites
             * a placed one (gm2_fsmisc_6: `FADD.v2f16 r1, r0.h00, r0.h00`,
             * the late `fadd(d, d)`, then the product and `FADD.v2f16 r0,
             * r0^.neg.h00, 0x0.neg`) */
            ir::Op::Alu(AluOp::Fmul, s) if width_of(m, ir::ValueId(id.0)) == 1 && neg_literal(m, &s).is_some() => {
                let k = neg_literal(m, &s).ok_or(MaliError::Defect(Defect::IrIndex))?;
                let (c, v) = (lane(vals, s[k], 0), f.fp16_operand(lane(vals, s[1 - k], 0)));
                let (Val::Const(c), Val::Lane(..)) = (c, v) else { return Err(MaliError::ArithConstant) };
                let p = f.node(Alu::Fmul, [Val::Const(c ^ F32_NEG_ZERO), v, Val::Undef], at)?;
                out[0] = f.neg(p, at)?;
            }
            /* `smoothstep`'s last product: the front end's `(t * t) * (3.0 -
             * 2.0 * t)` is Mesa's `t * (t * (3.0 - 2.0 * t))`
             * (builtin_functions.cpp `_smoothstep`; gm2_fsmisc_4: `FMA.v2f16
             * r1, r0.h00, r1^.h00, 0x0.neg`, then `FMA.v2f16 r0, r0^.h00,
             * r1^.h00, 0x0.neg`) */
            ir::Op::Alu(AluOp::Fmul, s)
                if (0..width_of(m, ir::ValueId(id.0)))
                    .all(|i| f.smoothstep_tail(lane(vals, s[0], i), lane(vals, s[1], i)).is_some()) =>
            {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    let t = f.smoothstep_tail(x, y).ok_or(MaliError::Defect(Defect::IrIndex))?;
                    let ti = f.node(Alu::Fmul, [t, y, Val::Undef], at)?;
                    *o = f.node(Alu::Fmul, [t, ti, Val::Undef], at)?;
                }
            }
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s) => {
                /* the lanes the operands have: the front end's 4-wide
                 * operation on a vec3 (`normalize(light)` of a `vec3`
                 * uniform, gm2_fsarith_1) leaves the fourth undefined */
                let w = width_of(m, ir::ValueId(id.0));
                let defined =
                    (0..w).take_while(|&i| lane(vals, s[0], i) != Val::Undef && lane(vals, s[1], i) != Val::Undef);
                let w = match defined.count() {
                    0 => w,
                    n => n,
                };
                /* the rules run on 16-bit lane pairs: a rule holds for a
                 * pair when both its lanes' constants make it */
                let ident: &[u32] = if a == AluOp::Fmul { &[F32_ONE] } else { &[0, F32_NEG_ZERO] };
                /* an identity constant on every lane of the operation:
                 * nir_opt_algebraic folds it at the link already (`fmul(a,
                 * 1.0)`, `fadd(a, 0.0)`, nir_opt_algebraic.py:292, :236-237,
                 * in gl_nir_opts, gl_nir_linker.c:102; a constant source
                 * matches when every lane it reads is the constant, the
                 * sign of zero checked per lane, nir_search.c:260-286 and
                 * :282-285, so `+ vec2(0.0, -0.0)` is no identity there):
                 * f07's `a.x * 1.0` is the colour's own lane, converted at
                 * the store */
                let whole =
                    (0..2).any(|k| ident.iter().any(|&b| (0..w).all(|i| f.is_const(lane(vals, s[k], i), &[b]))));
                /* zeros of both signs on every lane (`+ vec2(0.0, -0.0)`):
                 * no fold at the link by the source, and no measured shape
                 * tells the tree's conversions from the colour's (n20's
                 * order is both): refused */
                let mixed = (0..2).any(|k| (0..w).all(|i| f.is_const(lane(vals, s[k], i), ident)));
                if a == AluOp::Fadd && mixed && !whole {
                    return Err(MaliError::FsVaryingLoad);
                }
                for pair in (0..w).step_by(2) {
                    let lanes = pair..w.min(pair + 2);
                    let all = |k: usize, bits: &[u32]| lanes.clone().all(|i| f.is_const(lane(vals, s[k], i), bits));
                    let rule = if all(1, ident) {
                        Rule::IdentitySecond
                    } else if all(0, ident) {
                        Rule::IdentityFirst
                    } else if a == AluOp::Fmul && (all(1, &[0, F32_NEG_ZERO]) || all(0, &[0, F32_NEG_ZERO])) {
                        Rule::Zero
                    } else if a == AluOp::Fmul && (all(1, &[F32_MINUS_ONE]) || all(0, &[F32_MINUS_ONE])) {
                        Rule::Negate
                    } else {
                        Rule::None
                    };
                    /* a pair whose every lane is an operation on two
                     * constants: `nir_opt_constant_folding` of the 16-bit
                     * vec2 that `nir_lower_alu_width` left (bi_vectorize_filter),
                     * each lane `f16_rtne(op(half(a), half(b)))` as the
                     * all-constant arm above. A pair with a computed lane
                     * keeps its constant lane an ALU lane
                     * (deqp_constop_op_degrees: lanes z, w the FAU word
                     * 0x38523800, lanes x, y an `FMA.v2f16`) */
                    if lanes
                        .clone()
                        .all(|i| matches!((lane(vals, s[0], i), lane(vals, s[1], i)), (Val::Const(_), Val::Const(_))))
                    {
                        for i in lanes.clone() {
                            let (Val::Const(p), Val::Const(q)) = (lane(vals, s[0], i), lane(vals, s[1], i)) else {
                                return Err(MaliError::Defect(Defect::IrIndex));
                            };
                            out[i] = Val::Const(fold_const16(a, p, q)?);
                        }
                        continue;
                    }
                    let negs = |f: &Fs<'_>, k: usize| {
                        lanes.clone().all(|i| matches!(f.alu_of(lane(vals, s[k], i)), Some((Alu::Fneg, _))))
                    };
                    let vec_neg = lanes.len() == 1
                        || negs(f, 0)
                        || negs(f, 1)
                        || !lanes.clone().any(|i| {
                            matches!((lane(vals, s[0], i), lane(vals, s[1], i)), (Val::Const(_), Val::Const(_)))
                        });
                    for i in lanes.clone() {
                        let (x, y) = (f.fp16_operand(lane(vals, s[0], i)), f.fp16_operand(lane(vals, s[1], i)));
                        out[i] = f.op(a, x, y, at, rule, Some(vec_neg))?;
                        /* an identity on some lanes of the operation only:
                         * at the link the operation is still an fp16 one
                         * whose operand the tree's `f2fmp` converts
                         * ([`Fs::folded`]) */
                        if matches!(rule, Rule::IdentitySecond | Rule::IdentityFirst) && !whole {
                            f.fold(out[i]);
                        }
                    }
                }
            }
            /* a mediump compare: one 16-bit lane each (`flt`, `fge` with
             * GLSL's `<=` swapped, `feq`; the mask a discard tests,
             * [`Fs::discard`]) */
            ir::Op::Alu(a @ (AluOp::Flt | AluOp::Fle | AluOp::Feq), s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (f.fp16_operand(lane(vals, s[0], i)), f.fp16_operand(lane(vals, s[1], i)));
                    if x == Val::Undef || y == Val::Undef {
                        if i > 0 {
                            break;
                        }
                        return Err(MaliError::ColourForm);
                    }
                    /* an unrolled float counter's test (`for (float i = 0.0; i
                     * < 8.0; ..)`, cflow_71_deadloop) folds as an int's
                     * ([`nir::fold_int`]) */
                    if matches!((x, y), (Val::Const(_), Val::Const(_))) {
                        *o = nir::fold_int(a, x, y).ok_or(MaliError::ConstantFold)?;
                        continue;
                    }
                    /* a -0.0 operand is the compare's new 0.0
                     * (nir_opt_algebraic.py:771-776, the sides already where
                     * glsl.rs `neg_zero_compares` puts them; negzero_const's
                     * `DISCARD.f32.eq r0^.h0, 0x0`) */
                    let pz = |v: Val| if v == Val::Const(F32_NEG_ZERO) { Val::Const(0) } else { v };
                    let (x, y) = (pz(x), pz(y));
                    /* an integer's float on either side (fp32: lower_precision
                     * lowers no conversion from an integer) makes the compare
                     * fp32, an fp16 operand widened (`f2f32`): dEQP's
                     * `isOk(float a, int b)`, `float(b) <= atemp` of the
                     * mediump `atemp = a + 0.5` (deqp_intval_reslib_int_vs:
                     * `F16_TO_F32 r0, r0^.h0`, `FCMP_OR.f32.ge.m1`). Another
                     * operand (a constant, which an algebraic rule may fold
                     * into the integer's compare; a varying's lane) has no
                     * reference */
                    if f.int_float(x) || f.int_float(y) {
                        let local = |k: usize| matches!(m.def(s[k].val), Some(ir::Op::LoadLocal { index: None, .. }));
                        let mut xy = [x, y];
                        for (k, v) in xy.iter_mut().enumerate() {
                            *v = match *v {
                                v if f.int_float(v) => v,
                                v if f.is_f16(v) && local(k) => f.widen_local(v)?,
                                _ => return Err(MaliError::IntValue),
                            };
                        }
                        let [x, y] = xy;
                        let u = Val::Undef;
                        *o = match a {
                            AluOp::Flt => f.op32(Alu::Fcmp(Cmpf::Lt), [x, y, u], at)?,
                            AluOp::Fle => f.op32(Alu::Fcmp(Cmpf::Ge), [y, x, u], at)?,
                            _ => f.op32(Alu::Fcmp(Cmpf::Eq), [x, y, u], at)?,
                        };
                        continue;
                    }
                    *o = match a {
                        AluOp::Flt => f.node(Alu::Fcmp(Cmpf::Lt), [x, y, Val::Undef], at)?,
                        AluOp::Fle => f.node(Alu::Fcmp(Cmpf::Ge), [y, x, Val::Undef], at)?,
                        _ => f.node(Alu::Fcmp(Cmpf::Eq), [x, y, Val::Undef], at)?,
                    };
                }
            }
            ir::Op::Alu(a @ (AluOp::Fneg | AluOp::Fabs), s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    *o = if a == AluOp::Fneg { f.neg(x, at)? } else { f.abs(x, at)? };
                }
            }
            /* `ceil` of a computed lane: `fceil` (cflow_71_deadloop's `%4 =
             * fceil %3.x`, `FROUND.v2f16.rtp r1, r0.h00`); of a constant the
             * front end folds, of anything else no reference */
            ir::Op::Alu(AluOp::Fceil, s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    if f.alu_of(x).is_none() {
                        return Err(MaliError::Arithmetic);
                    }
                    *o = f.node(Alu::Fceil, [x, Val::Undef, Val::Undef], at)?;
                }
            }
            /* `dFdx` / `dFdy` of a varying's lane or of the narrowed
             * gl_FragCoord's x or y (case 138 deriv, fsarith_138_deriv.
             * expect.json; docs/notes/mali-derivatives.md 1): one 16-bit
             * node a lane. `dFdy`'s operand is scaled by the y-flip `T.x`
             * first (`nir_lower_wpos_ytransform`'s `lower_fddy` multiplies
             * the source, `fmul(a, f2f16(T.x))`, the state uniform's lane
             * narrowed once for every `dFdy`: `FADD.f32 r2.h0, u2.w0,
             * 0x0.neg`, then `FMA.v2f16 r1, r1^.h00, r2.h00, 0x0.neg`). The
             * front end gives a scalar call a 4-wide node whose lanes all
             * read the operand ([`Fs::node`] makes them one); a vector
             * derivative (lanes that differ: Mesa's `CLPER` of the pair
             * word) and a constant operand (no fold) have no reference. A
             * highp varying's lane makes the shader mixed
             * ([`Fs::fp16_operand`]); the highp gl_FragCoord's island of an
             * ES 3.00 shader ([`Fs::fc32`]) meets the island's arm first
             * ([`Fs::alu32`]); the operand's form is the pair graph's
             * ([`deriv_operand`]) */
            ir::Op::Alu(a @ (AluOp::Fddx | AluOp::Fddy), s) => {
                let w = width_of(m, ir::ValueId(id.0));
                let first = lane(vals, s[0], 0);
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let mut x = f.fp16_operand(lane(vals, s[0], i));
                    match x {
                        Val::Undef => return Err(MaliError::ColourForm),
                        Val::Const(_) => return Err(MaliError::Arithmetic),
                        _ if x != first => return Err(MaliError::Arithmetic),
                        _ => {}
                    }
                    let u = Val::Undef;
                    if a == AluOp::Fddy {
                        let before = f.nir.n;
                        let t = f.nir.load(Kind::Uniform { dword: FB_WPOS_Y_TRANSFORM, comps: 2 })?;
                        if f.nir.n > before {
                            f.origin[t as usize] = at as u16;
                        }
                        let t16 = f.node(Alu::F2f16, [Val::Lane(t, 0), u, u], at)?;
                        x = f.op(AluOp::Fmul, x, t16, at, Rule::None, None)?;
                    }
                    *o = f.node(if a == AluOp::Fddx { Alu::Fddx } else { Alu::Fddy }, [x, u, u], at)?;
                }
            }
            /* the front end's `mix(a, b, t)` is `ffma(t, fadd(b, fneg(a)),
             * a)`, the `ffma` nir_lower_flrp makes too: case 197's vtl_fs,
             * `FADD.v2f16 r2, r2^, r0.neg` then `FMA.v2f16 r0, u2.w0.h00,
             * r2^, r0^` (tools/mali/testdata/vtl_fs.expect.json) */
            ir::Op::Alu(AluOp::Ffma, s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = [0, 1, 2].map(|k| f.fp16_operand(lane(vals, s[k], i)));
                    if x.contains(&Val::Undef) {
                        /* a vec3's undefined fourth lane ([`Fs::op`]'s arm) */
                        if i > 0 {
                            break;
                        }
                        return Err(MaliError::ColourForm);
                    }
                    if x[1] == Val::Const(F32_ONE) {
                        if let Some(r) = f.reassoc_last(x[0], x[2], at)? {
                            *o = r;
                            continue;
                        }
                    }
                    *o = f.node(Alu::Ffma, x, at)?;
                }
            }
            /* `1.0 / c` of a constant: the GLSL front end folds a quotient
             * of constants (gm2_fsmisc_4's `smoothstep(0.25, 0.75, x)`, whose
             * `(x - e0) / (e1 - e0)` is `(x - 0.25) * 2.0` in Mesa's
             * program), exact in fp16 or refused; of anything else no
             * reference */
            ir::Op::Alu(AluOp::Frcp, s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let Val::Const(c) = lane(vals, s[0], i) else { return Err(MaliError::Arithmetic) };
                    let r = (1.0 / f32::from_bits(c)).to_bits();
                    if f16_exact(c).is_none() || f16_exact(r).is_none() {
                        return Err(MaliError::ConstantFold);
                    }
                    *o = Val::Const(r);
                }
            }
            /* `max` and `min` of a mediump lane and a constant (`clamp(x, lo,
             * hi)` is the front end's `fmin(fmax(x, lo), hi)`; gm2_fsmisc_3:
             * `FMAX.v2f16 r0, r0^, 0x34000000.h11`, then `FMIN.v2f16 r0,
             * r0^, u2.w0`): one 16-bit node a lane. `fmin(fmax(a, 0.0),
             * 1.0)` is `fsat(a)` (nir_opt_algebraic.py:1088), folded into its
             * producer (gm2_fsmisc_4's `FADD.v2f16.clamp_0_1`). Two computed
             * operands, or two constants (no fold here), have no reference */
            ir::Op::Alu(a @ (AluOp::Fmax | AluOp::Fmin), s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (f.fp16_operand(lane(vals, s[0], i)), f.fp16_operand(lane(vals, s[1], i)));
                    if x == Val::Undef || y == Val::Undef {
                        return Err(MaliError::ColourForm);
                    }
                    /* both commute: the constant second (`max(0.0, dot(..))`,
                     * gm2_fsarith_3) */
                    let (x, y) = if matches!(x, Val::Const(_)) { (y, x) } else { (x, y) };
                    let u = Val::Undef;
                    /* of two computed lanes (gm2_fsmisc_6's `min(x * w, min(y
                     * * w, z * w))`: `FMIN.v2f16 r1, r0.h11, r1^.h00`) */
                    if f.is_f16(x) && f.is_f16(y) {
                        *o = f.node(if a == AluOp::Fmax { Alu::Fmax } else { Alu::Fmin }, [x, y, u], at)?;
                        continue;
                    }
                    let Val::Const(k) = y else { return Err(MaliError::Arithmetic) };
                    if matches!(x, Val::Const(_)) {
                        return Err(MaliError::Arithmetic);
                    }
                    *o = match (a, f.alu_of(x)) {
                        /* `fmax(a, 0.0)` is `fclamp_pos(a)` (bifrost_nir_algebraic.py:34),
                         * folded into its producer (gm2_fsarith_1's `max(dot(N, L),
                         * 0.0)`, `FMA.v2f16.clamp_0_inf`) */
                        (AluOp::Fmax, _) if k == 0 => f.node(Alu::Fclamp(clamp::ZERO_INF), [x, u, u], at)?,
                        (AluOp::Fmin, Some((Alu::Fclamp(clamp::ZERO_INF), [b, ..]))) if k == F32_ONE => {
                            f.node(Alu::Fclamp(clamp::ZERO_ONE), [b, u, u], at)?
                        }
                        (AluOp::Fmax, _) => f.node(Alu::Fmax, [x, y, u], at)?,
                        _ => f.node(Alu::Fmin, [x, y, u], at)?,
                    };
                }
            }
            /* `any` and `all` of a bvec: `nir_lower_alu_width`'s reduction
             * from the last lane (`ior(ior(ior(w, z), y), x)`), of bool
             * uniforms' words as 32-bit `ior` / `iand` and of mediump
             * compares' masks as 16-bit ones (intbool_76_boolany:
             * `LSHIFT_OR.i32 r2, u2.w1, r1.b00, u2.w0` of `u_bv.z` and
             * `.y`, `LSHIFT_OR.v2i16 r3, r2.h11, 0x0.b00, r2` of `gt.w` and
             * `.z`). The front end repeats the last lane read into the
             * spare ones (builtin_call.rs) */
            ir::Op::Alu(a @ (AluOp::Bany | AluOp::Ball), s) => {
                let n = (1..=4).find(|&n| s[0].swizzle[n - 1..].iter().all(|&c| c == s[0].swizzle[3])).unwrap_or(4);
                let op = if a == AluOp::Bany { Alu::Ior } else { Alu::Iand };
                let mut acc = lane(vals, s[0], n - 1);
                let mut accs = [Val::Undef; 3];
                for i in (0..n - 1).rev() {
                    let before = f.nir.n;
                    acc = f.mask_op(op, acc, lane(vals, s[0], i), at)?;
                    accs[n - 2 - i] = if f.nir.n > before { acc } else { Val::Undef };
                }
                if acc == Val::Undef {
                    return Err(MaliError::ColourForm);
                }
                /* `all(a == b)` / `any(a != b)` is one GLSL IR operation
                 * (`ir_binop_all_equal`, `ball_iequal`), which
                 * `nir_lower_alu_width`'s `lower_reduction` (nir_lower_alu_width.c:
                 * 96-126) scalarises from the last lane, each lane's compare
                 * right before the merge that reads it: `ieq.z`, `ieq.y`,
                 * `iand`, `ieq.x`, `iand` (deqp_intval_reslib_bvec_fs,
                 * opfs_equal_bvec2; measured on Mesa 26.2.1's per-pass NIR).
                 * The compares the front end made in lane order take that
                 * order here. A compare of floats (`all(lessThanEqual(..))`)
                 * is its own vector operation, in lane order (reslib_float_fs) */
                let equality = matches!(m.def(s[0].val), Some(ir::Op::Alu(AluOp::Ieq | AluOp::Ine, _)));
                if equality && n > 1 && accs[..n - 1].iter().all(|v| *v != Val::Undef) {
                    let lanes: [Val; 4] = [0, 1, 2, 3].map(|i| lane(vals, s[0], i));
                    reduction_order(f, &lanes[..n], &accs[..n - 1])?;
                }
                out[0] = acc;
            }
            /* the integer and boolean operations of a mediump shader, as
             * 32-bit values ([`Fs::int_lane`]): folded where the constants
             * decide them (the index of a uniform array element: `int(t.w
             * * 0.0)` is the constant 0 and `fidx + 0` is `fidx`, case 122's
             * program has neither an F32_TO_S32 nor an IADD, uniidx_i), else
             * the compares and conversions the references draw */
            ir::Op::Alu(a, s) if int_op(a) => {
                let w = width_of(m, ir::ValueId(id.0));
                let k = ir_arity(a);
                for i in 0..w.min(4) {
                    let x = [lane(vals, s[0], i), lane(vals, s[1], i), lane(vals, s[2], i)];
                    if x[..k].contains(&Val::Undef) {
                        if i > 0 {
                            break;
                        }
                        return Err(MaliError::ColourForm);
                    }
                    out[i] = f.int_lane(m, a, &s, x, at)?;
                }
            }
            /* a mediump select of constants on gl_FrontFacing's 32-bit mask
             * (facing.frag): `bcsel(a, b, b)` is `b`, `bcsel(a, 1.0, 0.0)`
             * `b2f(a)` (nir_opt_algebraic.py), the rest a select of the two
             * constants, which the colour's conversion folds into a 16-bit
             * select as a highp shader's ([`mask_colour`]; builtin_49's
             * `MUX.v2i16 r1, 0x3C000000.h11, 0x0, r0.h00`) */
            /* a condition the walk folded picks its side; `bcsel(inot(a),
             * b, c)` is `bcsel(a, c, b)` (nir_opt_algebraic.py; case 112's
             * `!b && .. ? green : red`, two `MUX.v2i16` on the bool
             * uniform's word, intbig_case112.expect.json) */
            ir::Op::Alu(AluOp::Bcsel, s) if f.bool32(lane(vals, s[0], 0)) || f.mask16(lane(vals, s[0], 0)) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (mut c, mut y, mut z) = (lane(vals, s[0], i), lane(vals, s[1], i), lane(vals, s[2], i));
                    if let Some((Alu::Inot, [a, ..])) = f.alu_of(c) {
                        (c, y, z) = (a, z, y);
                    }
                    *o = match (c, y, z) {
                        (Val::Const(k), ..) => {
                            if k != 0 {
                                y
                            } else {
                                z
                            }
                        }
                        _ if y == z => y,
                        (_, Val::Const(F32_ONE), Val::Const(0)) => f.op32(Alu::B2f, [c, Val::Undef, Val::Undef], at)?,
                        (_, Val::Const(_), Val::Const(_)) => f.op32(Alu::Bcsel, [c, y, z], at)?,
                        _ => return Err(MaliError::Arithmetic),
                    };
                }
            }
            /* `sin`, `cos`, `log2` of an fp16 uniform's component: evaluated
             * at fp32 and narrowed (`bi_lower_bit_size`), one 16-bit node a
             * lane, so a reader stays fp16 (case 77's `log2(u_a.z) * 0.25`,
             * `FMA.v2f16` after the narrowing, fsarith_77_transc.expect.json).
             * Another operand (a constant the front end left, gltest's
             * f1_fs `log2(16.0)` in a global's initialiser) has no
             * reference. `exp2` is the next arm */
            ir::Op::Alu(a @ (AluOp::Fsin | AluOp::Fcos | AluOp::Flog2), s) => {
                let op = match a {
                    AluOp::Fsin => Alu::Fsin,
                    AluOp::Fcos => Alu::Fcos,
                    _ => Alu::Flog2,
                };
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = lane(vals, s[0], i);
                    /* `log2` of a computed fp16 lane too (`pow`'s, gm2_fsarith_1:
                     * `F16_TO_F32 r0, r0^.h0`, then `FREXPE.f32.log`) */
                    let computed = op == Alu::Flog2 && f.is_f16(x);
                    if !computed && (f.alu_of(x).is_some() || f.uniform_dword(x).is_none()) {
                        return Err(MaliError::Arithmetic);
                    }
                    *o = f.node(op, [x, Val::Undef, Val::Undef], at)?;
                }
            }
            /* `exp2` of a mediump lane, evaluated at fp32 (`bi_lower_bit_size`,
             * `bi_fexp_32`; [`exp2_rows`]): of a varying's lane, the load's
             * fp32 word itself, its `f2fmp` gone at the link (`f2f32(f2fmp(
             * a))` is `a`), the results narrowed in pairs (gm2_fsmisc_5:
             * `FMA_RSCALE.f32 r3, r3^, 0x3F800000, 0x0.neg, r4` of the
             * load's words, then `V2F32_TO_V2F16 r2, r2^, r3^`,
             * [`vectorise`]); of a uniform's component or a computed fp16
             * lane, widened and narrowed as `sin`'s is (gm2_fsmisc_6's
             * `F16_TO_F32 r0, r0^.h0` .. `FADD.f32 r0.h0, r0^, 0x0.neg`,
             * [`transc_rows`]). Of a constant there is no reference (no
             * fold here) */
            ir::Op::Alu(AluOp::Fexp2, s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    /* `fexp2(fmul(flog2(a), b))` is `fpow(a, b)`
                     * (nir_opt_algebraic.py:1872; the front end's `pow`),
                     * `bi_fpow_32`: `log2(a)`, then `bi_fexp_32` with `b` as
                     * the scaled value and the log as the base (gm2_fsarith_1:
                     * `FMA_RSCALE.f32 r0, u8.w1, r0^, 0x0.neg, r2^` of 100.0).
                     * A constant `b` only, other than the exponents :1882-1886
                     * rewrite (1.0 to 4.0) */
                    if let Some((a, b)) = f.pow_of(x) {
                        if matches!(f32::from_bits(b), 1.0 | 2.0 | 3.0 | 4.0) {
                            return Err(MaliError::AlgebraicForm);
                        }
                        *o = f.node(Alu::Fpow, [a, Val::Const(b), Val::Undef], at)?;
                        continue;
                    }
                    let varying = f.input_lane(x).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS);
                    match x {
                        Val::Undef => return Err(MaliError::ColourForm),
                        Val::Const(_) => return Err(MaliError::Arithmetic),
                        _ if varying || f.uniform_dword(x).is_some() || f.is_f16(x) => {}
                        _ => return Err(MaliError::Arithmetic),
                    }
                    *o = f.node(Alu::Fexp2, [x, Val::Undef, Val::Undef], at)?;
                }
            }
            /* a mediump dot product (`dot`, and `normalize` and `length`'s
             * `Fdot(v, v)`): `nir_lower_alu_width`'s chain from the last
             * lane, its product then an `ffma` a lane, one 16-bit lane each
             * (fsvec_length_mediump: `FMA.v2f16 r1, r1^.h00, r1^.h00,
             * 0x0.neg` of z, then `.. r0.h11, r0.h11, r1^.h00` of y and
             * `.. r0^.h00, r0^.h00, r1^.h00` of x). The scalar is in every
             * lane the IR reads */
            ir::Op::Alu(a @ (AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4), s) => {
                let n = match a {
                    AluOp::Fdot2 => 2,
                    AluOp::Fdot3 => 3,
                    _ => 4,
                };
                let x = |f: &mut Fs<'_>, k: usize, i: usize| f.fp16_operand(lane(vals, s[k], i));
                let (p, q) = (x(f, 0, n - 1), x(f, 1, n - 1));
                let mut acc = f.op(AluOp::Fmul, p, q, at, Rule::None, None)?;
                for i in (0..n - 1).rev() {
                    let (p, q) = (x(f, 0, i), x(f, 1, i));
                    if p == Val::Undef || q == Val::Undef {
                        return Err(MaliError::ColourForm);
                    }
                    acc = f.node(Alu::Ffma, [p, q, acc], at)?;
                }
                out = [acc; 4];
            }
            /* `inversesqrt` of a computed mediump lane, and `sqrt` as
             * `lower_fsqrt`'s `frcp(frsq(a))`: `FRSQ.f16`, then `FRCP.f16`,
             * on the lane's low half (fsvec_normalize_mediump's `FRSQ.f16
             * r2, r2^.h00`, fsvec_length_mediump's `FRSQ.f16 r0, r0^.h00`
             * then `FRCP.f16 r0, r0^.h00`). Of a constant or of a lane the
             * walk did not compute (a varying's, a uniform's) there is no
             * reference; nor of `frcp` alone (`1.0 / x`) */
            ir::Op::Alu(a @ (AluOp::Frsq | AluOp::Fsqrt), s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    if x == Val::Undef {
                        return Err(MaliError::ColourForm);
                    }
                    if f.alu_of(x).is_none() && f.input_lane(x).is_none() {
                        return Err(MaliError::Arithmetic);
                    }
                    *o = f.node(Alu::Frsq, [x, Val::Undef, Val::Undef], at)?;
                }
                /* `lower_fsqrt` runs on the vector before the lanes are
                 * split: every lane's `frsq`, then every lane's `frcp`
                 * (gm2_fsmisc_1's four `FRSQ.f16`, then four `FRCP.f16`) */
                if a == AluOp::Fsqrt {
                    for o in out.iter_mut().take(w) {
                        *o = f.node(Alu::Frcp, [*o, Val::Undef, Val::Undef], at)?;
                    }
                }
            }
            /* `fract` of a mediump lane: `lower_ffract` (nir_opt_algebraic.py:
             * 546), `fadd(a, fneg(ffloor(a)))`, one 16-bit lane each
             * (fsvec_fract_mediump: `FROUND.v2f16.rtn r1, r0.h00`, then
             * `FADD.v2f16 r0, r0^.h00, r1^.neg.h00`). Of a constant there
             * is no reference */
            ir::Op::Alu(AluOp::Ffract, s) => {
                let w = width_of(m, ir::ValueId(id.0));
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    match x {
                        Val::Undef => return Err(MaliError::ColourForm),
                        Val::Const(_) => return Err(MaliError::Arithmetic),
                        _ => {}
                    }
                    let fl = f.node(Alu::Ffloor, [x, Val::Undef, Val::Undef], at)?;
                    let ng = f.neg(fl, at)?;
                    *o = f.node(Alu::Fadd, [x, ng, Val::Undef], at)?;
                }
            }
            ir::Op::Alu(..) => return Err(MaliError::Arithmetic),
            /* fp32: the load Mesa makes, as the vertex stage builds it
             * (nir.rs), CSE'd by what it loads */
            ir::Op::LoadUniform { dword_off, comps } if prec.highp => {
                /* an int or bool uniform's words are raw bits, read as a
                 * float's are (docs/notes/mali-int-uniforms.md) */
                let (base, size) = uniform_load(m, dword_off)?;
                let first = dword_off - base;
                if first + u16::from(comps) > size {
                    return Err(MaliError::Defect(Defect::IrIndex));
                }
                let before = f.nir.n;
                let n = f.nir.load(Kind::Uniform { dword: base, comps: size as u8 })?;
                if f.nir.n > before {
                    f.origin[n as usize] = at as u16;
                }
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, (first as usize + c) as u8);
                }
            }
            ir::Op::LoadUniform { dword_off, comps } => {
                let decl = m.uniforms.iter().find(|u| {
                    u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK
                        && dword_off >= u.dword_off
                        && dword_off < u.dword_off + u.dwords.max(1)
                });
                /* an int or a bool: one 32-bit word (a bool's as GL stores
                 * it, read as it is, intbig_case112.expect.json); an ivec's
                 * lanes too (deqp_intval_reslib_mixed_fs's `ivec3 ref_out0`,
                 * `ICMP_OR.u32.eq.m1 r1, r1^, u2.w1, 0x0`) */
                let int = decl.is_some_and(|u| {
                    matches!(
                        u.gl_type as u32,
                        GL_INT
                            | GL_INT_VEC2
                            | GL_INT_VEC3
                            | GL_INT_VEC4
                            | GL_BOOL
                            | GL_BOOL_VEC2
                            | GL_BOOL_VEC3
                            | GL_BOOL_VEC4
                    ) && u.array_len <= 1
                });
                /* a float uniform Mesa keeps fp32 (`FsPrecision::fp32_uniforms`):
                 * each lane a 32-bit word narrowed by its own `f2fmp`
                 * (fshighp_color: `V2F32_TO_V2F16 r0, u2.w0, u2.w1`) */
                let k = m.uniforms.iter().position(|u| decl.is_some_and(|d| core::ptr::eq(u, d)));
                let wide = k.is_some_and(|k| prec.fp32_uniforms & 1u64.checked_shl(k as u32).unwrap_or(0) != 0);
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    if wide {
                        let l = f.uniform_lane(dword_off + c as u16, 32, at)?;
                        *o = f.node16(Alu::F2f16, [l, Val::Undef, Val::Undef], 1, at)?;
                        continue;
                    }
                    *o = f.uniform_lane(dword_off + c as u16, if int { 32 } else { 16 }, at)?;
                    if let (Val::Lane(n, _), Some(u), false) = (*o, decl, int) {
                        /* a matrix's load is its column's */
                        /* an array's load is its element's */
                        f.nir.uniform_load[n as usize] =
                            match (nir::matrix_cols(u.gl_type), nir::element_load(u, dword_off)) {
                                (Some((_, _, stride)), _) => u.dword_off + (dword_off - u.dword_off) / stride * stride,
                                (None, Some((base, _))) => base,
                                (None, None) => u.dword_off,
                            };
                    }
                }
            }
            /* a mediump shader's read of a vec4 array element at a run-time
             * index, Mesa's `load_ubo(0, index * 8)` then `LD_PKA.i64`
             * (docs/notes/mali-uniform-index.md; uniidx_i, uniidx_d): the
             * index an int uniform's word (the only index form a reference
             * draws: `fidx + int(t.w * 0.0)` folds to `fidx`), one such
             * read a shader; or a mediump float array's element, Mesa's
             * 16-bit `load_ubo(0, base + (index << 2))` then `LD_PKA.i16`
             * (docs/notes/mali-uniform-arrays.md; case108_uniidx: four
             * reads, each at its own int uniform). A highp shader's fp32
             * float array: the vertex stage's 32-bit `load_ubo` and
             * `LD_PKA.i32` ([`nir::uniform_idx`], uniarray_highp) */
            ir::Op::LoadUniformIdx { base_dw, index, comps, .. } if prec.highp => {
                if f.nubo >= MAX_UBO_READS {
                    return Err(MaliError::UniformIndexed);
                }
                let n = nir::uniform_idx(&mut f.nir, m, base_dw, comps, lane(vals, index, 0))?;
                f.origin[n as usize] = at as u16;
                f.ubo_idx[f.nubo] = n;
                f.nubo += 1;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, c as u8);
                }
            }
            ir::Op::LoadUniformIdx { base_dw, index, comps, .. } => {
                let (u, _) = uniform_extent(m, base_dw)?;
                let idx = lane(vals, index, 0);
                let int_word = matches!(idx, Val::Lane(n, 0)
                    if matches!(f.nir.nodes[n as usize], NirNode { kind: Kind::Uniform { .. }, bits: 32, .. }));
                let vec4 = f.ubo_idx[..f.nubo].iter().any(|&n| f.ubo_comps(n) == 4);
                /* a vec2, vec3 or vec4 array's element read more than once,
                 * every read of one array type (deqp_uniarray_dyn_vec*_fs:
                 * each read's offset and `LD_PKA.i32` / `.i48` / `.i64`, as
                 * case108_uniidx's float reads, [`ubo_fetch`]) */
                let same = f.ubo_idx[..f.nubo].iter().all(|&n| f.ubo_comps(n) == comps);
                let shape = match u.gl_type as u32 {
                    GL_FLOAT => comps == 1 && !vec4,
                    crate::gl::consts::GL_FLOAT_VEC2 => comps == 2 && same,
                    crate::gl::consts::GL_FLOAT_VEC3 => comps == 3 && same,
                    _ => comps == 4 && same,
                };
                if f.nubo >= MAX_UBO_READS || u.dword_off != base_dw || !shape || !int_word {
                    return Err(MaliError::UniformIndexed);
                }
                let nd =
                    NirNode { kind: Kind::UniformIdx { dword: base_dw, comps }, src: [idx; 3], bits: 16, width: 1 };
                let n = f.nir.push_node(nd)?;
                f.origin[n as usize] = at as u16;
                f.ubo_idx[f.nubo] = n;
                f.nubo += 1;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `textureLod` on a cube (cases 41 and 114): `va_lower_tex`
             * replaces the direction by `build_cube_desc`'s two words in
             * either stage (pan_nir_lower_tex.c:825-829), its lanes any
             * value, a constant one an operand (case114_f.expect.json's
             * `CUBEFACE1.discard r2, 0x3F800000, r0, r1`). An implicit
             * level is the arm after the bias's */
            ir::Op::Tex {
                dim: ir::TexDim::Cube,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Lod(level),
                offset: None,
            } => {
                let dir = [lane(vals, coord, 0), lane(vals, coord, 1), lane(vals, coord, 2)];
                let level = lane(vals, level, 0);
                if dir.contains(&Val::Undef) || level == Val::Undef || dir.iter().all(|v| matches!(v, Val::Const(_))) {
                    return Err(MaliError::TexCoord);
                }
                /* `nir_scalar_is_zero`: the bits 0 */
                let explicit = level != Val::Const(0);
                let src = if prec.highp {
                    for v in dir.into_iter().chain([level]) {
                        f.mark_mixed(v);
                    }
                    let first = f.nir.n;
                    let (s, t) = f.nir.cube_desc(dir[0], dir[1], dir[2])?;
                    let l = if explicit { f.nir.sfixed_8_8(level)? } else { Val::Undef };
                    for id in first..f.nir.n {
                        f.origin[id] = at as u16;
                    }
                    [s, t, l]
                } else if explicit {
                    /* a mediump level (its 8.8 word from an fp16 value):
                     * no reference */
                    return Err(MaliError::TexKind);
                } else {
                    dir
                };
                /* a highp sampler's look-up stays 32-bit in a highp
                 * shader (case114_f.expect.json's `TEX_SINGLE ... .32`) */
                let bits = if prec.highp && prec.highp_samplers >> (sampler & 31) & 1 != 0 { 32 } else { 16 };
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src, bits, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let cube = Some(Cube { lod: if explicit { CubeLod::Explicit } else { CubeLod::Zero } });
                f.push_look_up(TexLookup::new(sampler, TexCoord::Computed, cube, false, at, n))?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `textureCube(s, dir, bias)` in a mediump shader (case 101
             * cubebias, varyconst_101_cubebias.expect.json): the look-up at
             * `COMPUTED_BIAS`, its third staging word `build_lod_bias_clamp`'s
             * `pack_32_2x16(sfixed_8_8(bias), 0)` of the mediump uniform
             * widened (pan_nir_lower_tex.c:144-149, :938-940: `F16_TO_F32
             * r2, u2.w0.h0` then the 8.8 chain, [`select`]); the direction's
             * lanes as a cube's at level 0, raw varying lanes and the
             * link's constant among them, of the shapes the implicit arm
             * takes ([`cube_dir_measured`]; fstex_cubebias_uni). A bias
             * that is no uniform lane, or a highp shader's: no reference */
            ir::Op::Tex {
                dim: ir::TexDim::Cube,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Bias(bias),
                offset: None,
            } if !prec.highp => {
                let dir = [lane(vals, coord, 0), lane(vals, coord, 1), lane(vals, coord, 2)];
                let bias = lane(vals, bias, 0);
                if !cube_dir_measured(f, m, coord, dir) {
                    return Err(MaliError::TexCoord);
                }
                if sampler_array_element(m, sampler) {
                    return Err(MaliError::TexArrayElement);
                }
                if f.uniform_dword(bias).is_none() {
                    return Err(MaliError::TexKind);
                }
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src: dir, bits: 16, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let mut t =
                    TexLookup::new(sampler, TexCoord::Computed, Some(Cube { lod: CubeLod::Computed }), false, at, n);
                t.bias = bias;
                f.push_look_up(t)?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `textureCube(s, dir)` in a mediump shader (dEQP-GLES2's
             * texture cube cases, fstex_cube.expect.json): the look-up at
             * `COMPUTED_LOD` (pan_nir_lower_tex.c:944-945), two staging
             * words, `build_cube_desc`'s of the direction's lanes as case
             * 41's arm takes them ([`select`]), for the direction shapes
             * measured against Mesa ([`cube_dir_measured`]). Any other
             * direction, or a highp shader's look-up: no reference */
            ir::Op::Tex {
                dim: ir::TexDim::Cube,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Implicit,
                offset: None,
            } if !prec.highp => {
                let dir = [lane(vals, coord, 0), lane(vals, coord, 1), lane(vals, coord, 2)];
                if !cube_dir_measured(f, m, coord, dir) {
                    return Err(MaliError::TexCoord);
                }
                if sampler_array_element(m, sampler) {
                    return Err(MaliError::TexArrayElement);
                }
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src: dir, bits: 16, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let cube = Some(Cube { lod: CubeLod::Computed });
                f.push_look_up(TexLookup::new(sampler, TexCoord::Computed, cube, false, at, n))?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `texelFetch(s, ivec2, 0)` of a highp shader: Mesa's `txf`,
             * `TEX_FETCH` on the coordinate's two integer words with no
             * level word (fbload_fbload.expect.json, of a `sampler2D`;
             * fbload_fbload_int.expect.json, of a `usampler2D`;
             * fbload_isamp.expect.json, of an `isampler2D`: the same
             * `.32.2d` word). A level other than the constant 0 has no
             * reference */
            ir::Op::Tex {
                dim: ir::TexDim::D2,
                shadow: false,
                proj: false,
                ret: ret @ (ir::ScalarType::F32 | ir::ScalarType::U32 | ir::ScalarType::I32),
                sampler,
                coord,
                lod: ir::TexLod::Fetch(level),
                offset: None,
            } if prec.highp => {
                if lane(vals, level, 0) != Val::Const(0) {
                    return Err(MaliError::TexKind);
                }
                let (s, t) = (lane(vals, coord, 0), lane(vals, coord, 1));
                if s == Val::Undef || t == Val::Undef {
                    return Err(MaliError::TexCoord);
                }
                let highp = prec.highp_samplers >> (sampler & 31) & 1 != 0;
                let bits = if highp || ret != ir::ScalarType::F32 { 32 } else { 16 };
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src: [s, t, Val::Undef], bits, width: 1 })?;
                f.origin[n as usize] = at as u16;
                f.push_look_up(TexLookup::new(sampler, TexCoord::Computed, None, true, at, n))?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* a 2D array's look-up of a mediump shader (case 42 es3array,
             * texmore_206_array; docs/notes/mali-texture-lookups.md 3):
             * `texture(s, vec3(uv, layer))` is `TEX_SINGLE .2d.array_enable
             * .computed` of the three lanes at fp32 (`F16_TO_F32` each, the
             * layer not rounded: the sampler clamps it), `texelFetch(s,
             * ivec3, 0)` `TEX_FETCH .2d.array_enable` of the three 32-bit
             * integers (pan_nir_lower_tex.c:835-849). A level other than the
             * constant 0, a constant lane: no reference */
            ir::Op::Tex {
                dim: ir::TexDim::D2Array,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod,
                offset: None,
            } if !prec.highp => {
                let fetch = match lod {
                    ir::TexLod::Implicit => false,
                    ir::TexLod::Fetch(level) if lane(vals, level, 0) == Val::Const(0) => true,
                    _ => return Err(MaliError::TexKind),
                };
                let mut src = [Val::Undef; 3];
                for (c, s) in src.iter_mut().enumerate() {
                    let v = lane(vals, coord, c);
                    let Val::Lane(n, _) = v else { return Err(MaliError::TexCoord) };
                    let bits = f.nir.nodes[n as usize].bits;
                    *s = match (fetch, bits) {
                        (true, 32) if !f.is_f16(v) => v,
                        (false, 16) => f.widen_any(v, at)?,
                        _ => return Err(MaliError::TexCoord),
                    };
                }
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src, bits: 16, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let mut t = TexLookup::new(sampler, TexCoord::Computed, None, fetch, at, n);
                t.array = true;
                f.push_look_up(t)?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `texture`, `textureLod(.., 0.0)` and `texelFetch(.., 0)` of
             * a `sampler3D` in a highp shader (case 48 es3vol, case 206;
             * texmore_48, docs/notes/mali-texture-lookups.md 3): `TEX_SINGLE
             * .3d` of the three fp32 lanes at the computed level or at
             * `zero`, `TEX_FETCH .3d` of the three integers; the texels
             * 32-bit for a highp sampler. Another level, a bias, an
             * integer sampler: no reference */
            ir::Op::Tex {
                dim: ir::TexDim::D3,
                shadow: false,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod,
                offset: None,
            } if prec.highp => {
                let (fetch, zero) = match lod {
                    ir::TexLod::Implicit => (false, false),
                    ir::TexLod::Lod(level) if lane(vals, level, 0) == Val::Const(0) => (false, true),
                    ir::TexLod::Fetch(level) if lane(vals, level, 0) == Val::Const(0) => (true, false),
                    _ => return Err(MaliError::TexKind),
                };
                let mut src = [Val::Undef; 3];
                for (c, s) in src.iter_mut().enumerate() {
                    let v = lane(vals, coord, c);
                    if v == Val::Undef {
                        return Err(MaliError::TexCoord);
                    }
                    f.mark_mixed(v);
                    *s = v;
                }
                let bits = if prec.highp_samplers >> (sampler & 31) & 1 != 0 { 32 } else { 16 };
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src, bits, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let mut t = TexLookup::new(sampler, TexCoord::Computed, None, fetch, at, n);
                t.d3 = true;
                t.zero = zero;
                f.push_look_up(t)?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* `textureSize` at level 0 of a 2D texture, a 2D array or a 3D
             * texture ([`Fs::tex_size`]; case 42, texmore_46, texmore_48).
             * `lower_txs_lod` makes another level's size a shift of this
             * one (pan_compiler.c:120), which no reference draws */
            ir::Op::TexSize { sampler, dim, lod } => {
                if lane(vals, lod, 0) != Val::Const(0) {
                    return Err(MaliError::TexKind);
                }
                f.tex_size(sampler, dim, at, &mut out)?;
            }
            /* `texture(sampler2DShadow, vec3(uv, ref))` of a mediump shader
             * (case 218 shadowcmp, fsarith_218_shadowcmp.expect.json;
             * docs/notes/mali-fragcoord-fp16-ops.md): `TEX_SINGLE ... .16.r
             * .2d.shadow.computed` of three staging words, the fp16
             * coordinate's two lanes widened (`F16_TO_F32` each, as a
             * computed coordinate's, [`tex_highp`]) and the reference, an
             * fp32 constant pushed (`MOV.i32 r6, u5.w1`); the one texel
             * fp16 (a mediump sampler's). A computed reference, a highp
             * shader's look-up, a cube or an array shadow sampler: no
             * reference */
            ir::Op::Tex {
                dim: ir::TexDim::D2,
                shadow: true,
                proj: false,
                ret: ir::ScalarType::F32,
                sampler,
                coord,
                lod: ir::TexLod::Implicit,
                offset: None,
            } if !prec.highp => {
                let (s, t, r) = (lane(vals, coord, 0), lane(vals, coord, 1), lane(vals, coord, 2));
                if !f.is_f16(s) || !f.is_f16(t) {
                    return Err(MaliError::TexCoord);
                }
                if !matches!(r, Val::Const(_)) {
                    return Err(MaliError::TexKind);
                }
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src: [s, t, r], bits: 16, width: 1 })?;
                f.origin[n as usize] = at as u16;
                let mut l = TexLookup::new(sampler, TexCoord::Computed, None, false, at, n);
                l.shadow = true;
                f.push_look_up(l)?;
                out[0] = Val::Lane(n, 0);
            }
            ir::Op::Tex { sampler, coord, .. } => {
                if !matches!(op.es100_tex(), Some((Es100Tex::Tex2D, None))) {
                    return Err(MaliError::TexKind);
                }
                let (s, t) = (lane(vals, coord, 0), lane(vals, coord, 1));
                let coord = match (f.input_lane(s), f.input_lane(t), s, t) {
                    (Some((v, 0)), Some((w, 1)), ..) if v == w && v != POINT_COORD => {
                        /* a vec2 varying read whole (the references' coordinate),
                         * or the first two lanes of a wider one in a highp
                         * shader (gm2_texcoord_4's `v.xy` of a vec4, the
                         * load `.v4` when `.zw` is read too, `.v2` when not:
                         * docs/notes/mali-tex-coords.md). A mediump shader's
                         * has no reference here */
                        let comps = m.inputs.iter().find(|d| d.slot() == IoSlot::Varying(v)).map(|d| d.components);
                        if comps < Some(2) || (comps != Some(2) && !prec.highp) {
                            return Err(MaliError::TexCoord);
                        }
                        TexCoord::Varying(v)
                    }
                    (None, None, Val::Const(x), Val::Const(y)) => TexCoord::Const(x, y),
                    /* a computed coordinate: fp32 values of a highp shader,
                     * fp16 ones of a mediump shader, which the look-up reads
                     * converted (`F16_TO_F32`, [`select`]) */
                    _ if s != Val::Undef && t != Val::Undef => TexCoord::Computed,
                    _ => return Err(MaliError::TexCoord),
                };
                let src = match coord {
                    TexCoord::Computed => [s, t, Val::Undef],
                    _ => [Val::Undef; 3],
                };
                /* a highp sampler's look-up is 32-bit in a highp shader, as
                 * a cube's (fbload_mipgen.expect.json's `TEX_SINGLE ... .32
                 * .2d.computed` of `highp sampler2D u_src`) */
                let bits = if prec.highp && prec.highp_samplers >> (sampler & 31) & 1 != 0 { 32 } else { 16 };
                let n = f.nir.push_node(NirNode { kind: Kind::Tex, src, bits, width: 1 })?;
                f.origin[n as usize] = at as u16;
                f.push_look_up(TexLookup::new(sampler, coord, None, false, at, n))?;
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* a `discard` outside `if (c) discard;` (unconditional, or in a
             * block with other statements): `demote`'s `DISCARD.f32 0, 0
             * .eq`, which no reference draws */
            ir::Op::Discard => return Err(MaliError::DiscardForm),
        }
        vals[id.0 as usize] = out;
    }
    Ok(Flow::Next)
}

/// The direction of a mediump `textureCube` at the implicit level or with
/// a bias is one of the shapes measured byte-equal to Mesa 26.2.1's
/// drm-shim (fstex_cube*):
///
/// - raw: each lane a varying's lane or a constant fp16 holds exactly,
///   one varying lane at least, each varying read whole unless it is the
///   coordinate itself (fstex_cube, fstex_cubedir_twovary, _constneg,
///   _rawhalf, _v4part). The cube ops read the load's fp32 words
///   (`src_f32`) and the constant as an operand: +-1.0 the table's
///   `0x3F800000`, 0.5 an FAU word (their sources have no swizzle,
///   valhall.c CUBEFACE1).
/// - pair: x and y fp16 trees of `+`, `*` and negation over varying
///   lanes, constants and uniforms, vectorised as one pair (the colour's
///   forms, [`ADMITTED`], checked in [`select`]); z a constant or a
///   uniform (fstex_cubeface4, fstex_cubedir_face4uni, _uniscale, _fmak,
///   _pairhalf). Each lane is `F16_TO_F32`'d at the look-up, a constant z
///   rounded to fp16 first ([`cube_lane`]).
///
/// Measured otherwise, and so refused:
/// - a raw constant fp16 does not hold (`vec3(a2, 0.1)`,
///   fstex_cubeno_texcoord_rawtenth): Mesa rounds it to fp16 and reads the
///   widened value from an FAU word; the raw lanes here keep the fp32
///   constant;
/// - raw lanes beside a uniform (`vec3(a2, u)`): Mesa keeps the varying
///   fp32, the pair path narrows it;
/// - gl_FragCoord or gl_PointCoord: Mesa keeps them fp32;
/// - a product with 2.0 left alone (`a2 * 2.0`): Mesa's late rule makes it
///   `FADD.v2f16 r0, r0^, r0^` (bifrost_nir_algebraic.py:31), which the
///   pair here does not. Under an add it is the fused `FMA` (dEQP's
///   `v.x*2.0 - 1.0`), measured equal.
fn cube_dir_measured(f: &Fs<'_>, m: &Module<'_>, coord: ir::Src, dir: [Val; 3]) -> bool {
    /* a varying's lane; gl_PointCoord's ([`POINT_COORD`]) is none */
    let varying = |v: Val| f.input_lane(v).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS);
    /* ours loads the lanes read. Mesa's load of a direction built of
     * parts is the varying's whole width in the forms measured so
     * (fstex_cubeno_texcoord_partialx: `a2.x` loads `.v2`), and the lanes
     * read in others (a vec4's `.x, .y, .z` loads `.v3`); no rule found
     * gives the width for every form (docs/notes/mali-cube-textures.md
     * (h), "The raw load's width"). So the raw lanes must read every
     * component of each float vector varying, where both loads are its
     * width. A varying that is the coordinate itself (`a4.xyz`) is a
     * texture argument, which lower_precision leaves as it is
     * (`visit_enter(ir_texture *)`, lower_precision.cpp:785-790): the
     * lanes read (fstex_cubedir_v4part) */
    let direct = matches!(m.def(coord.val), Some(ir::Op::LoadInput { .. }));
    let whole = |loc: u8| {
        let lanes = dir.iter().filter_map(|&v| f.input_lane(v)).filter(|&(l, _)| l == loc);
        let read = lanes.fold(0u8, |r, (_, c)| r | 1 << c);
        m.inputs.iter().any(|d| {
            d.kind == IoKind::Varying
                && d.location == loc
                && is_float_varying(d.gl_type)
                && !is_matrix(d.gl_type)
                && u32::from(read) + 1 == 1 << d.components
        })
    };
    let uniform = |v: Val| f.alu_of(v).is_none() && f.uniform_dword(v).is_some();
    /* an fp16 tree; `fused`: an fadd takes it into its FMA (the fadd's
     * one product; with two, which one fuses is not measured) */
    fn tree(f: &Fs<'_>, v: Val, fused: bool, leaf: &dyn Fn(Val) -> bool) -> bool {
        let Some((op, s)) = f.alu_of(v) else { return leaf(v) };
        let fmul = |v: Val| matches!(f.alu_of(v), Some((Alu::Fmul, _)));
        match op {
            Alu::Fneg => tree(f, s[0], false, leaf),
            Alu::Fadd => {
                let one = fmul(s[0]) != fmul(s[1]);
                tree(f, s[0], one, leaf) && tree(f, s[1], one, leaf)
            }
            Alu::Fmul if !fused && (s[0] == Val::Const(F32_TWO) || s[1] == Val::Const(F32_TWO)) => false,
            Alu::Fmul => tree(f, s[0], false, leaf) && tree(f, s[1], false, leaf),
            _ => false,
        }
    }
    let exact = |v: Val| matches!(v, Val::Const(c) if f16_exact(c).is_some());
    let raw = dir.iter().all(|&v| varying(v) || exact(v))
        && dir.iter().any(|&v| varying(v))
        && (direct || dir.iter().filter_map(|&v| f.input_lane(v)).all(|(loc, _)| whole(loc)));
    let leaf = |v: Val| varying(v) || uniform(v) || matches!(v, Val::Const(_));
    let pair = dir[..2].iter().all(|&v| f.alu_of(v).is_some() && tree(f, v, false, &leaf))
        && (matches!(dir[2], Val::Const(_)) || uniform(dir[2]));
    raw || pair
}

/// Sampler binding `b` is an element of a sampler array other than its
/// first. Mesa numbers an array's elements together, so `sa[1]` read alone
/// is descriptor index 1 (fstex_cubeno_texarrayelement_implicit: the
/// handle `0x20011801`); [`layout`] counts the samplers read and gives it
/// 0, `sa[0]`'s texture. Refused until the index counts the array.
fn sampler_array_element(m: &Module<'_>, b: u8) -> bool {
    m.uniforms
        .iter()
        .any(|u| u.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK && b > u.binding && b - u.binding < u.array_len)
}

/// The sources `t` and `x` of atan's `fabs(t) * fsign(x)`, `t` the
/// `ffma(fabs(u), res, bcsel(..))` of builtin_call.rs `atan`.
fn atan_sign(m: &Module<'_>, a: AluOp, srcs: &[ir::Src; 3]) -> Option<(ir::Src, ir::Src)> {
    let alu = |s: ir::Src| match m.def(s.val) {
        Some(ir::Op::Alu(op, ss)) => Some((*op, *ss)),
        _ => None,
    };
    if a != AluOp::Fmul {
        return None;
    }
    let (AluOp::Fabs, [t, ..]) = alu(srcs[0])? else { return None };
    let (AluOp::Fsign, [x, ..]) = alu(srcs[1])? else { return None };
    let (AluOp::Ffma, [u, _, bias]) = alu(t)? else { return None };
    let shape = matches!(alu(u), Some((AluOp::Fabs, _))) && matches!(alu(bias), Some((AluOp::Bcsel, _)));
    shape.then_some((t, x))
}

/// The `fneg(arc)` (its value, and `arc`) of atan2's select whose test is
/// `flt(fmin, 0.0)` of the `fmin` instruction `id` (builtin_call.rs
/// `atan2`: the `fmin`, the `flt`, the `fneg`, the `bcsel`, in that order).
fn atan2_fneg(m: &Module<'_>, a: AluOp, id: usize) -> Option<(ir::ValueId, ir::Src)> {
    if a != AluOp::Fmin {
        return None;
    }
    let flt = id + 1;
    let (neg, sel) = (id + 2, id + 3);
    match (m.instrs.get(flt)?, m.instrs.get(neg)?, m.instrs.get(sel)?) {
        (ir::Op::Alu(AluOp::Flt, f), ir::Op::Alu(AluOp::Fneg, [arc, ..]), ir::Op::Alu(AluOp::Bcsel, [c, t, e]))
            if f[0].val.0 as usize == id
                && c.val.0 as usize == flt
                && t.val.0 as usize == neg
                && e.val == arc.val
                && e.swizzle == arc.swizzle =>
        {
            Some((ir::ValueId(neg as u32), *arc))
        }
        _ => None,
    }
}

/// The reads of IR value `v` (an `If`'s condition is a bool, never a sum's).
pub(super) fn ir_uses(m: &Module<'_>, v: ir::ValueId) -> usize {
    let mut n = 0;
    for op in m.instrs.iter() {
        op.for_each_src(|s| n += usize::from(s.val == v));
    }
    n
}

/// `nir::is_mat_term` for a highp fragment shader: the column a uniform
/// load of a mat4's column or of a vec4 array's element (`mat3(frag[0].xyz,
/// frag[1].xyz, frag[2].xyz)`, whose columns the front end reads as the
/// elements' loads), the scale a splat lane, the accumulator a product.
fn is_mat_term_fs(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let col = match m.def(srcs[0].val) {
        Some(ir::Op::LoadUniform { dword_off, comps: 4 }) => *dword_off,
        _ => return false,
    };
    let is_col = uniform_extent(m, col)
        .is_ok_and(|(u, _)| u.gl_type as u32 == crate::gl::consts::GL_FLOAT_MAT4 || u.array_len > 1);
    let splat = srcs[1].swizzle.iter().all(|&s| s == srcs[1].swizzle[0]);
    let acc = matches!(m.def(srcs[2].val), Some(ir::Op::Alu(AluOp::Fmul | AluOp::Ffma, _)));
    is_col && splat && acc
}

/// Where an IR value stands in the front end's chain of `m * v` by a
/// constant `v` ([`const_mat_chain`]).
enum ChainAt {
    None,
    /// A term the chain's last `ffma` reads (made there, not here).
    Inner,
    /// The chain's last `ffma`: its terms in the columns' order, each the
    /// column and its constant scale, and their number.
    Top([(ir::Src, u32); 4], usize),
}

/// The front end's `m * v` (lower.rs `mat_times_column`, Mesa's GLSL
/// order: `fmul(m1, v.y)`, then `ffma(m0, v.x, ..)`, `ffma(m2, v.z, ..)`,
/// `ffma(m3, v.w, ..)`, every scale a splat lane of one value) whose scales
/// are all constants other than 0.0 and -1.0, each term but the last read
/// once: GLSL's `lower_mat_op_to_vec` makes it `((m0 * v.x + m1 * v.y) +
/// m2 * v.z) + m3 * v.w` (lower_mat_op_to_vec.cpp:157-183), a term by 1.0
/// the column itself (opt_algebraic.cpp's `is_vec_one`), and NIR's rules
/// meet the scalar sums before the late fusion ([`MatSum`]).
fn const_mat_chain(m: &Module<'_>, vals: &[[Val; 4]], id: usize) -> ChainAt {
    let scale_of = |v: usize| match m.def(ir::ValueId(v as u32)) {
        Some(ir::Op::Alu(AluOp::Fmul | AluOp::Ffma, s)) => Some(s[1]),
        _ => None,
    };
    /* the next term's `ffma`, when `v` is its accumulator */
    let link_user = |v: usize| -> Option<usize> {
        let scale = scale_of(v)?;
        if ir_uses(m, ir::ValueId(v as u32)) != 1 {
            return None;
        }
        m.instrs.iter().position(|op| {
            matches!(op, ir::Op::Alu(AluOp::Ffma, s) if s[2].val.0 as usize == v
                && s[0].val.0 as usize != v && s[1].val == scale.val)
        })
    };
    let mut top = id;
    while let Some(u) = link_user(top) {
        top = u;
    }
    let mut terms = [(ir::Src::id(ir::ValueId(0)), 0u32); 4];
    let mut seen = [false; 4];
    let mut comps = [0u8; 4];
    let mut n = 0;
    let mut cur = top;
    let mut has_id = false;
    let vec = match scale_of(top) {
        Some(s) => s.val,
        None => return ChainAt::None,
    };
    loop {
        has_id |= cur == id;
        let (op, s) = match m.def(ir::ValueId(cur as u32)) {
            Some(ir::Op::Alu(op @ (AluOp::Fmul | AluOp::Ffma), s)) => (*op, *s),
            _ => return ChainAt::None,
        };
        let c = s[1].swizzle[0];
        if n == 4 || s[1].val != vec || s[1].swizzle.iter().any(|&x| x != c) || c > 3 || seen[c as usize] {
            return ChainAt::None;
        }
        let k = match vals.get(vec.0 as usize).map(|v| v[c as usize]) {
            Some(Val::Const(k)) if nir::plain(k) && k & !F32_NEG_ZERO != 0 && k != F32_MINUS_ONE => k,
            _ => return ChainAt::None,
        };
        seen[c as usize] = true;
        terms[c as usize] = (s[0], k);
        comps[n] = c;
        n += 1;
        if op == AluOp::Fmul {
            break;
        }
        cur = s[2].val.0 as usize;
    }
    /* innermost first: v.y, v.x, then v.z and v.w */
    const ORDER: [u8; 4] = [1, 0, 2, 3];
    if n < 2 || !(0..n).all(|k| comps[n - 1 - k] == ORDER[k]) {
        return ChainAt::None;
    }
    match (has_id, top == id) {
        (_, true) => ChainAt::Top(terms, n),
        (true, false) => ChainAt::Inner,
        (false, false) => ChainAt::None,
    }
}

/// One scalar instruction of [`MatSum`] (indices into its table).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Term {
    Leaf(Val),
    Neg(u8),
    Add(u8, u8),
    Mul(u8, u8),
}

const MAT_TERMS: usize = 64;

/// One lane of a matrix product by a constant vector as NIR holds it after
/// `nir_lower_alu_to_scalar` (gl_nir_linker.c:71-75: Bifrost's
/// `lower_to_scalar`), simplified as the GL front end's optimisation loop
/// does: `nir_opt_algebraic` (nir_search.c:869-905: the last instruction
/// first, a rewritten instruction's users queued again, the new
/// instructions before the old one's place, nir_search.c:700-750), each
/// pass after `nir_opt_dce` and `nir_opt_cse`, a constant operation folded
/// as it is made (`nir_opt_constant_folding`). The rules are the ones the
/// sums of scaled terms reach, in their list order: `fadd(fmul(a, b),
/// fmul(a, c))` (nir_opt_algebraic.py:246), `fadd(a, a)` (:274),
/// `fadd(a, fadd(a, b))` (:275), `fadd(fadd(fmul, fmul), c)` (:2419);
/// `fmul(fneg(a), b)` (:2387), `fmul(fmul(a, b), #c)` (:2394) and
/// `fmul(#a, fmul(b, #c))` (:2406); each commutative operand tried in
/// nir_search's order (a combination's bit k swaps the k-th commutative
/// expression of the pattern, the outer one first). A sum with a constant
/// left after the folding meets the constant reassociations
/// (:2398-2412), which are not transcribed ([`MaliError::ArithConstant`]).
/// Measured: algfold_189_half and algfold_189_mulone.expect.json.
struct MatSum {
    t: [Term; MAT_TERMS],
    /// A leaf that is a product (`is_fmul`, nir_search_helpers.h:403-416).
    fmul: [bool; MAT_TERMS],
    /// Replaced or dropped by the dead code.
    gone: [bool; MAT_TERMS],
    n: usize,
    /// The instructions in program order.
    order: [u8; MAT_TERMS],
    len: usize,
    /// Where a rule's new instructions go.
    cursor: Option<usize>,
}

impl MatSum {
    fn new() -> MatSum {
        MatSum {
            t: [Term::Leaf(Val::Undef); MAT_TERMS],
            fmul: [false; MAT_TERMS],
            gone: [false; MAT_TERMS],
            n: 0,
            order: [0; MAT_TERMS],
            len: 0,
            cursor: None,
        }
    }

    fn konst(&self, i: u8) -> Option<u32> {
        match self.t[i as usize] {
            Term::Leaf(Val::Const(k)) => Some(k),
            _ => None,
        }
    }

    fn is_fmul(&self, i: u8) -> bool {
        match self.t[i as usize] {
            Term::Mul(..) => true,
            Term::Neg(x) => self.is_fmul(x),
            Term::Leaf(_) => self.fmul[i as usize],
            Term::Add(..) => false,
        }
    }

    fn leaf(&mut self, v: Val, fmul: bool) -> Result<u8, MaliError> {
        let i = self.mk(Term::Leaf(v))?;
        self.fmul[i as usize] |= fmul;
        Ok(i)
    }

    fn fold(x: u32, y: u32, mul: bool) -> Result<u32, MaliError> {
        let (a, b) = (f32::from_bits(x), f32::from_bits(y));
        let r = if mul { a * b } else { a + b }.to_bits();
        if !nir::plain(x) || !nir::plain(y) || !nir::plain(r) {
            return Err(MaliError::ConstantFold);
        }
        Ok(r)
    }

    /// Instruction `t`, folded and CSE'd.
    fn mk(&mut self, t: Term) -> Result<u8, MaliError> {
        let zero = |k: Option<u32>| k.is_some_and(|k| k & !F32_NEG_ZERO == 0);
        let t = match t {
            Term::Neg(x) => match (self.konst(x), self.t[x as usize]) {
                (Some(k), _) => Term::Leaf(Val::Const(k ^ F32_NEG_ZERO)),
                (_, Term::Neg(y)) => return Ok(y),
                _ => t,
            },
            Term::Add(x, y) => match (self.konst(x), self.konst(y)) {
                (Some(a), Some(b)) => Term::Leaf(Val::Const(Self::fold(a, b, false)?)),
                (a, _) if zero(a) => return Ok(y),
                (_, b) if zero(b) => return Ok(x),
                (Some(_), _) | (_, Some(_)) => return Err(MaliError::ArithConstant),
                _ => t,
            },
            Term::Mul(x, y) => match (self.konst(x), self.konst(y)) {
                (Some(a), Some(b)) => Term::Leaf(Val::Const(Self::fold(a, b, true)?)),
                (a, b) if zero(a) || zero(b) => Term::Leaf(Val::Const(0)),
                (Some(F32_ONE), _) => return Ok(y),
                (_, Some(F32_ONE)) => return Ok(x),
                (Some(F32_MINUS_ONE), _) | (_, Some(F32_MINUS_ONE)) => return Err(MaliError::ArithConstant),
                _ => t,
            },
            Term::Leaf(_) => t,
        };
        let same = |u: Term| match (u, t) {
            (Term::Add(a, b), Term::Add(c, d)) | (Term::Mul(a, b), Term::Mul(c, d)) => u == t || (a == d && b == c),
            _ => u == t,
        };
        if let Some(i) = (0..self.n).find(|&i| !self.gone[i] && same(self.t[i])) {
            return Ok(i as u8);
        }
        if self.n == MAT_TERMS {
            return Err(MaliError::TooManyValues);
        }
        let i = self.n;
        self.t[i] = t;
        self.n += 1;
        if !matches!(t, Term::Leaf(_)) {
            let at = self.cursor.unwrap_or(self.len);
            self.order.copy_within(at..self.len, at + 1);
            self.order[at] = i as u8;
            self.len += 1;
            if let Some(c) = self.cursor.as_mut() {
                *c += 1;
            }
        }
        Ok(i as u8)
    }

    fn srcs(&self, i: u8) -> [Option<u8>; 2] {
        match self.t[i as usize] {
            Term::Add(a, b) | Term::Mul(a, b) => [Some(a), Some(b)],
            Term::Neg(a) => [Some(a), None],
            Term::Leaf(_) => [None, None],
        }
    }

    /// The instructions `root` reads, transitively.
    fn live(&self, root: u8) -> [bool; MAT_TERMS] {
        let mut live = [false; MAT_TERMS];
        let mut stack = [0u8; MAT_TERMS];
        let mut sp = 1;
        stack[0] = root;
        while sp > 0 {
            sp -= 1;
            let i = stack[sp];
            if live[i as usize] {
                continue;
            }
            live[i as usize] = true;
            for s in self.srcs(i).into_iter().flatten() {
                if !live[s as usize] && sp < MAT_TERMS {
                    stack[sp] = s;
                    sp += 1;
                }
            }
        }
        live
    }

    /// The reads of `i` by live instructions, the store's of the root.
    fn uses(&self, live: &[bool; MAT_TERMS], root: u8, i: u8) -> usize {
        let reads = (0..self.n)
            .filter(|&k| live[k] && !self.gone[k])
            .map(|k| self.srcs(k as u8).iter().filter(|&&s| s == Some(i)).count())
            .sum::<usize>();
        reads + usize::from(root == i)
    }

    /// The first rule of the list that matches instruction `i`, applied.
    fn rule(&mut self, i: u8, live: &[bool; MAT_TERMS], root: u8) -> Result<Option<u8>, MaliError> {
        let flip = |p: (u8, u8), on: bool| if on { (p.1, p.0) } else { p };
        let pair = |s: &Self, x: u8| match s.t[x as usize] {
            Term::Mul(a, b) => Some((a, b)),
            _ => None,
        };
        let sum = |s: &Self, x: u8| match s.t[x as usize] {
            Term::Add(a, b) => Some((a, b)),
            _ => None,
        };
        match self.t[i as usize] {
            Term::Add(p, q) => {
                /* :246, `fadd(fmul(a, b), fmul(a, c))` -> `fmul(a, fadd(b, c))` */
                if let (Some(_), Some(_)) = (pair(self, p), pair(self, q)) {
                    for comb in 0..8 {
                        let (f, s) = flip((p, q), comb & 1 != 0);
                        let (Some(fp), Some(sp)) = (pair(self, f), pair(self, s)) else { continue };
                        let (a, b) = flip(fp, comb & 2 != 0);
                        let (a2, c) = flip(sp, comb & 4 != 0);
                        if a == a2 {
                            let bc = self.mk(Term::Add(b, c))?;
                            return self.mk(Term::Mul(a, bc)).map(Some);
                        }
                    }
                }
                /* :274, `fadd(a, a)` -> `fmul(a, 2.0)` */
                if p == q {
                    let two = self.leaf(Val::Const(F32_TWO), false)?;
                    return self.mk(Term::Mul(p, two)).map(Some);
                }
                /* :275, `fadd(a, fadd(is_used_once)(a, b))` -> `fadd(b, fmul(a, 2.0))` */
                for comb in 0..4 {
                    let (a, inner) = flip((p, q), comb & 1 != 0);
                    let Some(s) = sum(self, inner) else { continue };
                    let (a2, b) = flip(s, comb & 2 != 0);
                    if a2 == a && self.uses(live, root, inner) == 1 {
                        let two = self.leaf(Val::Const(F32_TWO), false)?;
                        let m2 = self.mk(Term::Mul(a, two))?;
                        return self.mk(Term::Add(b, m2)).map(Some);
                    }
                }
                /* :2419, `fadd(fadd(is_used_once)(a(is_fmul), b(is_fmul)),
                 * c(is_not_fmul))` -> `fadd(fadd(a, c), b)` */
                for comb in 0..4 {
                    let (inner, c) = flip((p, q), comb & 1 != 0);
                    let Some(s) = sum(self, inner) else { continue };
                    let (a, b) = flip(s, comb & 2 != 0);
                    if self.is_fmul(a) && self.is_fmul(b) && !self.is_fmul(c) && self.uses(live, root, inner) == 1 {
                        let ac = self.mk(Term::Add(a, c))?;
                        return self.mk(Term::Add(ac, b)).map(Some);
                    }
                }
                Ok(None)
            }
            Term::Mul(p, q) => {
                /* :2387, `fmul(fneg(a), b)` -> `fneg(fmul(a, b))` */
                for (x, y) in [(p, q), (q, p)] {
                    if let Term::Neg(a) = self.t[x as usize] {
                        let ab = self.mk(Term::Mul(a, y))?;
                        return self.mk(Term::Neg(ab)).map(Some);
                    }
                }
                let nonconst = |s: &Self, x: u8| s.konst(x).is_none();
                /* :2394, `fmul(fmul(is_used_once)(a, b), #c)` -> `fmul(fmul(a, c), b)`,
                 * `a` and `b` not constants */
                for comb in 0..4 {
                    let (inner, c) = flip((p, q), comb & 1 != 0);
                    let Some(s) = pair(self, inner) else { continue };
                    let (a, b) = flip(s, comb & 2 != 0);
                    if self.konst(c).is_some()
                        && nonconst(self, a)
                        && nonconst(self, b)
                        && self.uses(live, root, inner) == 1
                    {
                        let ac = self.mk(Term::Mul(a, c))?;
                        return self.mk(Term::Mul(ac, b)).map(Some);
                    }
                }
                /* :2406, `fmul(#a, fmul(b, #c))` -> `fmul(fmul(a, c), b)` */
                for comb in 0..4 {
                    let (a, inner) = flip((p, q), comb & 1 != 0);
                    let Some(s) = pair(self, inner) else { continue };
                    let (b, c) = flip(s, comb & 2 != 0);
                    if self.konst(a).is_some() && self.konst(c).is_some() {
                        let ac = self.mk(Term::Mul(a, c))?;
                        return self.mk(Term::Mul(ac, b)).map(Some);
                    }
                }
                Ok(None)
            }
            Term::Neg(_) | Term::Leaf(_) => Ok(None),
        }
    }

    /// The optimisation loop to its fixed point; the new root.
    fn simplify(&mut self, root: u8) -> Result<u8, MaliError> {
        let mut root = root;
        for _ in 0..16 {
            /* `nir_opt_dce` */
            let live = self.live(root);
            let mut k = 0;
            while k < self.len {
                let i = self.order[k] as usize;
                if live[i] {
                    k += 1;
                } else {
                    self.gone[i] = true;
                    self.order.copy_within(k + 1..self.len, k);
                    self.len -= 1;
                }
            }
            let mut queue = [0u8; 4 * MAT_TERMS];
            let (mut head, mut tail) = (0, 0);
            for k in (0..self.len).rev() {
                queue[tail] = self.order[k];
                tail += 1;
            }
            let mut progress = false;
            while head < tail {
                let i = queue[head];
                head += 1;
                let live = self.live(root);
                if self.gone[i as usize] || !live[i as usize] {
                    continue;
                }
                let Some(pos) = self.order[..self.len].iter().position(|&x| x == i) else { continue };
                self.cursor = Some(pos);
                let r = self.rule(i, &live, root);
                self.cursor = None;
                let Some(r) = r? else { continue };
                if r == i {
                    continue;
                }
                for k in 0..self.n {
                    if self.gone[k] {
                        continue;
                    }
                    self.t[k] = match self.t[k] {
                        Term::Add(a, b) => Term::Add(if a == i { r } else { a }, if b == i { r } else { b }),
                        Term::Mul(a, b) => Term::Mul(if a == i { r } else { a }, if b == i { r } else { b }),
                        Term::Neg(a) if a == i => Term::Neg(r),
                        t => t,
                    };
                }
                if root == i {
                    root = r;
                }
                self.gone[i as usize] = true;
                if let Some(at) = self.order[..self.len].iter().position(|&x| x == i) {
                    self.order.copy_within(at + 1..self.len, at);
                    self.len -= 1;
                }
                for k in 0..self.len {
                    let u = self.order[k];
                    if self.srcs(u).contains(&Some(r)) {
                        if tail == queue.len() {
                            return Err(MaliError::ArithConstant);
                        }
                        queue[tail] = u;
                        tail += 1;
                    }
                }
                progress = true;
            }
            if !progress {
                return Ok(root);
            }
        }
        Err(MaliError::ArithConstant)
    }
}

/// The uniform of the block that holds front-end dword `dword`, and the
/// components it takes in Mesa's packed block (`PackedDriverUniformStorage`,
/// select.rs [`UboLayout`]): a float, vec2, vec3, vec4 or mat4 as the
/// vertex stage reads it (`nir::uniform_of`), or an array of float, vec2,
/// vec3 or vec4 (femtovg's `frag[14]`, case108_uniidx's `u_arr[4]`,
/// deqp_uniarray_*: the front end's `dwords`, each element's place in
/// Mesa's block is [`nir::array_element`]'s). Any other array has no
/// reference yet ([`MaliError::UniformArray`]).
pub fn uniform_extent<'m>(m: &Module<'m>, dword: u16) -> Result<(&'m ir::UniformDecl<'m>, u16), MaliError> {
    nir::uniform_of(m, dword)
}

/// The `load_ubo` Mesa makes for a read at front-end dword `dword` of a
/// highp fragment shader: (its first dword, its components). A vector is
/// loaded whole, a mat4 by its column, a vec4 array by its element
/// (`frag[6]` is one four-component load at its element's offset: the
/// constant index folds into the deref's offset, nir_lower_io); the lanes
/// NIR shrinks away are taken off at the selection ([`shrink_uniforms`]).
fn uniform_load(m: &Module<'_>, dword: u16) -> Result<(u16, u16), MaliError> {
    let (u, total) = uniform_extent(m, dword)?;
    /* an array's element ([`nir::element_load`]) */
    if let Some(load) = nir::element_load(u, dword) {
        return Ok(load);
    }
    Ok(if total == 16 { (u.dword_off + (dword - u.dword_off) / 4 * 4, 4) } else { (u.dword_off, total) })
}

/// `nir_opt_shrink_vectors` for the uniform loads of a highp shader: each
/// keeps the lanes up to the highest one read (the vertex stage's `shrink`,
/// nir.rs; a load the colour alone reads counts its stores, pushed before
/// this runs). It never takes off the lanes below the lowest one read
/// (`shrink_start` is false, bifrost_nir.c:286): which words Mesa pushes
/// is the layout's rule ([`super::select::UboLayout::load_of`]).
fn shrink_uniforms(f: &mut Fs<'_>) {
    for k in 0..f.nir.len {
        let id = f.nir.order[k];
        let Kind::Uniform { dword, .. } = f.nir.nodes[id as usize].kind else { continue };
        let mut top = 0u8;
        for (u, s) in f.nir.uses(id) {
            if let Val::Lane(_, c) = f.nir.nodes[u as usize].src[s] {
                top = top.max(c + 1);
            }
        }
        f.nir.nodes[id as usize].kind = Kind::Uniform { dword, comps: top };
    }
}

/// The colour's lane `root` reads lane `e` (an input's or a texel) through
/// its tree.
fn reaches(f: &Fs<'_>, root: Val, e: Val) -> bool {
    let mut stack = [Val::Undef; MAX_NODES];
    let mut sp = 1;
    stack[0] = root;
    let mut seen = [false; MAX_NODES];
    while sp > 0 {
        sp -= 1;
        let v = stack[sp];
        if v == e {
            return true;
        }
        let Val::Lane(n, _) = v else { continue };
        if seen[n as usize] {
            continue;
        }
        seen[n as usize] = true;
        if let Some((arity, src)) = f.srcs_of(v) {
            for y in &src[..arity] {
                if *y != Val::Undef && sp < stack.len() {
                    stack[sp] = *y;
                    sp += 1;
                }
            }
        }
    }
    false
}

/// Mesa's link keeps every lane of varying `loc` (its input node `node`),
/// not only the lanes the program reads (`roots`): true when GLSL's
/// `lower_precision` converts it in a tree it lowers. The pass converts
/// the whole variable at each dereference (`f2fmp` of the deref, before
/// the swizzle: lower_precision.cpp:724-733), and the link
/// (`nir_opt_varyings_bulk`, gl_nir_linker.c:1524) runs while that vector
/// conversion still reads every lane of the loads; the lanes no one reads
/// are dropped only when the fragment shader is compiled after it
/// (`nir_lower_alu_width`, then `nir_opt_dce`). Measured with Mesa
/// 26.2.1's driver on the drm-shim: a mediump float varying of a mediump
/// shader that an fp16 operation reads (fsvec_*, the discards of
/// discard_81_branchquad, the integers of intloop_*), or that a lowered
/// colour takes as it is (cmp_*, fsload_*), is stored and loaded whole.
/// A highp shader, a highp varying and an integer one (`caps->int16` is
/// false, pan_screen.c:591) are not converted, and neither is a colour
/// that holds an fp32 value (case 89's `float(v_idx) / 4.0`,
/// intbool_89_flatlast; a highp varying's lane, hp_var2; a highp
/// uniform's, hp_uni; a highp sampler's texel, hp_tex): those keep the
/// lanes read. A highp ES 3.00 output is a lowered tree converted back
/// (es3_hpout loads `.v4`). A colour lane of a form not measured either
/// way is refused ([`MaliError::FsVaryingLoad`]).
fn kept_whole(
    f: &Fs<'_>,
    m: &Module<'_>,
    loc: u8,
    node: u16,
    roots: &[Val],
    all_read: bool,
) -> Result<bool, MaliError> {
    let mediump = |l: u8| {
        !f.prec.highp
            && (l as usize) < MAX_VARYINGS
            && f.prec.other >> (l & 31) & 1 == 0
            && m.inputs.iter().any(|d| d.kind == IoKind::Varying && d.location == l && is_float_varying(d.gl_type))
    };
    /* a highp shader (fsload_highp), a highp varying (fsload_hp_var2), an
     * integer one (intbool_89_flatlast's v_idx): no `f2fmp`, the lanes
     * read */
    if !mediump(loc) {
        return Ok(false);
    }
    let reads = |src: &[Val]| src.iter().any(|s| matches!(*s, Val::Lane(n, _) if n == node));
    let reached =
        |id: u16, width: u8| (0..width.max(1)).any(|k| roots.iter().any(|&r| reaches(f, r, Val::Lane(id, k))));
    let order = &f.nir.order[..f.nir.len];
    /* an fp16 operation reads a lane: arithmetic, a phi or a compare of
     * the lowered tree (fsvec_fract_mediump.expect.json loads `.v4` where
     * `fract(v.x)` reads one lane; discard_81_branchquad's compares read
     * y and w) */
    let fp16_op = order.iter().any(|&id| {
        let nd = f.nir.nodes[id as usize];
        let arity = match nd.kind {
            Kind::Alu(op) => op.arity(),
            Kind::Phi => 2,
            _ => 0,
        };
        nd.bits == 16 && reads(&nd.src[..arity]) && reached(id, nd.width)
    });
    /* an integer made of fp16 lanes (intloop_int_compare.expect.json and
     * intloop_tri_int.expect.json load `.v4` where the colour reads two
     * and three lanes) */
    let int_of = order.iter().any(|&id| {
        let nd = f.nir.nodes[id as usize];
        let from_lane = |v: Val| match f.alu_of(v) {
            Some((op, s)) if !op.is_int() => reads(&s[..op.arity()]),
            _ => false,
        };
        nd.kind == Kind::Alu(Alu::F2i32) && from_lane(nd.src[0]) && reached(id, 1)
    });
    /* an integer's conversion of the lanes as loaded (`f2i32(f2fmp(a))` is
     * `f2i32(a)`, nir_opt_algebraic.py:2002). Measured: a scalar `int(v.z)`
     * leaves the lanes read (deqp_intval_opfs_add_int_mediump's `int(v_in0.z)`
     * and `int(v_in1.x)`: one `.v2` load of the two), a vector conversion
     * through a swizzle keeps the varying whole (deqp_intval_opfs_cmp_ivec3's
     * `ivec3(v_in0.zxy)` and deqp_intval_opfs_sub_assign_ivec2_lowp's
     * `ivec2(v_in0.wy)`: `.v4` loads, every lane stored). The vector's
     * `f2fmp` is still read whole at the link; which pass keeps it there
     * is not read from Mesa, the bytes are */
    let vector_int = order.iter().any(|&id| {
        let nd = f.nir.nodes[id as usize];
        nd.kind == Kind::Alu(Alu::F2i) && reads(&nd.src[..1]) && reached(id, 1) && f.is_vector_node(id)
    });
    if fp16_op || int_of || vector_int {
        return Ok(true);
    }
    /* the colour takes a lane as it is: converted with the colour's tree
     * when lower_precision lowers that tree, which it does when every lane
     * is mediump */
    let colour = f.colour.ok_or(MaliError::NoColour)?;
    if !reads(&colour) {
        /* no fp16 operation and no colour lane reads it: a cube look-up's
         * direction, read raw at fp32 (varyconst_101_cubebias: the lanes
         * read, `.v2`), or a scalar integer conversion of the lane as
         * loaded (above); any other reader has no reference */
        let other = order.iter().any(|&id| {
            let nd = f.nir.nodes[id as usize];
            let (arity, src) = f.srcs_of(Val::Lane(id, 0)).unwrap_or((0, nd.src));
            nd.kind != Kind::Tex && nd.kind != Kind::Alu(Alu::F2i) && reads(&src[..arity]) && reached(id, nd.width)
        });
        return if other { Err(MaliError::FsVaryingLoad) } else { Ok(false) };
    }
    let (mut fp32, mut unknown) = (false, false);
    for v in colour {
        let n = match v {
            Val::Const(_) => continue,
            Val::Lane(n, _) => n,
            _ => {
                unknown = true;
                continue;
            }
        };
        let nd = f.nir.nodes[n as usize];
        match nd.kind {
            /* gl_PointCoord, mediump in ES 1.00 (pointcoordvary.expect.json:
             * `v_color.b` beside it loads `.v4`) */
            Kind::Input { .. } if Some(n) == f.pcoord => {}
            Kind::Input { .. } if Some(n) == f.fz => unknown = true,
            Kind::Input { .. } => match f.inputs.iter().position(|&x| x == Some(n)) {
                Some(l) if mediump(l as u8) => {}
                /* hp_var2: `vec4(a.x, b.y, ..)`, b highp, keeps a.x alone */
                Some(_) => fp32 = true,
                None => unknown = true,
            },
            Kind::Alu(_) | Kind::Phi if nd.bits == 16 => {}
            /* a mediump uniform's component, read at a constant or a
             * run-time index (uni_m, uni_mf; uniidx_j's `fcols[fidx].ba`
             * beside `v_color.rg`: `.v4`); a highp one is fp32 (hp_uni:
             * `vec4(a.x, u.y, ..)` keeps a.x alone) */
            Kind::Uniform { .. } | Kind::UniformIdx { .. } if nd.bits == 16 => {}
            Kind::Uniform { .. } | Kind::UniformIdx { .. } if nd.bits == 32 => fp32 = true,
            /* case 89 */
            Kind::Alu(_) | Kind::Phi if nd.bits == 32 => fp32 = true,
            /* a texel of an fp16 look-up (tex_col: `vec4(texture2D(s,
             * a.xy).x, a.z, ..)` loads `.v4`) */
            Kind::Tex if nd.bits == 16 => {}
            /* a highp sampler's texel is fp32 (hp_tex: `vec4(a.x,
             * texture2D(s, t).x, ..)` keeps a.x and t) */
            Kind::Tex if nd.bits == 32 => fp32 = true,
            _ => unknown = true,
        }
    }
    match (unknown, fp32) {
        (true, _) => Err(MaliError::FsVaryingLoad),
        (false, true) => Ok(false),
        /* a lowered colour: the lane keeps its varying whole through a
         * value whose `f2fmp` the link keeps, not through ones it undoes
         * ([`colour_keeps_whole`]) */
        (false, false) => colour_keeps_whole(m, loc, all_read),
    }
}

/// The colour keeps a varying's `f2fmp` through the link (true), or takes
/// its lanes only through values whose conversion the link undoes
/// (false). gl_nir_opts runs before `nir_opt_varyings_bulk`
/// (gl_nir_linker.c:1441, :1524): `nir_lower_vars_to_ssa` makes a local
/// what was stored, the whole-identity folds remove `* 1.0` and `+ 0.0`
/// (nir_opt_algebraic.py:236-237, :292), copy propagation makes a vector
/// of one value's lanes a swizzle of it, `f2f32(f2fmp(a))` is `a` (:1991)
/// and DCE drops a dead tree. Each colour store is judged by the lanes it
/// writes, each followed through locals stored once in that lane,
/// swizzles and whole identities ([`ir_colour_lane`]); every answer is a
/// measured form (drm-shim references), anything else is refused
/// ([`MaliError::FsVaryingLoad`]) unless the colour reads every lane of
/// the varying (`all_read`: kept or not, the layout is the same):
/// - every lane written is this varying's: one swizzle of its `f2fmp`,
///   cancelled, the lanes read (fsload_bare1-4, fsload_cancel1-8,
///   fsload_cancel10-11, fsload_ctor1: `t.xy` of `vec4(a.xy, 0.5, 1.0)`);
/// - a lane of it beside a constant, another varying's lane or
///   gl_PointCoord: `f2f32` reads a vector, the `f2fmp` survives, whole
///   (fsload_v4_one, fsload_ctor3-5, cmp_axby, case 105's pointcoordvary);
/// - a lane that is unknown but reaches the varying beside a constant
///   (a local stored on several paths, `intloop_ifloop`'s `vec4(res.rgb,
///   1.0)`): the constant makes a vector all the same, whole.
///
/// A store that writes no lane of it is not followed, so a dead tree reads
/// nothing (fsload_cancel9).
fn colour_keeps_whole(m: &Module<'_>, loc: u8, all_read: bool) -> Result<bool, MaliError> {
    use ir::Op;
    let (mut whole, mut unsure) = (false, false);
    for op in m.instrs {
        let (src, mask) = match *op {
            Op::StoreOutput { slot: IoSlot::FragColor(_), src } => (src, 0xf),
            Op::StoreLocal { local, index: None, src, write_mask } if ir_colour_local(m, local) => (src, write_mask),
            _ => continue,
        };
        /* the colour's own local, stored by its parts above */
        if matches!(m.def(src.val), Some(&Op::LoadLocal { local, .. }) if ir_colour_local(m, local)) {
            continue;
        }
        let (mut mine, mut beside, mut reaching, mut other) = (false, false, false, false);
        for i in (0..4).filter(|&i| mask & 1 << i != 0) {
            match ir_colour_lane(m, src.val, src.swizzle[i] & 3, 0) {
                ColourLane::Varying(l) if l == loc => mine = true,
                ColourLane::Varying(_) | ColourLane::Const | ColourLane::PointCoord => beside = true,
                ColourLane::Unknown(reach) if reach & 1 << (loc & 31) != 0 => reaching = true,
                ColourLane::Unknown(_) => other = true,
            }
        }
        if !mine && !reaching {
            continue;
        }
        if beside {
            whole = true;
        } else if mine && !reaching && !other {
            /* the varying's lanes alone: cancelled at the link */
        } else {
            /* beside a value of no measured kind (an operation, a uniform,
             * a texel: uniidx_j), or an unknown lane of it alone */
            unsure = true;
        }
    }
    if whole {
        Ok(true)
    } else if unsure && !all_read {
        Err(MaliError::FsVaryingLoad)
    } else {
        Ok(false)
    }
}

/// What one lane of a colour store holds ([`ir_colour_lane`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ColourLane {
    /// A lane of the varying at this location.
    Varying(u8),
    Const,
    PointCoord,
    /// Another value: the varying locations it may read (all of them when
    /// not known).
    Unknown(u32),
}

/// Lane `c` of value `v`, followed through locals with one store of that
/// lane, swizzles and whole identities ([`ir_whole_identity`]). A value of
/// a kind no reference names is [`ColourLane::Unknown`] with the varyings
/// it reads.
fn ir_colour_lane(m: &Module<'_>, v: ir::ValueId, c: u8, depth: u8) -> ColourLane {
    if depth > 32 {
        return ColourLane::Unknown(u32::MAX);
    }
    match m.def(v) {
        Some(&ir::Op::LoadInput { slot: IoSlot::Varying(l), .. }) => ColourLane::Varying(l),
        Some(&ir::Op::LoadInput { slot: IoSlot::PointCoord, .. }) => ColourLane::PointCoord,
        Some(&ir::Op::ConstF32(_)) => ColourLane::Const,
        Some(&ir::Op::LoadLocal { local, index: None, .. }) if !ir_colour_local(m, local) => {
            let mut stores = m.instrs.iter().filter_map(|op| match *op {
                ir::Op::StoreLocal { local: l, index: None, src, write_mask }
                    if l == local && write_mask & 1 << c != 0 =>
                {
                    Some(src)
                }
                _ => None,
            });
            match (stores.next(), stores.next()) {
                (Some(s), None) => ir_colour_lane(m, s.val, s.swizzle[c as usize & 3] & 3, depth + 1),
                _ => ColourLane::Unknown(ir_reach(m, v, &mut 0, 0)),
            }
        }
        Some(&ir::Op::Alu(a, s)) => match ir_whole_identity(m, a, &s, width_of(m, v)) {
            Some(o) => ir_colour_lane(m, o.val, o.swizzle[c as usize & 3] & 3, depth + 1),
            None => ColourLane::Unknown(ir_reach(m, v, &mut 0, 0)),
        },
        _ => ColourLane::Unknown(ir_reach(m, v, &mut 0, 0)),
    }
}

/// The varying locations value `v` reads, through every store of a local.
fn ir_reach(m: &Module<'_>, v: ir::ValueId, seen: &mut u64, depth: u8) -> u32 {
    if depth > 32 {
        return u32::MAX;
    }
    match m.def(v) {
        Some(&ir::Op::LoadInput { slot: IoSlot::Varying(l), .. }) => 1 << (l & 31),
        Some(&ir::Op::LoadLocal { local, .. }) => {
            if *seen >> (local.0 & 63) & 1 != 0 {
                return 0;
            }
            *seen |= 1 << (local.0 & 63);
            let mut r = 0;
            for op in m.instrs {
                if let ir::Op::StoreLocal { local: l, src, .. } = *op {
                    if l == local {
                        r |= ir_reach(m, src.val, seen, depth + 1);
                    }
                }
            }
            r
        }
        Some(&ir::Op::Alu(a, s)) => s[..a.arity()].iter().fold(0, |r, x| r | ir_reach(m, x.val, seen, depth + 1)),
        _ => 0,
    }
}

/// Local `l` is the colour's own (`gl_FragColor` written by parts, an ES
/// 3.00 output): its stores are the colour's ([`colour_keeps_whole`]).
fn ir_colour_local(m: &Module<'_>, l: ir::LocalId) -> bool {
    m.locals.get(l.0 as usize).is_some_and(|d| {
        d.name == "gl_FragColor" || m.outputs.iter().any(|o| o.kind == IoKind::Colour && o.name == d.name)
    })
}

/// The lanes of the IR value `id`: the exact width where the front end
/// records one ([`ir::Module::lanes_of`]: a built-in's steps, an increment's
/// sum, a conversion, whose `comps` is the blanket four: `smoothstep` of a
/// float is 1 lane, `abs` of a `vec3` 3), `comps` elsewhere. Every width
/// the walk reads is this one, 1 to 4: a lane past it has no value.
fn width_of(m: &Module<'_>, id: ir::ValueId) -> usize {
    usize::from(m.lanes_of(id).unwrap_or_else(|| m.comps_of(id)))
}

/// The operand a whole identity leaves of `a` over `s` (`w` lanes): the
/// other source of a `* 1.0`, or of a `+ 0.0` or `+ -0.0` with one sign of
/// zero on every lane (nir_opt_algebraic.py:236-237, :292; nir_search.c
/// :260-286, the sign of zero checked per lane, :282-285).
fn ir_whole_identity(m: &Module<'_>, a: AluOp, s: &[ir::Src; 3], w: usize) -> Option<ir::Src> {
    let bits: &[u32] = match a {
        AluOp::Fmul => &[F32_ONE],
        AluOp::Fadd => &[0, F32_NEG_ZERO],
        _ => return None,
    };
    let konst = |v: ir::ValueId, swz: &[u8], b: u32| match m.def(v) {
        Some(ir::Op::ConstF32(c)) => swz.iter().all(|&i| c.get(i as usize & 3) == Some(&b)),
        _ => false,
    };
    (0..2).find_map(|k| {
        let c = &s[k].swizzle[..w.clamp(1, 4)];
        bits.iter().any(|&b| konst(s[k].val, c, b)).then_some(s[1 - k])
    })
}

/// `gl_type` names a float varying: a scalar, a vector or a matrix's
/// column (an integer one is not lowered, [`kept_whole`]).
fn is_float_varying(gl_type: u16) -> bool {
    use crate::gl::consts as gl;
    let t = gl_type as u32;
    matches!(t, gl::GL_FLOAT | gl::GL_FLOAT_VEC2 | gl::GL_FLOAT_VEC3 | gl::GL_FLOAT_VEC4) || is_matrix(gl_type)
}

/// The varyings the colour reads, laid out and compacted as Mesa's link
/// does, the slots read loaded in the order of their first reads
/// (docs/notes/mali-partial-varyings.md, "The lanes the link keeps"). The
/// layout takes them in declaration order, or in the program's vertex
/// shader's order when the link compiled this shader for it (`order`,
/// link.rs `VaryingOrder`: Mesa's fragment shader has the vertex shader's
/// layout).
pub fn layout(f: &mut Fs<'_>, m: &Module<'_>, order: Option<super::link::VaryingOrder>) -> Result<(), MaliError> {
    /* colour outputs, one a render target; those past location 0 a
     * `vec4` as output 0 is ([`Fs::targets`]) */
    let first = m.outputs.iter().find(|o| o.kind == IoKind::Colour && o.location == 0);
    let target = |o: &ir::IoDecl<'_>| {
        o.kind == IoKind::Colour
            && (o.location == 0
                || ((o.location as usize) < MAX_TARGETS
                    && o.components == 4
                    && first.is_some_and(|z| z.gl_type == o.gl_type && z.components == 4)))
    };
    if !m.outputs.iter().all(target) {
        return Err(MaliError::FragOutput);
    }
    let c = f.colour.ok_or(MaliError::NoColour)?;
    if c.contains(&Val::Undef) {
        return Err(MaliError::ColourForm);
    }
    /* a further target's lanes are uniform words and constants, which
     * load nothing and compute nothing (fsout_55_es3mrt, fsout_106_mrt3);
     * a computed one has no reference of where Mesa puts its code */
    let leaf = |v: &Val| match *v {
        Val::Const(_) => true,
        Val::Lane(n, _) => matches!(f.nir.nodes[n as usize].kind, Kind::Uniform { .. }),
        _ => false,
    };
    if !targets(f).all(|(_, t)| t.iter().all(leaf)) {
        return Err(MaliError::FragOutput);
    }
    /* what the program reads: the colour's lanes, each `if`'s condition
     * and each discard's */
    let mut roots = [Val::Undef; 4 + nir::MAX_IFS + MAX_DISCARDS];
    roots[..4].copy_from_slice(&c);
    let mut nroots = 4;
    for r in f.nir.live_ifs() {
        roots[nroots] = r.cond;
        nroots += 1;
    }
    for id in f.nir.discards() {
        roots[nroots] = f.nir.nodes[id as usize].src[0];
        nroots += 1;
    }
    let roots = &roots[..nroots];
    let c = roots;
    /* gl_FragCoord's w (`load_frag_coord_w`), and a mediump shader's z,
     * are not lowered */
    if let Some(zw) = f.frag_zw {
        if (0..2).any(|k| c.iter().any(|&root| reaches(f, root, Val::Lane(zw, k)))) {
            return Err(MaliError::SpecialInput);
        }
    }
    let mut read = [0u8; MAX_VARYINGS];
    let mut whole = [false; MAX_VARYINGS];
    let (mut pcoord, mut fz) = (false, false);
    /* the inputs the walk loaded: each varying location, gl_PointCoord
     * and gl_FragCoord.z */
    let locs = (0..MAX_VARYINGS as u8).filter(|&l| f.inputs[l as usize].is_some());
    for loc in locs.chain(f.pcoord.map(|_| POINT_COORD)).chain(f.fz.map(|_| FRAG_Z)) {
        let node = match loc {
            POINT_COORD => f.pcoord,
            FRAG_Z => f.fz,
            _ => f.inputs[loc as usize],
        };
        let Some(node) = node else { continue };
        for comp in 0..4u8 {
            let reached = c.iter().any(|&root| reaches(f, root, Val::Lane(node, comp)));
            match loc {
                POINT_COORD => pcoord |= reached,
                FRAG_Z => fz |= reached,
                _ if reached => *read.get_mut(loc as usize).ok_or(MaliError::VaryingLocation)? |= 1 << comp,
                _ => {}
            }
        }
        /* a varying a dot product reads is read whole: the link sees the
         * `fdot`'s vector source, not its lanes (measured: `dot(v.xy,
         * vec2(2.0, 3.0))` keeps all four lanes of v, pan_shc-out/
         * s4_motion_dot.json, where `v.x * 2.0 + v.y * 3.0` keeps two) */
        if (loc as usize) < MAX_VARYINGS && read[loc as usize] != 0 {
            let order = &f.nir.order[..f.nir.len];
            whole[loc as usize] = order.iter().any(|&id| {
                let nd = f.nir.nodes[id as usize];
                f.is_dot_node(m, id)
                    && nd.src.iter().any(|s| matches!(*s, Val::Lane(n, _) if n == node))
                    && c.iter().any(|&root| reaches(f, root, Val::Lane(id, 0)))
            });
            /* and one GLSL's `lower_precision` converts whole before the
             * link ([`kept_whole`]) */
            let comps = m.inputs.iter().find(|d| d.kind == IoKind::Varying && d.location == loc).map(|d| d.components);
            let all_read = comps.is_some_and(|n| read[loc as usize] == (1u8 << n.min(4)) - 1);
            whole[loc as usize] |= kept_whole(f, m, loc, node, c, all_read)?;
        }
    }
    /* the texels the colour reads of each look-up; one it reads reads its
     * coordinate's two lanes (Mesa's DCE takes one the colour does not read) */
    for j in 0..f.nlook {
        let n = f.look_ups[j].node;
        let lanes = (0..4u8).filter(|&k| c.iter().any(|&root| reaches(f, root, Val::Lane(n, k))));
        f.look_ups[j].lanes = lanes.fold(0, |m, k| m | 1 << k);
        if let (true, TexCoord::Varying(loc)) = (f.look_ups[j].live(), f.look_ups[j].coord) {
            *read.get_mut(loc as usize).ok_or(MaliError::VaryingLocation)? |= 0b11;
        }
    }
    /* a size query the colour reads a lane of ([`SizeQuery::live`]) */
    for k in 0..f.nsizes {
        let outs = f.sizes[k].out;
        f.sizes[k].live = outs.iter().any(|&o| o != Val::Undef && c.iter().any(|&root| reaches(f, root, o)));
    }
    /* the descriptor index of each: its sampler's rank among the samplers
     * the program reads (a look-up's, a size query's: texmore_46 reads
     * `tex` at 0 and `arr` at 1 with no look-up), by binding (the
     * declaration order; [`TEX_INDEX`]) */
    let read_sampler = |f: &Fs<'_>, x: u8| {
        f.look_ups().iter().any(|t| t.live() && t.sampler == x)
            || f.sizes[..f.nsizes].iter().any(|q| q.live && q.sampler == x)
    };
    for j in 0..f.nlook {
        let b = f.look_ups[j].sampler;
        f.look_ups[j].index = (0..b).filter(|&x| read_sampler(f, x)).count() as u8;
    }
    for k in 0..f.nsizes {
        let b = f.sizes[k].sampler;
        f.sizes[k].index = (0..b).filter(|&x| read_sampler(f, x)).count() as u8;
    }
    let decls = m.inputs.iter().filter(|d| d.kind == IoKind::Varying && read.get(d.location as usize) > Some(&0));
    /* a declared varying the colour reads no lane of: Mesa's link removes
     * it from the vertex shader too (deqp_misc_cmp_ine0_hp keeps no
     * varying program), so the vertex shader is built for the reads */
    f.compacted |= m.inputs.iter().any(|d| d.kind == IoKind::Varying && read.get(d.location as usize) == Some(&0));
    for d in decls.clone() {
        /* a `flat` one is loaded `.src_flat32` (intbool_89_flatlast) */
        if d.interp != ir::INTERP_SMOOTH && d.interp != ir::INTERP_FLAT {
            return Err(MaliError::Interpolation);
        }
    }
    /* (rank, location, components): a stable insertion by the vertex rank */
    let mut by = [(0u8, 0u8, 0u8, 0u16); MAX_VARYINGS];
    let mut nd = 0;
    for d in decls {
        let rank = order.map_or(0, |o| o.rank(d.location));
        if nd >= MAX_VARYINGS {
            return Err(MaliError::VaryingLocation);
        }
        let at = by[..nd].iter().position(|x| x.0 > rank).unwrap_or(nd);
        by.copy_within(at..nd, at + 1);
        by[at] = (rank, d.location, d.components, d.gl_type);
        nd += 1;
    }
    let mut lay = [VaryingAt { location: 0, comps: 0, offset: 0 }; MAX_VARYINGS];
    let n = generic_layout(by[..nd].iter().map(|&(_, l, c, t)| (l, c, t)), &mut lay)?;
    /* `compact_varyings` (nir_opt_varyings.c:5491-5560, `fs_assign_slots`
     * in scalar slot order): every lane the link keeps, in layout order,
     * takes the next scalar slot from VAR0; a vertex shader's constant
     * lane is not one, the link propagates it into this shader */
    let mut s = 0u16;
    /* each 16-byte slot's lanes, and those of them the program reads */
    let (mut held, mut used) = ([0u8; MAX_VARYINGS], [0u8; MAX_VARYINGS]);
    /* each slot's first read: the first read of a varying with a lane read
     * there (`nir_opt_move` puts a load before its first use,
     * bifrost_nir.c:381; v3x_v3z_p3 loads q.yz before p.xyz,q.x, whose
     * q.x no one reads), the earlier slot first where they are equal (the
     * conversions of a vector, in lane order, nir_lower_alu_width) */
    let mut first_read = [u32::MAX; MAX_VARYINGS];
    /* (a flat lane kept, an interpolated lane kept): the link packs flat
     * lanes in slots of their own, a layout no reference draws beside
     * interpolated ones (nir.rs refuses the pair's vertex shader too) */
    let (mut flat, mut smooth) = (false, false);
    /* the lanes laid out, by the first vertex lane of their value
     * (`deduplicate_outputs`, nir_opt_varyings.c:3253-3420, in scalar slot
     * order: the first kept, the others read from it; fsload_dup_swz's
     * q.x is p.y): (that lane, slot, lane) */
    let mut placed = [(((0u8, 0u8), 0u8), 0u8, 0u8); 4 * MAX_VARYINGS];
    let mut nplaced = 0;
    for at in &lay[..n] {
        let loc = at.location as usize & 15;
        let first = s;
        let full = (1u8 << at.comps) - 1;
        let mut keep = if whole[loc] {
            let konst = |c: u8| f.prec.consts.get(at.location, c).is_some();
            (0..at.comps).filter(|&c| !konst(c)).fold(read[loc], |m, c| m | 1 << c)
        } else {
            read[loc]
        };
        for comp in 0..at.comps {
            if keep & (1 << comp) == 0 {
                continue;
            }
            /* within one interpolation qualifier (`get_input_qualifier`'s
             * tables, nir_opt_varyings.c:3042-3136, 3298-3305: b12's flat q
             * beside the smooth p, both a_a.xy, stays apart) */
            let interp = m.inputs.iter().find(|d| d.kind == IoKind::Varying && d.location == at.location);
            let interp = interp.map_or(ir::INTERP_SMOOTH, |d| d.interp);
            let class = (f.prec.consts.class(at.location, comp), interp);
            if let Some(&(_, slot, lane)) = placed[..nplaced].iter().find(|x| x.0 == class) {
                /* a lane equal to one laid out: read from it, not stored */
                keep &= !(1 << comp);
                if read[loc] & (1 << comp) != 0 {
                    used[slot as usize] |= 1 << lane;
                    first_read[slot as usize] = first_read[slot as usize].min(f.var_key[loc]);
                    f.lane_of[loc][comp as usize] = Some((slot, lane));
                }
                continue;
            }
            let declared = m
                .inputs
                .iter()
                .any(|d| d.kind == IoKind::Varying && d.location == at.location && d.interp == ir::INTERP_FLAT);
            if declared || f.prec.consts.flat(at.location, comp) {
                flat = true;
            } else {
                smooth = true;
            }
            let (slot, lane) = ((s / 4) as usize, (s % 4) as u8);
            *held.get_mut(slot).ok_or(MaliError::VaryingLocation)? |= 1 << lane;
            *placed.get_mut(nplaced).ok_or(MaliError::VaryingLocation)? = (class, slot as u8, lane);
            nplaced += 1;
            if read[loc] & (1 << comp) != 0 {
                used[slot] |= 1 << lane;
                first_read[slot] = first_read[slot].min(f.var_key[loc]);
                f.lane_of[loc][comp as usize] = Some((slot as u8, lane));
            }
            s += 1;
        }
        f.compacted |= keep != full;
        f.reads[f.nreads] = FsRead { location: at.location, mask: keep, offset: 4 * first };
        f.nreads += 1;
    }
    if flat && smooth {
        return Err(MaliError::VaryingNotFromAttribute);
    }
    /* a slot is loaded when the program reads a lane of it, at the width
     * the link left it (`nir_unlower_io_to_vars` makes one variable of the
     * slot's lanes, loaded whole; `pan_nir_lower_fs_inputs` loads
     * `component + num_components`, pan_nir_lower_fs_inputs.c:52-69, and no
     * pass shrinks it before: v3x_v3z_p3's `.v4` of which p.x alone is
     * read); a slot no one reads is not loaded (`nir_opt_dce`), though the
     * vertex shader stores it (v3x_v3x_p1) */
    let mut slots = [FsVarying::NONE; MAX_VARYINGS];
    let mut nslots = 0;
    for slot in 0..s.div_ceil(4) as usize {
        if used[slot] == 0 {
            continue;
        }
        let mut v = FsVarying::new(slot as u8, held[slot].count_ones() as u8, 16 * slot as u16, first_read[slot]);
        v.flat = flat;
        v.whole = used[slot] != held[slot];
        slots[nslots] = v;
        nslots += 1;
    }
    /* gl_PointCoord is loaded as its two lanes, of which the colour may
     * read one (pcv_c: `LD_VAR_SPECIAL.v2` for `gl_PointCoord.x`) */
    let mut pc = FsVarying::new(POINT_COORD, 2, 0, f.pcoord_key);
    pc.whole = true;
    let z = FsVarying::new(FRAG_Z, 1, 0, f.fz_key);
    for v in slots[..nslots].iter().copied().chain(pcoord.then_some(pc)).chain(fz.then_some(z)) {
        if f.nvar >= MAX_VARYINGS {
            return Err(MaliError::VaryingLocation);
        }
        let k = f.varyings[..f.nvar].iter().position(|x| x.key() > v.key()).unwrap_or(f.nvar);
        f.varyings.copy_within(k..f.nvar, k + 1);
        f.varyings[k] = v;
        f.nvar += 1;
    }
    /* the coordinate is the first two lanes of one load, read as fp32 (a
     * load of its own, or the first of a packed slot: femtovg's ftcoord
     * before fpos, fvg_fill_color_glyph_nodiscard.expect.json's
     * `LD_VAR_BUF_IMM ... .v4 @r0:r1:r2:r3` then `TEX_SINGLE ... @r0:r1`) */
    for j in 0..f.nlook {
        if let (true, TexCoord::Varying(loc)) = (f.look_ups[j].live(), f.look_ups[j].coord) {
            let ((k, x), (k1, y)) = (index(f, loc, 0)?, index(f, loc, 1)?);
            /* or lanes 2 and 3 of a slot the link packed two vec2
             * varyings in (`compact_varyings`; gm2_fclane_2's `t0` and `t1`,
             * `LD_VAR_BUF_IMM ... .v4 @r4:r5:r6:r7` then `TEX_SINGLE ...
             * @r6:r7, @r6:r7` and `TEX_SINGLE ... @r4:r5, @r4:r5`) */
            if k != k1 || !matches!((x, y), (0, 1) | (2, 3)) {
                return Err(MaliError::TexCoord);
            }
            f.look_ups[j].load = Some(k);
            f.look_ups[j].load_lane = x;
        }
    }
    Ok(())
}

/// `gl_type` names a float matrix (a varying declared once per column).
fn is_matrix(gl_type: u16) -> bool {
    use crate::gl::consts as gl;
    let t = gl_type as u32;
    (gl::GL_FLOAT_MAT2..=gl::GL_FLOAT_MAT4).contains(&t) || (gl::GL_FLOAT_MAT2x3..=gl::GL_FLOAT_MAT4x3).contains(&t)
}

/// Lane `c` of the input at `loc`: its load's index and its lane there.
fn index(f: &Fs<'_>, loc: u8, c: u8) -> Result<(u8, u8), MaliError> {
    let (slot, lane) = if loc == POINT_COORD || loc == FRAG_Z {
        (loc, c)
    } else {
        f.lane_of.get(loc as usize).and_then(|x| x[c as usize & 3]).ok_or(MaliError::Defect(Defect::IrIndex))?
    };
    let i = f.varyings[..f.nvar].iter().position(|v| v.location == slot);
    i.map(|i| (i as u8, lane)).ok_or(MaliError::Defect(Defect::IrIndex))
}

/// A two-lane leaf of the colour (the `P` forms): a converted pair of
/// load `k` from lane `h` (`swz` H01, or H10 swapped), one converted lane
/// in both halves, a converted lane and a constant half (`vec2`), a
/// constant pair, or word `w` of the fp16 uniform block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Leaf {
    Half {
        k: u8,
        h: u8,
        swz: Swz,
    },
    Splat {
        k: u8,
        c: u8,
    },
    Mk {
        k: u8,
        c: u8,
        half: u16,
        /// The constant is the low half: `MKVEC.v2i16 <half>.h0, x.h0`
        /// (deqp_colourpair_ck_lane_highpcolour's `MKVEC.v2i16 r1, 0x0.h0,
        /// r0.h0`).
        first: bool,
    },
    /// Lane `c` of load `k` and lane `d` of load `e`, not one aligned pair,
    /// each converted alone, as a word: `MKVEC.v2i16 x.h0, y.h0`
    /// (varyconst_78_matvary.expect.json: `FADD.f32 r0.h0, r0^, 0x0.neg`,
    /// `FADD.f32 r1.h0, r3^, 0x0.neg`, then `MKVEC.v2i16 r0, r0^.h0,
    /// r1^.h0` of a `.v4` load's lanes 0 and 3).
    Lanes {
        k: u8,
        c: u8,
        e: u8,
        d: u8,
    },
    Const(u32),
    /// Word `base + comp` of the fp16 uniform block (`base` the first word
    /// of the uniform's load, which the FAU push counts from), its two
    /// halves (`H01`) or one of them in both lanes (`H00`, `H11`: a float
    /// uniform, vtl_fs's `u2.w0.h00`).
    Uniform {
        base: u16,
        comp: u8,
        swz: Swz,
    },
    /// Word `h` of look-up `j`'s fp16 result ([`Fs::look_ups`]): texels
    /// `2h` and `2h + 1`.
    Tex(u8, u8),
    /// That word with its halves swapped (texels `2h + 1`, `2h`), an
    /// output word only: `MOV.i32` of the word `.h10`, which opt.rs makes
    /// `IADD.v2u16 r0, r0^.h10, 0x0` (gm2_texswz_3's `.yx`).
    TexSwapped(u8, u8),
    /// Word `h` of the uniform array element read at a run-time index
    /// (`LD_PKA.i64`'s two words, uniidx_i).
    Ubo(u8),
    /// Component `c` of that element alone, its half of word `c / 2` in
    /// both lanes (a one-lane operation's source, [`coord_lane`]).
    UboLane(u8),
    /// Read `k` of a float array at a run-time index ([`Fs::ubo_idx`]):
    /// `LD_PKA.i16`'s half, one lane ([`Pairs::scalar`]).
    UboHalf(u8),
    /// Word `h` of read `k` of a vec2, vec3 or vec4 array read more than once
    /// ([`Fs::ubo_word`]): the loaded register, a lane pair (or a vec3's z
    /// alone, [`Pairs::scalar`]).
    UboWord(u8, u8),
    /// One 16-bit lane and 0.0 as a word: `MKVEC.v2i16 x, #0`, which
    /// bi_lower_mkvec_swz makes `U16_TO_U32 x` (case108_uniidx's red and
    /// green): node `x` a lane alone ([`Pairs::scalar`]).
    Zext(u8),
    /// Node `x`, a lane alone ([`lane_tree`]), and a constant half other
    /// than 0.0 as a word: `MKVEC.v2i16 x.h0, <half>.h0` (texmore_2's
    /// `MKVEC.v2i16 r1, r1^.h0, u3.w0.h0`, the 1.0 of the alpha).
    Lane {
        x: u8,
        half: u16,
    },
    /// Texel `c` of look-up `j`'s fp16 result alone: its half of word
    /// `c / 2` in both lanes (a one-lane operation's source, `%35^.h00` in
    /// texmore_2.stderr).
    TexLane(u8, u8),
    /// gl_FragCoord's x and y narrowed to fp16, in order, as one word
    /// (`MKVEC.v2i16 x.h0, y.h0`; [`Fs::frag_coord16`], [`frag16`]).
    Frag,
    /// Lane `c` (0 x, 1 y) of the narrowed position alone, its own
    /// register (`FADD.f32 r3.h0, r3^, 0x0.neg`; [`frag16_lanes`]): a
    /// derivative's operand, or the scaled operand's (fsarith_138_deriv).
    FragLane(u8),
    /// Nodes `x` and `y`, each a lane alone, as a word: `MKVEC.v2i16
    /// x.h0, y.h0` (cflow_74_ifoutput's `MKVEC.v2i16 r0, r1^.h0, r0^.h0`
    /// of the red and green selects of constants).
    Pack {
        x: u8,
        y: u8,
    },
    /// Lanes `2h` and `2h + 1` of load `k` as the load's fp32 words, not
    /// converted: the operand of a two-lane `exp2` ([`exp2_rows`], whose
    /// two results one `V2F32_TO_V2F16` narrows; gm2_fsmisc_5).
    Raw {
        k: u8,
        h: u8,
    },
    /// Half `h` of node `x`, an operation of two lanes, read by an operation
    /// of one ([`lane_tree`]; gm2_fsarith_1's `r2.h11` of the matrix
    /// product's x and y pair in the dot product).
    Part {
        x: u8,
        h: u8,
    },
    /// Words `comp` and `comp + 1` of the fp32 uniform load at word `base`
    /// (a float uniform Mesa keeps fp32 in a mediump shader,
    /// [`super::FsPrecision::fp32_uniforms`]), narrowed together:
    /// fshighp_color's `V2F32_TO_V2F16 r0, u2.w0, u2.w1`.
    Wide {
        base: u16,
        comp: u8,
    },
    /// Word `comp` of that load alone, narrowed into a low half (a lane
    /// of one-lane arithmetic): vsstruct_partial's `FADD.f32 r1.h0, u2.w0,
    /// 0x0.neg` of the struct member `val.b`.
    WideLane {
        base: u16,
        comp: u8,
    },
}

/// The pair nodes: an operation of two lanes, or a leaf.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum P {
    Leaf(Leaf),
    Op(Alu, [u8; 3]),
}

/// The pair graph of the colour, beside the walk's nodes.
struct Pairs {
    nodes: [P; MAX_PAIRS],
    place: [(u32, u8); MAX_PAIRS],
    /// A node of one lane, which `nir_opt_vectorize` paired with nothing
    /// (a lane and a constant): its operation reads each source's low half
    /// (`.h00`, `bi_emit_alu` of a one-component 16-bit op).
    scalar: [bool; MAX_PAIRS],
    n: usize,
    /// The pairs of vector operations' lanes [`lane_tree`] made (`nir_opt_cse`
    /// of the pair): (low lane, high lane, node).
    cache: [(Val, Val, u8); 16],
    ncache: usize,
    /// The colour lane the next [`lane_tree`] call stands for (taken by
    /// it): the lane of a vector operation it is ([`Fs::pair_lane`]).
    hint: Option<usize>,
    /// Per node, the pair of scalar operations `nir_opt_vectorize` makes it
    /// one of, (low, high) ([`mates`]).
    mate: [Option<(u16, u16)>; MAX_NODES],
}

/// The pair graph's nodes ([`MAX_PAIR_NODES`]).
pub const MAX_PAIRS: usize = MAX_PAIR_NODES;

impl Pairs {
    fn push(&mut self, p: P, place: (u32, u8)) -> Result<u8, MaliError> {
        if self.n >= MAX_PAIRS {
            return Err(MaliError::FragmentTooLarge);
        }
        self.nodes[self.n] = p;
        self.place[self.n] = place;
        self.scalar[self.n] = false;
        self.n += 1;
        Ok((self.n - 1) as u8)
    }

    fn form(&self, x: u8) -> Form {
        match self.nodes[x as usize] {
            P::Leaf(Leaf::Half { swz: Swz::H10, .. }) => Form::HalfSwapped,
            P::Leaf(Leaf::Half { .. }) => Form::Half,
            P::Leaf(Leaf::Splat { .. }) => Form::Splat,
            P::Leaf(Leaf::Mk { .. } | Leaf::Lanes { .. }) => Form::Mk,
            P::Leaf(Leaf::Uniform { .. }) => Form::Uniform,
            P::Leaf(Leaf::Tex(..) | Leaf::TexLane(..) | Leaf::TexSwapped(..)) => Form::Tex,
            P::Leaf(Leaf::Ubo(_) | Leaf::UboLane(_) | Leaf::UboHalf(_) | Leaf::UboWord(..)) => Form::Ubo,
            /* `MKVEC.v2i16 x, #0`: a lane and a constant half */
            P::Leaf(Leaf::Zext(_) | Leaf::Lane { .. } | Leaf::Pack { .. }) => Form::Mk,
            /* `MKVEC.v2i16 x, y` of the position's two halves, or one of
             * them alone */
            P::Leaf(Leaf::Frag | Leaf::FragLane(_)) => Form::Mk,
            /* read by its `exp2` alone, whose sources are not noted */
            P::Leaf(Leaf::Raw { .. }) => Form::Half,
            P::Leaf(Leaf::Part { .. }) => Form::Half,
            /* a conversion's value, as a varying pair's */
            P::Leaf(Leaf::Wide { .. }) => Form::Half,
            P::Leaf(Leaf::WideLane { .. }) => Form::Splat,
            P::Leaf(Leaf::Const(_)) => Form::Const,
            P::Op(Alu::Fneg, _) => Form::Neg,
            P::Op(Alu::Fabs, _) => Form::Abs,
            P::Op(..) => Form::Op,
        }
    }

    /// The productions of the tree under `x` (`at`: where it stands).
    fn note(&self, x: u8, at: Place, prods: &mut u128) {
        *prods |= prod(self.form(x), at);
        /* a one-lane tree's own forms are [`lone_tree`]'s, checked there
         * (one read by a two-lane operation: [`vectorise`]) */
        if self.scalar[x as usize] {
            return;
        }
        if let P::Op(op, s) = self.nodes[x as usize] {
            let places: &[Place] = match op {
                Alu::Fadd => &[Place::AddA, Place::AddB],
                Alu::Fmul => &[Place::MulA, Place::MulB],
                Alu::Ffma => &[Place::FmaA, Place::FmaB, Place::FmaC],
                Alu::Fneg => &[Place::NegSrc],
                /* a select's sources are the forms [`vectorise`] admits */
                Alu::Bcsel | Alu::Fcmp(_) => &[],
                /* read at fp32 by its expansion ([`transc_rows`]) */
                Alu::Fsin | Alu::Fcos | Alu::Flog2 | Alu::Fexp2 | Alu::Fpow => &[],
                /* a derivative's operand and the narrowing of the y-flip's
                 * lane take the forms [`deriv_operand`] builds, no other */
                Alu::Fddx | Alu::Fddy | Alu::F2f16 => &[],
                _ => &[Place::AbsSrc],
            };
            for (k, p) in places.iter().enumerate() {
                self.note(s[k], *p, prods);
            }
        }
    }
}

/// `v` is a 16-bit select of one lane on its own fp16 compare whose sides
/// are computed (the form [`lane_tree`]'s `Bcsel` arm takes), all of it
/// computed from the narrowed position ([`reaches_frag16`]): not a select
/// of constants (cflow_74_ifoutput's, [`select_lane`]). Another select
/// beside a constant in the colour has no reference here.
fn lone_select(f: &Fs<'_>, v: Val) -> bool {
    let Val::Lane(n, _) = v else { return false };
    let nd = f.nir.nodes[n as usize];
    match f.alu_of(v) {
        Some((Alu::Bcsel, [m, t, e])) => {
            nd.bits == 16
                && f.mask16(m)
                && matches!(f.alu_of(m), Some((Alu::Fcmp(_), _)))
                && !(matches!(t, Val::Const(_)) && matches!(e, Val::Const(_)))
                && reaches_frag16(f, v, 0)
        }
        _ => false,
    }
}

/// `v` is the narrowed position's lane ([`Fs::frag16_lane`]) or a 16-bit
/// operation reading one, through any depth.
fn reaches_frag16(f: &Fs<'_>, v: Val, depth: u8) -> bool {
    if f.frag16.is_none() {
        return false;
    }
    if f.frag16_lane(v).is_some() {
        return true;
    }
    let Val::Lane(n, _) = v else { return false };
    let nd = f.nir.nodes[n as usize];
    let Kind::Alu(op) = nd.kind else { return false };
    nd.bits == 16 && depth < 32 && nd.src[..op.arity()].iter().any(|&s| reaches_frag16(f, s, depth + 1))
}

/// `nir_opt_vectorize` of lanes `a` and `b` (one pair's low and high
/// lane): the two trees match operation for operation, their leaves pair
/// up as [`Leaf`] names them; the late fusion where an fadd stands.
fn vectorise(f: &Fs<'_>, pr: &mut Pairs, a: Val, b: Val, pair: u8, ubo: &UboLayout) -> Result<u8, MaliError> {
    /* a mediump tree's constant is rounded to fp16 ([`f16_rtne`]) */
    let f16 = |bits: u32| f16_rtne(bits).ok_or(MaliError::Fp16Inexact);
    let place = (f.key(a), pair);
    let p = match (f.alu_of(a), f.alu_of(b)) {
        /* a float array's indexed read, or a sum of them, and 0.0:
         * nir_opt_vectorize pairs nothing with the constant, so the sum
         * stays one lane (`FADD.v2f16 a.h00, b.h00`) and the word is
         * `vec2(x, 0.0)` (case108_uniidx, uniarray_b) */
        (Some((Alu::Fadd, _)) | None, _) if b == Val::Const(0) && sum_of_reads(f, a) => {
            P::Leaf(Leaf::Zext(scalar(f, pr, a, pair)?))
        }
        /* the same of a vec3 array's z and another constant: the constant's
         * half packed (deqp_uniarray_dyn_vec3_fs: `FADD.v2f16 r2, r5^.h00,
         * r3^.h00` per read, then `MKVEC.v2i16 r1, r1^.h0, u4.w0.h0`) */
        (Some((Alu::Fadd, _)) | None, _) if matches!(b, Val::Const(_)) && f.ubo_word_tree(a) => {
            let Val::Const(c) = b else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let x = scalar(f, pr, a, pair)?;
            P::Leaf(Leaf::Lane { x, half: f16(c)? })
        }
        /* a vec2, vec3 or vec4 array element's word read at a run-time
         * index more than once ([`Leaf::UboWord`]) */
        _ if f.ubo_word(a, Some(b)).is_some() => {
            let (k, h) = f.ubo_word(a, Some(b)).ok_or(MaliError::Defect(Defect::IrIndex))?;
            P::Leaf(Leaf::UboWord(k, h))
        }
        /* a lane computed by `sin`, `cos` or `log2` (or read by a product
         * of one) and another computed lane: each lane alone, the word
         * packed (fsarith_77_transc: `(sin, cos)` and `(log2 * 0.25, exp2)`
         * as `MKVEC.v2i16 r0, r0^.h0, r1^.h0` after ATEST). The same of a
         * derivative: Mesa pairs `dFdx` with `dFdy` no more than two
         * distinct definitions (`alu_srcs_equal`), fsarith_138_deriv's two
         * scalar `FMA.v2f16` then `MKVEC.v2i16 r0, r0^.h0, r2^.h0` */
        /* `exp2` of a varying's two lanes: at fp32 on the load's words of
         * one aligned pair, in order, the two results narrowed together
         * (gm2_fsmisc_5: `V2F32_TO_V2F16 r0, r0^, r1^` of x and y's `FEXP`);
         * another pairing of the lanes has no reference */
        /* two words of an fp32 uniform in a mediump shader, each lane's
         * `f2fmp` paired into one (fshighp_color: `V2F32_TO_V2F16 r0,
         * u2.w0, u2.w1`); a lane alone or another pairing has no reference */
        _ if f.wide_uniform(a).is_some() || f.wide_uniform(b).is_some() => {
            match (f.wide_uniform(a), f.wide_uniform(b)) {
                (Some(d), Some(e)) => {
                    let w = ubo.word(d).ok_or(MaliError::FsUniform)?;
                    if ubo.word(e) != Some(w + 1) || ubo.half(d).is_some() {
                        return Err(MaliError::FsUniform);
                    }
                    let (base, _) = ubo.load_of(w).ok_or(MaliError::FsUniform)?;
                    P::Leaf(Leaf::Wide { base, comp: (w - base) as u8 })
                }
                _ => return Err(MaliError::FsUniform),
            }
        }
        (Some((Alu::Fexp2, [x, ..])), Some((Alu::Fexp2, [y, ..])))
            if f.input_lane(x).is_some() || f.input_lane(y).is_some() =>
        {
            let (Some((v, i)), Some((w, j))) = (f.input_lane(x), f.input_lane(y)) else {
                return Err(MaliError::FsOperandForm);
            };
            let ((k, i), (e, j)) = (index(f, v, i)?, index(f, w, j)?);
            if k != e || i % 2 != 0 || j != i + 1 {
                return Err(MaliError::FsOperandForm);
            }
            let raw = pr.push(P::Leaf(Leaf::Raw { k, h: i / 2 }), place)?;
            P::Op(Alu::Fexp2, [raw, 0, 0])
        }
        _ if a != b
            && f.alu_of(a).is_some()
            && f.alu_of(b).is_some()
            && !f.same_op(a, b)
            && (has_lane_op(f, a, 0) || has_lane_op(f, b, 0)) =>
        {
            let x = lane_tree(f, pr, a, pair, ubo)?;
            let y = lane_tree(f, pr, b, pair, ubo)?;
            P::Leaf(Leaf::Pack { x, y })
        }
        /* a scalar of [`vec_tree`]'s forms in both lanes (`normalize`'s
         * `frsq` splat over the pair, fsvec_normalize_mediump's `FMA.v2f16
         * r0, r0^, r2.h00, 0x0.neg`; `reflect`'s `-2.0 * d`,
         * fsvec_reflect_mediump's `FMA.v2f16 r0, r2.neg.h00, u2.w0, r0^`):
         * the one-lane tree itself, read through its low half by the pair's
         * operation ([`op`]) */
        /* the same of a scalar operation's value (`0.8 * d` under
         * gm2_fsarith_1's `vec3(0.1) + vec3(0.8 * d)`); a vector operation's
         * lanes CSE'd into one are paired as an operation of two lanes, by
         * the arms below ([`Fs::wide`]) */
        _ if a == b && f.alu_of(a).is_some() && (vec_tree(f, a) || !f.wide(a)) => {
            return lane_tree(f, pr, a, pair, ubo);
        }
        /* a computed lane then a constant (dEQP's operator colour, `vec4(res,
         * 0.0, 0.0, 1.0) * scale + bias`: the pair a vec2 of the lane and the
         * constant). `vec2(fneg@16(x), b)` is `fmul(vec2(x, b), vec2(-1.0,
         * 1.0))` (nir_opt_algebraic.py:3871, a late rule;
         * deqp_constop_op_minus: `U16_TO_U32 r0, r0^.h0` of the converted
         * lane, then `FMA.v2f16 r0, r0^, u2.w0, 0x0.neg` by 0x3C00BC00);
         * another lane alone and the constant's half packed, `U16_TO_U32`
         * of 0.0 (deqp_constop_op_abs: `FADD.v2f16 r0, r0^.abs.h00, 0x0.neg`
         * then `U16_TO_U32 r0, r0^.h0`) */
        /* also a select of a lane's own fp16 compare ([`lane_tree`]'s
         * `Bcsel` arm; gm2_fcoord16_5's `vec4(d, d, d, 1.0)` of `d = d > 0.5
         * ? d - 0.5 : d + 0.25`: `MUX.v2i16 r0, r1^.h11, r1^.h00, r0^.h00`,
         * then `MKVEC.v2i16 r2, r0^.h0, u4.w0.h0`) */
        (Some((op, _)), None)
            if matches!(b, Val::Const(_))
                && (op != Alu::Bcsel || lone_select(f, a))
                && !is_transc(op)
                && !is_deriv(op) =>
        {
            let Val::Const(c) = b else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let half = f16(c)?;
            match f.alu_of(a) {
                Some((Alu::Fneg, [x, ..])) => {
                    let v = match (f.alu_of(x), f.input_lane(x)) {
                        (None, Some((loc, i))) if half == 0 => {
                            let (k, i) = index(f, loc, i)?;
                            let s = lone_node(pr, P::Leaf(Leaf::Splat { k, c: i }), place)?;
                            pr.push(P::Leaf(Leaf::Zext(s)), place)?
                        }
                        _ => vectorise(f, pr, x, b, pair, ubo)?,
                    };
                    let k = pr.push(P::Leaf(Leaf::Const(0x3c00_bc00)), place)?;
                    P::Op(Alu::Fmul, [v, k, 0])
                }
                _ => {
                    let x = lane_tree(f, pr, a, pair, ubo)?;
                    if half == 0 {
                        P::Leaf(Leaf::Zext(x))
                    } else {
                        P::Leaf(Leaf::Lane { x, half })
                    }
                }
            }
        }
        /* a computed lane then a varying's lane (gm2_fsmisc_2's `vec4(d,
         * x.yzw)`): the lane alone and the varying's half of its pair
         * converted at the load ([`pair_read`]), packed, `MKVEC.v2i16 r0,
         * r1^.h0, r0^.h1`; alone, its own conversion's low half */
        _ if mixed_word(f, a, b).is_some() => {
            let (_, (loc, c)) = mixed_word(f, a, b).ok_or(MaliError::Defect(Defect::IrIndex))?;
            let x = lane_tree(f, pr, a, pair, ubo)?;
            let (k, c) = index(f, loc, c)?;
            let leaf = if pair_read(f, k, c) {
                Leaf::Half { k, h: c / 2, swz: if c % 2 == 0 { Swz::H00 } else { Swz::H11 } }
            } else {
                Leaf::Splat { k, c }
            };
            let y = lone_node(pr, P::Leaf(leaf), place)?;
            P::Leaf(Leaf::Pack { x, y })
        }
        /* a computed lane then an fp16 texel alone (terrain d697's `vec4(v,
         * v, v, t.w)`, gm2_texswz_6): the lane alone and the texel's half of
         * the look-up's word, packed, `MKVEC.v2i16 r2, r2^.h0, r1^.h1` */
        _ if f.alu_of(a).is_some() && f.texel(b).is_some_and(|(j, _)| f.tex_f16(j)) => {
            let x = lane_tree(f, pr, a, pair, ubo)?;
            let y = lane_tree(f, pr, b, pair, ubo)?;
            P::Leaf(Leaf::Pack { x, y })
        }
        /* `sqrt` and `inversesqrt` of two lanes: `FRSQ.f16` and `FRCP.f16`
         * are of one lane (`bi_emit_alu` of a 16-bit op with no `.v2f16`
         * form), so each lane alone and the word packed (gm2_fsmisc_1:
         * `FRSQ.f16 r2, r0.h00`, `FRSQ.f16 r0, r0^.h11`, .., then
         * `MKVEC.v2i16 r0, r2^.h0, r0^.h0`) */
        (Some((Alu::Frsq | Alu::Frcp, _)), Some((Alu::Frsq | Alu::Frcp, _))) if a != b => {
            let x = lane_tree(f, pr, a, pair, ubo)?;
            let y = lane_tree(f, pr, b, pair, ubo)?;
            P::Leaf(Leaf::Pack { x, y })
        }
        /* a flattened `if`'s selects ([`walk_if16`]) of one compare: of a
         * uniform's aligned pairs, one `bcsel_pan` of two lanes
         * (cflow_68_armselect: `MUX.v2i16 r1, u2.w0, u2.w1, r0.h00`, the
         * else side first); of constants, each lane alone and the word
         * packed (cflow_74_ifoutput: `MUX.v2i16 r1, 0x0, 0x3C000000.h11,
         * r0.h00`, then `MKVEC.v2i16`). Another operand has no reference */
        (Some((Alu::Bcsel, sa)), Some((Alu::Bcsel, sb))) if sa[0] == sb[0] => {
            let k = |v: Val| matches!(v, Val::Const(_));
            if [sa[1], sa[2], sb[1], sb[2]].iter().all(|&v| k(v)) {
                let x = select_lane(f, pr, a, pair, ubo)?;
                let y = select_lane(f, pr, b, pair, ubo)?;
                P::Leaf(Leaf::Pack { x, y })
            } else {
                let c = cmp_node(f, pr, sa[0], ubo)?;
                let t = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
                let e = vectorise(f, pr, sa[2], sb[2], pair, ubo)?;
                if [t, e].iter().any(|&x| pr.form(x) != Form::Uniform) {
                    return Err(MaliError::FsOperandForm);
                }
                P::Op(Alu::Bcsel, [c, t, e])
            }
        }
        /* selects on each lane's own compare (gm2_fsmisc_8's `step(0.5, x)
         * * y`): the two compares one of two lanes, its mask read whole by
         * the select, `FCMP_OR.v2f16.ge.m1 r0, r0^, 0x38000000.h11, 0x0`
         * then `MUX.v2i16 r0, 0x0, r2^, r0^`; the compares' operands and the
         * sides are pairs ([`vectorise`] of each) */
        (Some((Alu::Bcsel, sa)), Some((Alu::Bcsel, sb))) => {
            let (Some((c @ Alu::Fcmp(_), ca)), Some((d, cb))) = (f.alu_of(sa[0]), f.alu_of(sb[0])) else {
                return Err(MaliError::FsOperandForm);
            };
            if c != d {
                return Err(MaliError::FsOperandForm);
            }
            let x = vectorise(f, pr, ca[0], cb[0], pair, ubo)?;
            let y = vectorise(f, pr, ca[1], cb[1], pair, ubo)?;
            let mask = pr.push(P::Op(c, [x, y, 0]), place)?;
            let t = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
            let e = vectorise(f, pr, sa[2], sb[2], pair, ubo)?;
            P::Op(Alu::Bcsel, [mask, t, e])
        }
        (Some((Alu::Fadd, sa)), Some((Alu::Fadd, sb))) => {
            let x = vectorise(f, pr, sa[0], sb[0], pair, ubo)?;
            let y = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
            /* the late fusion: an fadd of a product is an ffma, the fadd's
             * first source matched first; then an fadd of a negated product
             * is the ffma of the negated first factor */
            let neg_mul = |n: u8| match pr.nodes[n as usize] {
                P::Op(Alu::Fneg, [m, ..]) => match pr.nodes[m as usize] {
                    P::Op(Alu::Fmul, [m0, m1, _]) => Some((m0, m1)),
                    _ => None,
                },
                _ => None,
            };
            /* a scalar's product fused into the pair's sum is the vector
             * `ffma` NIR fuses (`fadd(c, fmul(a, b).xx)`): its constant a
             * pair of the constant (gm2_fsarith_1's `vec3(0.1) + vec3(0.8
             * * d)`, `FMA.v2f16 r1, u8.w0, r1^.h00, 0x2E660000.h11`) */
            let wide = |pr: &mut Pairs, m: u8, n: u8| -> Result<u8, MaliError> {
                match pr.nodes[m as usize] {
                    P::Leaf(Leaf::Const(c)) if pr.scalar[m as usize] && pr.scalar[n as usize] => {
                        pr.push(P::Leaf(Leaf::Const(c & 0xffff | c << 16)), place)
                    }
                    _ => Ok(m),
                }
            };
            let once = |k: usize| f.used_once(sa[k]) && f.used_once(sb[k]);
            match (pr.nodes[x as usize], pr.nodes[y as usize]) {
                (P::Op(Alu::Fmul, [m0, m1, _]), _) if once(0) => {
                    P::Op(Alu::Ffma, [wide(pr, m0, x)?, wide(pr, m1, x)?, y])
                }
                (_, P::Op(Alu::Fmul, [m0, m1, _])) if once(1) => {
                    P::Op(Alu::Ffma, [wide(pr, m0, y)?, wide(pr, m1, y)?, x])
                }
                _ => match (neg_mul(x), neg_mul(y)) {
                    (Some((m0, m1)), _) => P::Op(Alu::Ffma, [pr.push(P::Op(Alu::Fneg, [m0, 0, 0]), place)?, m1, y]),
                    (_, Some((m0, m1))) => P::Op(Alu::Ffma, [pr.push(P::Op(Alu::Fneg, [m0, 0, 0]), place)?, m1, x]),
                    _ => P::Op(Alu::Fadd, [x, y, 0]),
                },
            }
        }
        (Some((Alu::Fmul, sa)), Some((Alu::Fmul, sb))) => {
            let x = vectorise(f, pr, sa[0], sb[0], pair, ubo)?;
            let y = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
            P::Op(Alu::Fmul, [x, y, 0])
        }
        /* `max` / `min` of a pair and a constant pair (gm2_fsmisc_3); the
         * bound is the walk's constant ([`walk_nodes`]) */
        (Some((op @ (Alu::Fmax | Alu::Fmin), sa)), Some((o2, sb))) if o2 == op => {
            let x = vectorise(f, pr, sa[0], sb[0], pair, ubo)?;
            let y = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
            if pr.form(y) != Form::Const {
                return Err(MaliError::FsOperandForm);
            }
            P::Op(op, [x, y, 0])
        }
        (Some((op @ Alu::Fclamp(clamp::ZERO_ONE), sa)), Some((o2, sb))) if o2 == op => {
            P::Op(op, [vectorise(f, pr, sa[0], sb[0], pair, ubo)?, 0, 0])
        }
        /* the front end's `ffma` (`mix`): its three sources in order */
        (Some((Alu::Ffma, sa)), Some((Alu::Ffma, sb))) => {
            let x = vectorise(f, pr, sa[0], sb[0], pair, ubo)?;
            let y = vectorise(f, pr, sa[1], sb[1], pair, ubo)?;
            let z = vectorise(f, pr, sa[2], sb[2], pair, ubo)?;
            P::Op(Alu::Ffma, [x, y, z])
        }
        (Some((op @ (Alu::Fneg | Alu::Fabs), sa)), Some((o2, sb))) if o2 == op => {
            P::Op(op, [vectorise(f, pr, sa[0], sb[0], pair, ubo)?, 0, 0])
        }
        /* the narrowed position: x and y in order (prec_84's `vec4(x / 64.0,
         * y / 64.0, ..)`); any other pairing of its lanes has no reference */
        _ if f.frag16_lane(a).is_some() || f.frag16_lane(b).is_some() => match (f.frag16_lane(a), f.frag16_lane(b)) {
            (Some(0), Some(1)) => P::Leaf(Leaf::Frag),
            _ => return Err(MaliError::FragCoordMediump),
        },
        /* a look-up's result in its own pairs, in order, or swapped (an
         * output word only, [`select`]) */
        _ if f.texel(a).is_some() || f.texel(b).is_some() => match (f.texel(a), f.texel(b)) {
            (Some((t, i)), Some((u, j))) if t == u && i % 2 == 0 && j == i + 1 => P::Leaf(Leaf::Tex(t as u8, i / 2)),
            (Some((t, i)), Some((u, j))) if t == u && j % 2 == 0 && i == j + 1 && f.tex_f16(t) => {
                P::Leaf(Leaf::TexSwapped(t as u8, j / 2))
            }
            _ => return Err(MaliError::TexLanes),
        },
        /* the indexed element's words in their own pairs, in order (a lane
         * alone or swapped has no reference) */
        _ if f.ubo_lane(a).is_some() || f.ubo_lane(b).is_some() => match (f.ubo_lane(a), f.ubo_lane(b)) {
            (Some(i), Some(j)) if i % 2 == 0 && j == i + 1 => P::Leaf(Leaf::Ubo(i / 2)),
            _ => return Err(MaliError::FsUniform),
        },
        _ => match (f.input_lane(a), f.input_lane(b), a, b) {
            (Some((v, i)), Some((w, j)), ..) if index(f, v, i)?.0 == index(f, w, j)?.0 => {
                let ((k, i), (_, j)) = (index(f, v, i)?, index(f, w, j)?);
                let comps = f.varyings[k as usize].comps;
                if i % 2 == 0 && j == i + 1 {
                    P::Leaf(Leaf::Half { k, h: i / 2, swz: Swz::H01 })
                } else if j % 2 == 0 && i == j + 1 {
                    P::Leaf(Leaf::Half { k, h: j / 2, swz: Swz::H10 })
                } else if i == j && comps > 1 && pair_read(f, k, i) {
                    /* one lane in both of a pair the trees convert whole:
                     * its half (gm2_fsarith_1's `n.y` of `nm * vec4(n,
                     * 1.0)`, `FMA.v2f16 r2, u2.w0, r0.h11, u2.w1`) */
                    P::Leaf(Leaf::Half { k, h: i / 2, swz: if i % 2 == 0 { Swz::H00 } else { Swz::H11 } })
                } else if i == j {
                    /* converted alone (`n.z`: `FADD.f32 r1.h0, r2^, 0x0.neg`,
                     * then `.. u7.w0, r1.h00, r2^`) */
                    P::Leaf(Leaf::Splat { k, c: i })
                } else if i / 2 != j / 2 && f.varyings[k as usize].whole {
                    /* two lanes of a load the link keeps whole (78's packed
                     * matrix); a swizzle of a compacted load has no
                     * reference */
                    P::Leaf(Leaf::Lanes { k, c: i, e: k, d: j })
                } else {
                    return Err(MaliError::ColourForm);
                }
            }
            /* the same of two loads the link keeps whole: 78's fragment
             * shader alone, where `v_after.z` is read too and the packed
             * matrix's lanes fall in two slots (its shader object's
             * record; the program's is compiled with the vertex shader's
             * constant lane, one load) */
            (Some((v, i)), Some((w, j)), ..)
                if f.varyings[index(f, v, i)?.0 as usize].whole && f.varyings[index(f, w, j)?.0 as usize].whole =>
            {
                let ((k, i), (e, j)) = (index(f, v, i)?, index(f, w, j)?);
                P::Leaf(Leaf::Lanes { k, c: i, e, d: j })
            }
            (Some((v, i)), None, _, Val::Const(c)) => {
                let half = f16(c)?;
                /* `MKVEC.v2i16 x, #0` is `U16_TO_U32 x` (bi_lower_mkvec_swz.c:256-257,
                 * opt.rs `lower_mkvec_swz`), drawn as a word of the colour
                 * (deqp_colourpair_default_attrib's `U16_TO_U32 r0, r0.h0`);
                 * as an operation's operand it has no reference */
                let word = |c: [Val; 4]| (a, b) == (c[0], c[1]) || (a, b) == (c[2], c[3]);
                if half == 0 && !f.colour.is_some_and(word) {
                    return Err(MaliError::ColourZeroHalf);
                }
                let (k, c) = index(f, v, i)?;
                P::Leaf(Leaf::Mk { k, c, half, first: false })
            }
            (None, None, Val::Const(x), Val::Const(y)) => P::Leaf(Leaf::Const(f16(x)? as u32 | (f16(y)? as u32) << 16)),
            /* a word of the fp16 uniform block (Mesa's fp16 block, two
             * components a word, sel_u_color): a uniform's own aligned pair
             * of components, or one component in both lanes (a float
             * uniform's `.h00`, vtl_fs's `u_pick`); any other read of it has
             * no reference */
            _ => match (f.uniform_dword(a), f.uniform_dword(b), a, b) {
                (Some(d), Some(e), ..) => {
                    let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
                    let swz = if d == e {
                        if half == 0 {
                            Swz::H00
                        } else {
                            Swz::H11
                        }
                    } else if half == 0 && ubo.half(e) == Some((w, 1, base)) {
                        Swz::H01
                    } else {
                        return Err(MaliError::FsUniform);
                    };
                    P::Leaf(Leaf::Uniform { base, comp: (w - base) as u8, swz })
                }
                (Some(_), ..) | (_, Some(_), ..) => return Err(MaliError::FsUniform),
                /* a constant, then a load's lane: the constant is the low
                 * half (deqp_colourpair_ck_lane_highpcolour), as a word of
                 * the colour; as an operation's operand it has no form here
                 * ([`conversion_order`]) */
                (None, None, Val::Const(c), _) if f.input_lane(b).is_some() => {
                    let word = |c: [Val; 4]| (a, b) == (c[0], c[1]) || (a, b) == (c[2], c[3]);
                    if !f.colour.is_some_and(word) {
                        return Err(MaliError::ColourPair);
                    }
                    let half = f16(c)?;
                    let Some((v, i)) = f.input_lane(b) else { return Err(MaliError::Defect(Defect::IrIndex)) };
                    let (k, c) = index(f, v, i)?;
                    P::Leaf(Leaf::Mk { k, c, half, first: true })
                }
                _ => return Err(MaliError::ColourForm),
            },
        },
    };
    pr.push(p, place)
}

/// Where the loads' fp16 conversions stand, in Mesa's order
/// ([`Conversions`]): an aligned pair that [`Leaf::Half`] reads is one
/// conversion (`V2F32_TO_V2F16`, the even lane low), and so is an aligned
/// pair of the colour's own lanes that no tree reads, whatever leaves read
/// them (deqp_mixprec_rot3's `(w, x, y, 1.0)`: w alone, then (x, y);
/// fsload_mk_pair's (z, w) beside constants); every other lane is alone
/// (`FADD.f32 .h0`). Mesa converts the lanes its fp16 trees read
/// where the varying's `f2fmp` stands, in lane order (`nir_lower_alu_width`
/// splits the vector conversion): at the load here. The colour's own lanes
/// lose their conversion at the link (`f2f32(f2fmp(a))` is `a`,
/// nir_opt_algebraic.py:1991, the colour being stored at fp32 then), and
/// `nir_lower_mediump_io` converts the stored colour right before its
/// store (bifrost_nir.c:914, nir_lower_mediump.c:102-104): after
/// everything else, a discard included, in the colour's lane order across
/// the loads (fsconv_cpq, fsconv_discw). `nir_opt_vectorize` makes the two
/// lanes of an aligned pair of one load one conversion at the first one's
/// place, that lane low (nir_opt_vectorize.c:166-171 and 559;
/// intloop_ifloop.expect.json's `(v.z, v.w, v.x, 1.0)`: the pair z,w
/// first, then x). A pair Mesa makes of a tree's lane and a colour lane
/// (fsload_disc_col: the compare's y, then the colour's x), or with its odd
/// lane low (swap_yx's `V2F32_TO_V2F16 r0, r1^,
/// r0^`), or a load whose fp16 trees read lanes of two varyings (each its
/// own `f2fmp`, at its own place), has no reference here.
fn conversion_order(
    f: &Fs<'_>,
    pr: &Pairs,
    roots: [u8; 2],
    conv_form: &mut [[Option<bool>; 4]; MAX_VARYINGS],
) -> Result<Conversions, MaliError> {
    let mut out = Conversions { at_load: [[NO_LANE; 4]; MAX_VARYINGS], at_store: [(0, NO_LANE); 8], n: 0 };
    let lanes_of = |x: &P| -> [Option<(u8, u8)>; 2] {
        match *x {
            P::Leaf(Leaf::Half { k, h, swz: Swz::H10 }) => [Some((k, 2 * h + 1)), Some((k, 2 * h))],
            P::Leaf(Leaf::Half { k, h, .. }) => [Some((k, 2 * h)), Some((k, 2 * h + 1))],
            P::Leaf(Leaf::Splat { k, c } | Leaf::Mk { k, c, .. }) => [Some((k, c)), None],
            P::Leaf(Leaf::Lanes { k, c, e, d }) => [Some((k, c)), Some((e, d))],
            _ => [None, None],
        }
    };
    /* the lanes the fp16 trees convert: every leaf but the colour's own,
     * and the lanes an operation read that the walk folded ([`Fs::folded`]) */
    let mut tree = [0u8; MAX_VARYINGS];
    for (i, x) in pr.nodes[..pr.n].iter().enumerate() {
        if roots.contains(&(i as u8)) {
            continue;
        }
        /* a constant then a lane as an operation's operand: Mesa computes
         * the lane alone and joins the halves after
         * (deqp_colourpair_rnd_fs_ck_madd, _zk_tree, _ck_product), a form
         * not built here; as a colour word it is the `MKVEC` */
        if matches!(x, P::Leaf(Leaf::Mk { first: true, .. })) {
            return Err(MaliError::ColourPair);
        }
        for (k, c) in lanes_of(x).into_iter().flatten() {
            tree[k as usize] |= 1 << c;
        }
    }
    for (loc, &m) in f.folded.iter().enumerate() {
        for c in (0..4u8).filter(|&c| m & 1 << c != 0) {
            if let Some((slot, lane)) = f.lane_of[loc][c as usize] {
                if let Some(k) = f.varyings[..f.nvar].iter().position(|v| v.location == slot) {
                    tree[k] |= 1 << lane;
                }
            }
        }
    }
    for k in 0..f.nvar {
        let slot = f.varyings[k].location;
        /* the varyings whose lanes the trees convert here */
        let owners = (0..MAX_VARYINGS)
            .filter(|&l| f.lane_of[l].iter().flatten().any(|&(s, lane)| s == slot && tree[k] & 1 << lane != 0));
        if owners.count() > 1 {
            return Err(MaliError::FsVaryingLoad);
        }
        /* Mesa's order of the conversions */
        let mut order = [0u8; 4];
        let mut n = 0;
        let mut push = |c: u8, order: &mut [u8; 4]| {
            if !order[..n].contains(&c) && n < 4 {
                order[n] = c;
                n += 1;
            }
        };
        for c in (0..4).filter(|&c| tree[k] & 1 << c != 0) {
            push(c, &mut order);
        }
        for r in roots {
            for (j, c) in lanes_of(&pr.nodes[r as usize]).into_iter().flatten() {
                if j as usize == k {
                    push(c, &mut order);
                }
            }
        }
        /* its instructions: (first lane, second lane of a pair) */
        let mut mesa = [(0u8, None::<u8>); 4];
        let mut nm = 0;
        for &c in &order[..n] {
            match mesa[..nm].iter_mut().find(|g| g.0 == c ^ 1 && g.1.is_none()) {
                Some(g) => g.1 = Some(c),
                None => {
                    mesa[nm] = (c, None);
                    nm += 1;
                }
            }
        }
        /* [`convert`]'s */
        let mut ours = [(0u8, None::<u8>); 4];
        let mut no = 0;
        let mut c = 0u8;
        while c < 4 {
            match conv_form[k][c as usize] {
                Some(true) => {
                    ours[no] = (c, Some(c + 1));
                    no += 1;
                    c += 2;
                }
                Some(false) => {
                    ours[no] = (c, None);
                    no += 1;
                    c += 1;
                }
                None => c += 1,
            }
        }
        /* an aligned pair of the colour's own lanes, each a lane alone in its
         * leaf, is one conversion, the even lane low, the odd one read as its
         * high half (deqp_mixprec_rot3's `V2F32_TO_V2F16` of (x, y) for
         * `(w, x, y, 1.0)`, then `MKVEC r0.h1, 1.0h`): a pair whose first
         * lane is odd or that a tree reads stays refused below */
        for g in &mesa[..nm] {
            let (c, Some(d)) = (g.0, g.1) else { continue };
            let lone = |c: u8| conv_form[k][c as usize] == Some(false) && tree[k] & 1 << c == 0;
            if c & 1 == 0 && lone(c) && lone(d) {
                conv_form[k][c as usize] = Some(true);
                conv_form[k][d as usize] = Some(true);
                let i = ours[..no].iter().position(|o| o.0 == c).ok_or(MaliError::Defect(Defect::IrIndex))?;
                ours[i] = (c, Some(d));
                let j = ours[..no].iter().position(|o| o.0 == d).ok_or(MaliError::Defect(Defect::IrIndex))?;
                ours.copy_within(j + 1..no, j);
                no -= 1;
            }
        }
        /* the same conversions (a pair Mesa makes with its odd lane low has
         * no reference here); a tree's at the load, in Mesa's order */
        let same = nm == no && mesa[..nm].iter().all(|g| ours[..no].contains(g));
        if !same {
            return Err(MaliError::FsVaryingLoad);
        }
        let at_tree = mesa[..nm].iter().filter(|g| tree[k] & 1 << g.0 != 0);
        for (o, g) in out.at_load[k].iter_mut().zip(at_tree) {
            *o = g.0;
        }
    }
    /* the colour's own conversions, at the store, in the colour's order
     * across the loads (fsconv_cpq: q.y of slot 0, c.z of slot 1, then p.x
     * of slot 0) */
    for r in roots {
        for (k, c) in lanes_of(&pr.nodes[r as usize]).into_iter().flatten() {
            /* the conversion's first lane: a pair's even one (the pairs
             * Mesa makes with the odd lane first are refused above) */
            let form = conv_form.get(k as usize).and_then(|x| x.get(c as usize)).copied().flatten();
            let Some(pair) = form else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let c = if pair { c & !1 } else { c };
            let tree_lane = tree.get(k as usize).is_some_and(|t| t & 1 << c != 0);
            if !tree_lane && !out.at_store[..out.n].contains(&(k, c)) {
                *out.at_store.get_mut(out.n).ok_or(MaliError::Defect(Defect::IrIndex))? = (k, c);
                out.n += 1;
            }
        }
    }
    Ok(out)
}

/// Where the fp16 conversions of the loads stand ([`conversion_order`]):
/// those of the fp16 trees at their load (each load's first lanes, in
/// order, padded with [`NO_LANE`]), the colour's own at its store, in the
/// colour's order ((load, first lane), `n` of them).
struct Conversions {
    at_load: [[u8; 4]; MAX_VARYINGS],
    at_store: [(u8, u8); 8],
    n: usize,
}

/// `op` is `sin`, `cos`, `log2`, `exp2` or `pow` ([`transc_rows`]).
const fn is_transc(op: Alu) -> bool {
    matches!(op, Alu::Fsin | Alu::Fcos | Alu::Flog2 | Alu::Fexp2 | Alu::Fpow)
}

/// `op` is `ddx` or `ddy` ([`Alu::Fddx`]).
const fn is_deriv(op: Alu) -> bool {
    matches!(op, Alu::Fddx | Alu::Fddy)
}

/// Lane `v` is one of [`is_transc`]'s or [`is_deriv`]'s (an operation of
/// one lane, which `nir_opt_vectorize` pairs with nothing here), or sums,
/// products and fnegs with one among them ([`lane_tree`]'s forms).
fn has_lane_op(f: &Fs<'_>, v: Val, depth: u8) -> bool {
    match f.alu_of(v) {
        Some((op, _)) if is_transc(op) || is_deriv(op) => true,
        Some((Alu::Fadd | Alu::Fmul | Alu::Fneg, s)) if depth < 8 => {
            s[..2].iter().any(|&x| x != Val::Undef && has_lane_op(f, x, depth + 1))
        }
        _ => false,
    }
}

/// Lane `v` is a `pow`, or sums, products and `ffma`s with one among them
/// ([`lone_tree`]).
fn has_pow(f: &Fs<'_>, v: Val, depth: u8) -> bool {
    match f.alu_of(v) {
        Some((Alu::Fpow, _)) => true,
        Some((op @ (Alu::Fadd | Alu::Fmul | Alu::Ffma), s)) if depth < 8 => {
            s[..op.arity()].iter().any(|&x| has_pow(f, x, depth + 1))
        }
        _ => false,
    }
}

/// Lane `v` is fadds whose leaves are a float array's indexed reads
/// ([`Fs::ubo_read`]), the form [`scalar`] takes.
fn sum_of_reads(f: &Fs<'_>, v: Val) -> bool {
    match f.alu_of(v) {
        Some((Alu::Fadd, s)) => s[..2].iter().all(|&x| sum_of_reads(f, x)),
        Some(_) => false,
        None => f.ubo_read(v).is_some() || f.ubo_word(v, None).is_some(),
    }
}

/// Lane `v` ([`sum_of_reads`]) as nodes of one lane: each fadd its own
/// node (its sources in order), each read its half.
fn scalar(f: &Fs<'_>, pr: &mut Pairs, v: Val, pair: u8) -> Result<u8, MaliError> {
    let place = (f.key(v), pair);
    let p = match (f.alu_of(v), f.ubo_read(v)) {
        (Some((Alu::Fadd, s)), _) => {
            let x = scalar(f, pr, s[0], pair)?;
            let y = scalar(f, pr, s[1], pair)?;
            P::Op(Alu::Fadd, [x, y, 0])
        }
        (None, Some(k)) => P::Leaf(Leaf::UboHalf(k)),
        (None, None) => match f.ubo_word(v, None) {
            Some((k, h)) => P::Leaf(Leaf::UboWord(k, h)),
            None => return Err(MaliError::ColourForm),
        },
        _ => return Err(MaliError::ColourForm),
    };
    let x = pr.push(p, place)?;
    pr.scalar[x as usize] = true;
    Ok(x)
}

/// An operand of a mediump discard's compare as a leaf of one lane and the
/// half it is read through (`bi_alu_src_index` of a one-lane 16-bit op
/// replicates the lane): a component of an fp16 uniform (`u2.w0.h11`,
/// alu_branchlane), a constant's half, or a varying's lane converted alone
/// (`FADD.f32 r0.h0, r1^, 0x0.neg`, then `DISCARD.f32.lt 0x38000000.h1,
/// r0^.h0`; discard_81_branchquad, whose varying the link keeps whole,
/// [`layout`]). Any other operand has no reference
/// ([`MaliError::DiscardForm`]).
fn cmp_leaf(f: &Fs<'_>, pr: &mut Pairs, v: Val, ubo: &UboLayout) -> Result<(u8, Swz), MaliError> {
    let (leaf, swz) = match (f.alu_of(v), f.input_lane(v), f.uniform_dword(v), v) {
        (_, _, _, Val::Const(c)) => (Leaf::Const(u32::from(f16_rtne(c).ok_or(MaliError::Fp16Inexact)?)), Swz::H00),
        (None, Some((loc, c)), _, _) if (loc as usize) < MAX_VARYINGS => {
            let (k, c) = index(f, loc, c)?;
            (Leaf::Splat { k, c }, Swz::H00)
        }
        (None, None, Some(d), _) => {
            let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
            (Leaf::Uniform { base, comp: (w - base) as u8, swz: Swz::H01 }, if half == 0 { Swz::H00 } else { Swz::H11 })
        }
        _ => return Err(MaliError::DiscardForm),
    };
    let x = pr.push(P::Leaf(leaf), (f.key(v), 0))?;
    pr.scalar[x as usize] = true;
    Ok((x, swz))
}

/// A flattened `if`'s compare ([`walk_if16`]) as a node of one lane, its
/// operands a uniform's half or a constant's (`FCMP_OR.v2f16.lt.m1 r0,
/// 0x38000000.h11, u4.w0.h00, ..`, cflow_68_armselect), made once for all
/// the selects that read it (`nir_opt_cse`). Another operand has no
/// reference ([`MaliError::FsOperandForm`]).
fn cmp_node(f: &Fs<'_>, pr: &mut Pairs, c: Val, ubo: &UboLayout) -> Result<u8, MaliError> {
    let Some((op @ Alu::Fcmp(_), s)) = f.alu_of(c) else { return Err(MaliError::FsOperandForm) };
    let mut leaf = [Leaf::Const(0); 2];
    for (l, v) in leaf.iter_mut().zip(s) {
        *l = match (f.alu_of(v), f.uniform_dword(v), v) {
            (_, _, Val::Const(k)) => Leaf::Const(u32::from(f16_rtne(k).ok_or(MaliError::Fp16Inexact)?)),
            (None, Some(d), _) => {
                let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
                Leaf::Uniform { base, comp: (w - base) as u8, swz: if half == 0 { Swz::H00 } else { Swz::H11 } }
            }
            _ => return Err(MaliError::FsOperandForm),
        };
    }
    let same = |x: u8, l: Leaf| pr.nodes[x as usize] == P::Leaf(l);
    if let Some(i) = (0..pr.n).find(|&i| match pr.nodes[i] {
        P::Op(o, [x, y, _]) => o == op && pr.scalar[i] && same(x, leaf[0]) && same(y, leaf[1]),
        _ => false,
    }) {
        return Ok(i as u8);
    }
    let place = (f.key(c), 0);
    let mut x = [0u8; 2];
    for (n, l) in x.iter_mut().zip(leaf) {
        *n = pr.push(P::Leaf(l), place)?;
        pr.scalar[*n as usize] = true;
    }
    let n = pr.push(P::Op(op, [x[0], x[1], 0]), place)?;
    pr.scalar[n as usize] = true;
    Ok(n)
}

/// Lane `v`, a flattened `if`'s select of two constants ([`vectorise`]),
/// as a node of one lane: `MUX.v2i16 else, then, c.h00`.
fn select_lane(f: &Fs<'_>, pr: &mut Pairs, v: Val, pair: u8, ubo: &UboLayout) -> Result<u8, MaliError> {
    let Some((Alu::Bcsel, [c, t, e])) = f.alu_of(v) else { return Err(MaliError::FsOperandForm) };
    let c = cmp_node(f, pr, c, ubo)?;
    let place = (f.key(v), pair);
    let mut x = [0u8; 2];
    for (n, k) in x.iter_mut().zip([t, e]) {
        let Val::Const(k) = k else { return Err(MaliError::FsOperandForm) };
        *n = pr.push(P::Leaf(Leaf::Const(u32::from(f16_rtne(k).ok_or(MaliError::Fp16Inexact)?))), place)?;
        pr.scalar[*n as usize] = true;
    }
    let n = pr.push(P::Op(Alu::Bcsel, [c, x[0], x[1]]), place)?;
    pr.scalar[n as usize] = true;
    Ok(n)
}

/// Pair `pair` of the colour as a lane alone and a constant (`a`, `b`):
/// `nir_opt_vectorize` pairs nothing with the constant, so lane `a` is
/// nodes of one lane ([`lane_tree`]) and the word `vec2(a, b)`
/// ([`Leaf::Lane`]). None for any other pair (a lane of a load, a
/// uniform, a pair of operations: [`vectorise`]'s), or a constant 0.0
/// ([`Leaf::Zext`]'s form, which no reference draws for this tree).
fn lane_word(f: &Fs<'_>, pr: &mut Pairs, a: Val, b: Val, pair: u8, ubo: &UboLayout) -> Result<Option<u8>, MaliError> {
    let Val::Const(c) = b else { return Ok(None) };
    if f.alu_of(a).is_none() || !lone_tree(f, a) {
        return Ok(None);
    }
    let half = f16_rtne(c).ok_or(MaliError::Fp16Inexact)?;
    /* `vec2(x, 0.0)` is `U16_TO_U32 x` ([`Leaf::Zext`]): drawn for a tree
     * of [`vec_tree`]'s forms (fsvec_length_mediump's `U16_TO_U32 r0,
     * r0^.h0` of the length), for no other */
    if half == 0 && !vec_tree(f, a) && !uniform_sum(f, a) {
        return Err(MaliError::ColourZeroHalf);
    }
    pr.hint = Some(2 * pair as usize);
    let x = lane_tree(f, pr, a, pair, ubo)?;
    let leaf = if half == 0 { Leaf::Zext(x) } else { Leaf::Lane { x, half } };
    pr.push(P::Leaf(leaf), (f.key(a), pair)).map(Some)
}

/// Lane `v` is a sum of uniform components with no constant
/// (deqp_uniarray_float_const_fs's `u_arr[0] + .. + u_arr[3]`, then
/// `U16_TO_U32 r0, r0^.h0` beside the colour's 0.0).
fn uniform_sum(f: &Fs<'_>, v: Val) -> bool {
    match f.alu_of(v) {
        Some((Alu::Fadd, s)) => uniform_sum(f, s[0]) && uniform_sum(f, s[1]),
        Some(_) => false,
        None => f.uniform_dword(v).is_some(),
    }
}

/// Lane `v` is sums and products of fp16 texels and constants, with a
/// texel among them, or sums of a uniform's component, negated or not,
/// and constants, with a uniform among them ([`lane_tree`]'s form;
/// varyconst_75_immneg's `1.0 - u_k.z`), or sums and products of
/// derivatives and constants (fsarith_138_deriv's blue, `0.5 + 0.125 *
/// (dFdx(fc.x) + dFdy(fc.y))`; the derivatives' operands are
/// [`deriv_operand`]'s).
fn lone_tree(f: &Fs<'_>, v: Val) -> bool {
    /* (a texel, a uniform, a derivative) among the leaves */
    fn walk(f: &Fs<'_>, v: Val) -> Option<(bool, bool, bool)> {
        let or = |x: (bool, bool, bool), y: (bool, bool, bool)| (x.0 | y.0, x.1 | y.1, x.2 | y.2);
        match (f.alu_of(v), f.texel(v), v) {
            (Some((op, _)), ..) if is_deriv(op) => Some((false, false, true)),
            (Some((Alu::Fadd | Alu::Fmul, s)), ..) => Some(or(walk(f, s[0])?, walk(f, s[1])?)),
            (Some((Alu::Fneg, s)), ..) => walk(f, s[0]),
            (None, Some((j, _)), _) if f.tex_f16(j) => Some((true, false, false)),
            (None, None, Val::Const(_)) => Some((false, false, false)),
            (None, None, _) if f.uniform_dword(v).is_some() => Some((false, true, false)),
            _ => None,
        }
    }
    /* each with the forms its reference draws: texels under sums and
     * products (texmore_2), a uniform under a sum and an fneg (75) */
    fn uniform_form(f: &Fs<'_>, v: Val) -> bool {
        match f.alu_of(v) {
            Some((Alu::Fadd, s)) => uniform_form(f, s[0]) && uniform_form(f, s[1]),
            Some((Alu::Fneg, s)) => uniform_form(f, s[0]),
            Some(_) => false,
            None => true,
        }
    }
    /* a mediump compare's `b2f16` (deqp_mixprec_cmpf's `vec4(r, r, r, 1.0)`:
     * `MKVEC.v2i16 r2, r0.h0, u3.w1.h0` of the compare's `.f1` result) */
    if f.cmp_select(v).is_some() {
        return true;
    }
    /* its select of a varying's lane and 0.0 ([`lane_tree`]) */
    if let Some((Alu::Bcsel, [m, t, Val::Const(0)])) = f.alu_of(v) {
        let lane = f.input_lane(t).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS);
        if f.mask16(m) && matches!(f.alu_of(m), Some((Alu::Fcmp(_), _))) && lane {
            return true;
        }
    }
    match walk(f, v) {
        Some((true, false, false)) => true,
        Some((false, true, false)) => uniform_form(f, v),
        Some((false, false, true)) => true,
        /* a tree with a `pow` among its operations: gm2_fsarith_1's blue,
         * `0.1 + 0.8 * d + 0.2 * pow(..)` beside the alpha 1.0, `FMA.v2f16
         * r0, u9.w1, r0^.h00, r1^.h00` then `MKVEC.v2i16 r2, r0^.h0,
         * u10.w0.h0`; another of [`is_transc`]'s has no reference */
        _ => vec_tree(f, v) || has_pow(f, v, 0),
    }
}

/// Lane `v` is the forms of a mediump `normalize`, `length`, `reflect` or
/// `fract`: products, `ffma`s, `frsq` and `frcp` of varyings' lanes and
/// uniforms' components, with an `frsq` or `reflect`'s `-2.0 * d` among
/// them (fsvec_normalize_mediump's `v.z * inversesqrt(dot(v, v))`,
/// fsvec_length_mediump's `frcp(frsq(dot(v, v)))`, fsvec_reflect_mediump's
/// `ffma(-2.0 * dot(u, v), u.z, v.z)`), or `lower_ffract`'s `fadd(a,
/// fneg(ffloor(a)))` of a varying's lane (fsvec_fract_mediump). Another
/// leaf (a constant) or operation has no reference here.
fn vec_tree(f: &Fs<'_>, v: Val) -> bool {
    let lane = |x: Val| f.input_lane(x).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS);
    if let Some((Alu::Fadd, [a, n, _])) = f.alu_of(v) {
        if let Some((Alu::Fneg, [fl, ..])) = f.alu_of(n) {
            return lane(a) && f.alu_of(fl) == Some((Alu::Ffloor, [a, Val::Undef, Val::Undef]));
        }
    }
    /* Some(an frsq or a `-2.0 * d` among them) */
    fn walk(f: &Fs<'_>, v: Val, depth: u8) -> Option<bool> {
        if depth > 16 {
            return None;
        }
        match f.alu_of(v) {
            Some((Alu::Frsq, s)) => walk(f, s[0], depth + 1).map(|_| true),
            Some((Alu::Frcp, s)) => walk(f, s[0], depth + 1),
            Some((Alu::Fmul, [d, Val::Const(F32_MINUS_TWO), _])) => walk(f, d, depth + 1).map(|_| true),
            Some((op @ (Alu::Fmul | Alu::Ffma), s)) => {
                let mut r = false;
                for &x in &s[..op.arity()] {
                    r |= walk(f, x, depth + 1)?;
                }
                Some(r)
            }
            Some(_) => None,
            None if f.uniform_dword(v).is_some() => Some(false),
            None => f.input_lane(v).filter(|&(loc, _)| (loc as usize) < MAX_VARYINGS).map(|_| false),
        }
    }
    walk(f, v, 0) == Some(true)
}

/// Lane `c` of load `k` is read by the colour's fp16 operations together
/// with the other lane of its aligned pair: the two are one conversion
/// (`nir_opt_vectorize` pairs the two `f2f16`, `V2F32_TO_V2F16 r0, r0^, r1^`
/// in fsvec_length_mediump), else the lane is converted alone (`FADD.f32
/// r1.h0, r2^, 0x0.neg`).
fn pair_read(f: &Fs<'_>, k: u8, c: u8) -> bool {
    let Some(colour) = f.colour else { return false };
    let mut seen = [false; MAX_NODES];
    let mut stack = [0u16; MAX_NODES];
    let mut n = 0;
    let mut read = 0u8;
    for v in colour {
        if let Val::Lane(x, _) = v {
            if !seen[x as usize] {
                seen[x as usize] = true;
                stack[n] = x;
                n += 1;
            }
        }
    }
    while n > 0 {
        n -= 1;
        let nd = f.nir.nodes[stack[n] as usize];
        let Kind::Alu(op) = nd.kind else { continue };
        if nd.bits != 16 {
            continue;
        }
        for &x in &nd.src[..op.arity()] {
            if let Some((loc, l)) = f.input_lane(x) {
                if let Ok((j, l)) = index(f, loc, l) {
                    if j == k {
                        read |= 1 << l;
                    }
                }
            } else if let Val::Lane(y, _) = x {
                if !seen[y as usize] {
                    seen[y as usize] = true;
                    stack[n] = y;
                    n += 1;
                }
            }
        }
    }
    /* a varying's lane beside a computed lane in an output word is
     * converted at the load too ([`mixed_word`]; gm2_fsmisc_2's `x.y`
     * beside `d`, one `V2F32_TO_V2F16 r0, r0^, r1^` with `x.x`) */
    for w in 0..2 {
        if let Some((_, (loc, l))) = mixed_word(f, colour[2 * w], colour[2 * w + 1]) {
            if let Ok((j, l)) = index(f, loc, l) {
                if j == k {
                    read |= 1 << l;
                }
            }
        }
    }
    read & 1 << c != 0 && read & 1 << (c ^ 1) != 0
}

/// Output word `(a, b)` is a computed lane and a varying's lane, in
/// either order: Some((the computed lane, the varying's (location,
/// lane))), the computed one first in the word when it is `a`.
fn mixed_word(f: &Fs<'_>, a: Val, b: Val) -> Option<(Val, (u8, u8))> {
    let varying = |v: Val| f.input_lane(v).filter(|&(loc, _)| (loc as usize) < MAX_VARYINGS);
    match (f.alu_of(a), varying(b)) {
        (Some(_), Some(l)) => Some((a, l)),
        _ => None,
    }
}

/// Lane `v` of the colour alone ([`lane_word`]) as nodes of one lane: the
/// fmuls and fadds of texels and constants, an fadd of a product fused as
/// [`vectorise`] fuses it (texmore_2.stderr: `%9 = fmul %6.z, 0.25`,
/// `%12 = ffma %4.z, 0.5, %9`; `FMA.v2f16 %35^.h00, u268[1].h11, u256.neg`
/// then `FMA.v2f16 %27^.h00, u269.h11, %9^.h00`).
fn lane_tree(f: &Fs<'_>, pr: &mut Pairs, v: Val, pair: u8, ubo: &UboLayout) -> Result<u8, MaliError> {
    let place = (f.key(v), pair);
    /* a lane of a vector operation ([`Fs::pair_lane`]): `nir_lower_alu_width`
     * keeps its lanes in pairs, so the lane is a half of its pair's
     * operation (gm2_fsarith_1's `nm * vec4(n, 1.0)`, x and y one
     * `FMA.v2f16` that the dot product reads as `r2.h11` and `r2.h00`); the
     * last lane of an odd width is an operation of its own, below */
    let hint = pr.hint.take();
    if let Some((lanes, i, w)) = f.pair_lane(v, hint) {
        let h = i / 2;
        if 2 * h + 1 < w {
            let (a, b) = (lanes[2 * h], lanes[2 * h + 1]);
            let x = match (0..pr.ncache).find(|&k| pr.cache[k].0 == a && pr.cache[k].1 == b) {
                Some(k) => pr.cache[k].2,
                None => {
                    let x = vectorise(f, pr, a, b, h as u8, ubo)?;
                    if pr.ncache < pr.cache.len() {
                        pr.cache[pr.ncache] = (a, b, x);
                        pr.ncache += 1;
                    }
                    x
                }
            };
            return lone_node(pr, P::Leaf(Leaf::Part { x, h: (i % 2) as u8 }), place);
        }
    }
    /* a scalar operation `nir_opt_vectorize` paired with another of the
     * same sources ([`mates`]): a half of the pair's operation */
    if let Val::Lane(n, _) = v {
        if let Some((lo, hi)) = pr.mate.get(n as usize).copied().flatten() {
            let (a, b) = (Val::Lane(lo, 0), Val::Lane(hi, 0));
            let x = match (0..pr.ncache).find(|&k| pr.cache[k].0 == a && pr.cache[k].1 == b) {
                Some(k) => pr.cache[k].2,
                None => {
                    let x = vectorise(f, pr, a, b, pair, ubo)?;
                    if pr.ncache < pr.cache.len() {
                        pr.cache[pr.ncache] = (a, b, x);
                        pr.ncache += 1;
                    }
                    x
                }
            };
            return lone_node(pr, P::Leaf(Leaf::Part { x, h: u8::from(n == hi) }), place);
        }
    }
    let p = match (f.alu_of(v), f.texel(v), v) {
        /* the narrowed position's lane alone ([`Leaf::FragLane`]), the
         * operand of a one-lane operation in its own register (an ES 1.00
         * mediump shader's `gl_FragCoord.x * gl_FragCoord.y`:
         * gm2_fcoord16_1.expect.json's `FADD.f32 r0.h0`, `FADD.f32 r1.h0`,
         * then `FMA.v2f16 r0, r0^.h00, r1^.h00, 0x0.neg`) */
        _ if f.frag16_lane(v).is_some() => P::Leaf(Leaf::FragLane(f.frag16_lane(v).unwrap_or(0))),
        /* an fp32 uniform's word narrowed alone ([`Leaf::WideLane`]) */
        _ if f.wide_uniform(v).is_some() => {
            let d = f.wide_uniform(v).ok_or(MaliError::Defect(Defect::IrIndex))?;
            let w = ubo.word(d).filter(|_| ubo.half(d).is_none()).ok_or(MaliError::FsUniform)?;
            let (base, _) = ubo.load_of(w).ok_or(MaliError::FsUniform)?;
            P::Leaf(Leaf::WideLane { base, comp: (w - base) as u8 })
        }
        /* `reflect`'s `d * -2.0` is `fneg(fadd(d, d))`, the fneg a source
         * modifier of its reader (fsvec_reflect_mediump: `FADD.v2f16 r2,
         * r2^.h00, r2^.h00`, then `FMA.v2f16 .., r2.neg.h00, ..`) */
        (Some((Alu::Fmul, [d, Val::Const(F32_MINUS_TWO), _])), ..) if vec_tree(f, v) => {
            let x = lane_tree(f, pr, d, pair, ubo)?;
            P::Op(Alu::Fneg, [lone_node(pr, P::Op(Alu::Fadd, [x, x, 0]), place)?, 0, 0])
        }
        (Some((Alu::Fmul, s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            P::Op(Alu::Fmul, [x, y, 0])
        }
        /* `min` / `max` of one lane (gm2_fsmisc_6) */
        (Some((op @ (Alu::Fmin | Alu::Fmax), s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            P::Op(op, [x, y, 0])
        }
        /* a saturate (`clamp(x, 0.0, 1.0)`, smoothstep's) or `max(x, 0.0)`,
         * folded into its producer by opt.rs (gm2_fsmisc_4, gm2_fsarith_1) */
        (Some((op @ Alu::Fclamp(clamp::ZERO_ONE | clamp::ZERO_INF), s)), ..) => {
            P::Op(op, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0])
        }
        /* an fneg of a uniform's component, a source modifier of its
         * reader (75: `FADD.v2f16 r1, 0x3C000000.h11, u2.w1.neg.h00`) */
        (Some((Alu::Fneg, s)), ..) => P::Op(Alu::Fneg, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0]),
        /* a mediump compare's `b2f16` ([`Fs::cmp_select`]): the compare of
         * one lane, its operands' trees, the select of 1.0h and 0.0 that
         * opt.rs folds into the compare's `.f1` result
         * (deqp_mixprec_cmpf: `FCMP_OR.v2f16.lt.f1 r0, r0.abs.h00, u3.w0`) */
        (Some((Alu::B2f, [m, ..])), ..)
            if f.mask16(m) && matches!(v, Val::Lane(n, _) if f.nir.nodes[n as usize].bits == 16) =>
        {
            let Some((c @ Alu::Fcmp(_), s)) = f.alu_of(m) else { return Err(MaliError::FsOperandForm) };
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            let mask = lone_node(pr, P::Op(c, [x, y, 0]), place)?;
            P::Op(Alu::B2f, [mask, 0, 0])
        }
        /* the same compare's select of a lane and 0.0 (`fmul(b2f16(a), b)`
         * is `bcsel(a, b, 0.0)`, nir_opt_algebraic.py:1695: the shader
         * object's `v_vtxOut * compare_float(..)`, before the link folds
         * the varying): `MUX.v2i16 0, b, mask.h00` */
        (Some((Alu::Bcsel, [m, t, e])), ..) if f.mask16(m) && matches!(f.alu_of(m), Some((Alu::Fcmp(_), _))) => {
            let Some((c, s)) = f.alu_of(m) else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            let mask = lone_node(pr, P::Op(c, [x, y, 0]), place)?;
            let t = lane_tree(f, pr, t, pair, ubo)?;
            let e = lane_tree(f, pr, e, pair, ubo)?;
            P::Op(Alu::Bcsel, [mask, t, e])
        }
        /* an fabs of a lane, a one-lane `FADD.v2f16 .abs` beside a constant
         * ([`vectorise`]; deqp_constop_op_abs: `FADD.v2f16 r0, r0^.abs.h00,
         * 0x0.neg`) */
        (Some((Alu::Fabs, s)), ..) => P::Op(Alu::Fabs, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0]),
        /* `sin`, `cos`, `log2` of a uniform's component, its 32-bit
         * expansion and narrowing ([`transc_rows`]) */
        /* `exp2` of a varying's lane alone (its fp32 word, narrowed alone)
         * has no reference */
        (Some((Alu::Fexp2, s)), ..) if f.input_lane(s[0]).is_some() => return Err(MaliError::FsOperandForm),
        /* `pow(a, #b)`: `b` the fp32 constant `bi_fexp_32` scales
         * ([`transc_rows`]), a leaf of its own */
        (Some((Alu::Fpow, [a, Val::Const(b), _])), ..) => {
            let x = lane_tree(f, pr, a, pair, ubo)?;
            let k = lone_node(pr, P::Leaf(Leaf::Const(b)), place)?;
            P::Op(Alu::Fpow, [x, k, 0])
        }
        (Some((op, s)), ..) if is_transc(op) => P::Op(op, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0]),
        /* `dFdx`, `dFdy` of a varying's lane or the narrowed position's,
         * scaled for Y ([`deriv_operand`]): the rows `(Fddx, 16, 1)`,
         * `(Fddy, 16, 1)` */
        (Some((op, s)), ..) if is_deriv(op) => P::Op(op, [deriv_operand(f, pr, s[0], pair, ubo, true)?, 0, 0]),
        (Some((Alu::Fadd, s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            match (pr.nodes[x as usize], pr.nodes[y as usize]) {
                (P::Op(Alu::Fmul, [m0, m1, _]), _) if f.used_once(s[0]) => P::Op(Alu::Ffma, [m0, m1, y]),
                (_, P::Op(Alu::Fmul, [m0, m1, _])) if f.used_once(s[1]) => P::Op(Alu::Ffma, [m0, m1, x]),
                _ => P::Op(Alu::Fadd, [x, y, 0]),
            }
        }
        /* [`vec_tree`]'s forms: a dot product's `ffma` (its sources in
         * order), `frsq`, `frcp` and `fract`'s `ffloor` of one lane */
        (Some((Alu::Ffma, s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            let z = lane_tree(f, pr, s[2], pair, ubo)?;
            P::Op(Alu::Ffma, [x, y, z])
        }
        /* `sqrt`'s `frcp(frsq(a))` of one IR instruction: every lane's
         * `frsq` stands before every lane's `frcp` (the walk's
         * `lower_fsqrt` of the vector, gm2_fsmisc_1), so the `frcp`s are
         * placed after both pairs */
        (Some((Alu::Frcp, s)), ..)
            if f.alu_of(s[0]).is_some_and(|(o, _)| o == Alu::Frsq) && f.key(s[0]) == f.key(v) =>
        {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            return lone_node(pr, P::Op(Alu::Frcp, [x, 0, 0]), (place.0, place.1 + 2));
        }
        (Some((op @ (Alu::Frsq | Alu::Frcp | Alu::Ffloor), s)), ..) => {
            P::Op(op, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0])
        }
        (None, Some((j, c)), _) => P::Leaf(Leaf::TexLane(j as u8, c)),
        /* a varying's lane: its half of the pair's conversion when the
         * other lane of the pair is read too ([`pair_read`]; `r0.h11` of
         * `v.y`), else its own conversion's (`r1^.h00` of `v.z`) */
        (None, None, _) if f.input_lane(v).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS || loc == FRAG_Z) => {
            let (loc, c) = f.input_lane(v).ok_or(MaliError::Defect(Defect::IrIndex))?;
            let (k, c) = index(f, loc, c)?;
            if pair_read(f, k, c) {
                P::Leaf(Leaf::Half { k, h: c / 2, swz: if c % 2 == 0 { Swz::H00 } else { Swz::H11 } })
            } else {
                P::Leaf(Leaf::Splat { k, c })
            }
        }
        /* a product with 2.0 is the late rules' `fadd(a, a)` unless the
         * fusion took it ([`late_rules`]; gm2_fsmisc_4's `FADD.v2f16.
         * clamp_0_1 r0, r0^.h00, r0^.h00`, gm2_fsarith_6's `FMA.v2f16 r1,
         * r1^.h00, 0x40000000.h11, 0x3C000000.neg.h11`) */
        (None, None, Val::Const(c)) => P::Leaf(Leaf::Const(u32::from(f16_rtne(c).ok_or(MaliError::Fp16Inexact)?))),
        /* a uniform's component: its half of the fp16 block's word */
        (None, None, _) if f.uniform_dword(v).is_some() => {
            let d = f.uniform_dword(v).ok_or(MaliError::Defect(Defect::IrIndex))?;
            let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
            let swz = if half == 0 { Swz::H00 } else { Swz::H11 };
            P::Leaf(Leaf::Uniform { base, comp: (w - base) as u8, swz })
        }
        _ => return Err(MaliError::ColourForm),
    };
    lone_node(pr, p, place)
}

/// `nir_opt_vectorize` (nir_opt_vectorize.c) on the scalar 16-bit
/// operations of the colour's trees that no vector IR operation made
/// ([`Fs::pair_lane`] keeps those paired): in program order, each `fadd`,
/// `fmul`, `ffma`, `fmin` or `fmax` joins the first earlier one of its
/// operation still alone whose sources it can share, each source pair the
/// same value, two lanes of one varying's load or two constants; two make
/// a pair, which takes no third (`bi_vectorize_filter`'s width 2).
/// Measured: gm2_fsmisc_6's `dist.x * dist.w` and `dist.y * dist.w` one
/// `FMA.v2f16 r0, r0^, r1.h11, 0x0.neg`, `dist.z * dist.w` alone.
fn mates(f: &Fs<'_>, mate: &mut [Option<(u16, u16)>; MAX_NODES]) {
    let candidate = |n: usize| {
        let nd = f.nir.nodes[n];
        f.live[n / 64] >> (n % 64) & 1 != 0
            && nd.bits == 16
            && nd.width == 1
            && matches!(nd.kind, Kind::Alu(Alu::Fadd | Alu::Fmul | Alu::Ffma | Alu::Fmin | Alu::Fmax))
            && f.pair_lane(Val::Lane(n as u16, 0), None).is_none()
    };
    let shared = |a: Val, b: Val| match (a, b) {
        (Val::Const(_), Val::Const(_)) => true,
        _ if a == b => true,
        /* an fp16 uniform's two components of one word of the block, in
         * order (deqp_mixprec_cmpv2's `u_var - vec2(..)` of `compare_vec2`'s
         * scalar lanes: one `FADD_IMM.v2f16 r0, u2.w0, #..`) */
        _ if f.uniform_dword(a).is_some() || f.uniform_dword(b).is_some() => {
            matches!((f.uniform_dword(a), f.uniform_dword(b)), (Some(d), Some(e)) if d % 2 == 0 && e == d + 1)
        }
        /* two lanes of one fp16 look-up's texel word, in order
         * (deqp_mixprec_cmptex's `texture2D(..) - vec4(..)` of
         * `compare_vec4`'s scalar lanes: `FADD_IMM.v2f16 r0, r0^, #..` on
         * the texel register itself, x,y then z,w) */
        _ if f.texel16(a) || f.texel16(b) => {
            matches!((f.texel(a), f.texel(b)), (Some((j, c)), Some((k, d))) if j == k && c % 2 == 0 && d == c + 1)
                && f.texel16(a)
        }
        _ => match (f.input_lane(a), f.input_lane(b)) {
            (Some((v, i)), Some((w, j))) => {
                matches!((index(f, v, i), index(f, w, j)), (Ok((k, _)), Ok((e, _))) if k == e)
            }
            _ => false,
        },
    };
    let mut order = [0u16; MAX_NODES];
    let mut n = 0;
    for k in (0..f.nir.n).filter(|&k| candidate(k)) {
        order[n] = k as u16;
        n += 1;
    }
    order[..n].sort_unstable_by_key(|&k| (f.key(Val::Lane(k, 0)), k));
    for i in 0..n {
        let v = order[i] as usize;
        let nv = f.nir.nodes[v];
        let Kind::Alu(op) = nv.kind else { continue };
        let found = order[..i].iter().copied().find(|&u| {
            let nu = f.nir.nodes[u as usize];
            mate[u as usize].is_none()
                && nu.kind == nv.kind
                && (0..op.arity()).all(|s| shared(nu.src[s], nv.src[s]))
                && (0..op.arity()).any(|s| nu.src[s] != nv.src[s])
        });
        if let Some(u) = found {
            mate[u as usize] = Some((u, v as u16));
            mate[v] = Some((u, v as u16));
        }
    }
}

/// Node `p` of one lane ([`lane_tree`]), one node for one value
/// (`nir_opt_cse`): `normalize`'s `frsq` read by the x,y pair's product and
/// by z's (fsvec_normalize_mediump: one `FRSQ.f16`), its tree's leaves and
/// operations made once.
fn lone_node(pr: &mut Pairs, p: P, place: (u32, u8)) -> Result<u8, MaliError> {
    if let Some(i) = (0..pr.n).find(|&i| pr.scalar[i] && pr.nodes[i] == p) {
        return Ok(i as u8);
    }
    let x = pr.push(p, place)?;
    pr.scalar[x as usize] = true;
    Ok(x)
}

/// A derivative's operand `v` ([`lane_tree`]'s `Fddx` / `Fddy` arm) as
/// nodes of one lane, the forms fsarith_138_deriv draws (docs/notes/
/// mali-derivatives.md 2): a varying's lane, read from its converted pair
/// (`V2F32_TO_V2F16 r0, r0^, r1^`, both lanes of the aligned pair
/// converted together: [`Leaf::Half`] through the lane's own half, which
/// [`op`] widens for the `CLPER` or the product reads as `r0.h11`); a lane
/// of the narrowed position alone ([`Leaf::FragLane`], `FADD.f32 r3.h0`);
/// or, with `scaled`, `dFdy`'s product of one of those with the y-flip's
/// narrowed lane (`FMA.v2f16 r1, r1^.h00, r2.h00, 0x0.neg`), the narrowing
/// `f2f16(T.x)` one node for the shader (`FADD.f32 r2.h0, u2.w0, 0x0.neg`,
/// `nir_opt_cse`). A uniform's component, a texel, a constant or any
/// computed lane has no reference ([`MaliError::FsOperandForm`]).
fn deriv_operand(f: &Fs<'_>, pr: &mut Pairs, v: Val, pair: u8, ubo: &UboLayout, scaled: bool) -> Result<u8, MaliError> {
    let place = (f.key(v), pair);
    let wpos16 = |f: &Fs<'_>, t: Val| match t {
        Val::Lane(n, 0) => matches!(
            f.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::F2f16), bits: 16, width: 1, src: [Val::Lane(u, 0), ..] }
                if matches!(f.nir.nodes[u as usize].kind, Kind::Uniform { dword: FB_WPOS_Y_TRANSFORM, .. })
        ),
        _ => false,
    };
    let p = match (f.alu_of(v), f.input_lane(v), f.frag16_lane(v)) {
        /* the narrowed position's lane (its `f2f16` node, [`Fs::frag_coord16`]) */
        (_, _, Some(c)) => P::Leaf(Leaf::FragLane(c)),
        (Some((Alu::Fmul, [a, t, _])), ..) if scaled && wpos16(f, t) => {
            let x = deriv_operand(f, pr, a, pair, ubo, false)?;
            /* the narrowed `T.x`, once: the fp32 state word (its own row of
             * the block, select.rs `UboLayout::fs16`) through `(F2f16, 16, 1)` */
            let base = ubo.word(FB_WPOS_Y_TRANSFORM).ok_or(MaliError::Defect(Defect::IrIndex))?;
            let leaf = Leaf::Uniform { base, comp: 0, swz: Swz::H01 };
            let made = (0..pr.n).find(|&i| match pr.nodes[i] {
                P::Op(Alu::F2f16, [l, ..]) => pr.nodes[l as usize] == P::Leaf(leaf),
                _ => false,
            });
            let s = match made {
                Some(i) => i as u8,
                None => {
                    let l = pr.push(P::Leaf(leaf), (f.key(t), pair))?;
                    pr.scalar[l as usize] = true;
                    let s = pr.push(P::Op(Alu::F2f16, [l, 0, 0]), (f.key(t), pair))?;
                    pr.scalar[s as usize] = true;
                    s
                }
            };
            P::Op(Alu::Fmul, [x, s, 0])
        }
        (None, Some((loc, c)), _) if (loc as usize) < MAX_VARYINGS => {
            let (k, c) = index(f, loc, c)?;
            /* the lane's aligned pair, both lanes of the load and both
             * read (a lane converted alone, as `dFdy(v.y)` of a varying
             * whose x the colour does not read, has no reference here:
             * [`pair_read`]) */
            if (c | 1) >= f.varyings[k as usize].comps || !pair_read(f, k, c) {
                return Err(MaliError::FsOperandForm);
            }
            P::Leaf(Leaf::Half { k, h: c / 2, swz: if c % 2 == 0 { Swz::H00 } else { Swz::H11 } })
        }
        _ => return Err(MaliError::FsOperandForm),
    };
    let x = pr.push(p, place)?;
    pr.scalar[x as usize] = true;
    Ok(x)
}

/// Lane `v` of a mediump shader's computed texture coordinate as nodes of
/// one lane ([`TexCoord::Computed`]): the form case 121 draws, a product of
/// a component of the run-time indexed element and a constant (`c.w *
/// 0.5`, case121.expect.json's `FMA.v2f16 r2, r1.h11, 0x38000000.h11,
/// 0x0.neg`, each source its own half in both lanes). Any other form has no
/// reference ([`MaliError::TexCoord`]).
fn coord_lane(f: &Fs<'_>, pr: &mut Pairs, v: Val) -> Result<u8, MaliError> {
    let place = (f.key(v), 0);
    let p = match (f.alu_of(v), f.ubo_lane(v), v) {
        (Some((Alu::Fmul, s)), ..) => {
            let x = coord_lane(f, pr, s[0])?;
            let y = coord_lane(f, pr, s[1])?;
            P::Op(Alu::Fmul, [x, y, 0])
        }
        (None, Some(c), _) => P::Leaf(Leaf::UboLane(c)),
        /* a product with 2.0 is the late rules' `fadd(a, a)`: no reference */
        (None, None, Val::Const(F32_TWO)) => return Err(MaliError::AlgebraicForm),
        (None, None, Val::Const(c)) => P::Leaf(Leaf::Const(u32::from(f16_rtne(c).ok_or(MaliError::Fp16Inexact)?))),
        _ => return Err(MaliError::TexCoord),
    };
    let x = pr.push(p, place)?;
    pr.scalar[x as usize] = true;
    Ok(x)
}

/// The z lane of a mediump cube direction ([`select`]): an fp32 constant
/// ([`cube_lane`]), or a node of one lane read through the swizzle of its
/// half.
#[derive(Clone, Copy)]
enum CubeLane {
    F32(u32),
    Node(u8, Swz),
}

/// A lane of a mediump cube direction the cube ops read raw ([`select`]):
/// an fp32 constant (the link's, varyconst_101_cubebias's `0x3F800000`),
/// or lane `c` of the varying at location `loc`, the load's word.
#[derive(Clone, Copy)]
enum RawLane {
    F32(u32),
    Input(u8, u8),
}

/// A mediump cube direction ([`select`]): x and y one fp16 pair and z a
/// lane alone (case41), or three raw lanes (varyconst_101_cubebias).
#[derive(Clone, Copy)]
enum CubeDir {
    Pair(u8, CubeLane),
    Raw([RawLane; 3]),
}

impl CubeLane {
    fn node(self) -> Option<u8> {
        match self {
            CubeLane::Node(x, _) => Some(x),
            CubeLane::F32(_) => None,
        }
    }
}

/// Lane `v` of a mediump cube direction alone: a component of an fp16
/// uniform (its half of the word, `u2.w1.h0` in case41.expect.json), a
/// constant (its fp16 value, which NIR folds `f2f32(f2fmp(c))` to), or a
/// one-lane tree ([`coord_lane`]).
fn cube_lane(f: &Fs<'_>, pr: &mut Pairs, v: Val, ubo: &UboLayout) -> Result<CubeLane, MaliError> {
    if let Val::Const(c) = v {
        let h = f16_rtne(c).ok_or(MaliError::Fp16Inexact)?;
        return Ok(CubeLane::F32(super::fau::half_to_float(h)));
    }
    if let (None, Some(d)) = (f.alu_of(v), f.uniform_dword(v)) {
        let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
        let x = pr.push(P::Leaf(Leaf::Uniform { base, comp: (w - base) as u8, swz: Swz::H01 }), (f.key(v), 0))?;
        pr.scalar[x as usize] = true;
        return Ok(CubeLane::Node(x, if half == 0 { Swz::H00 } else { Swz::H11 }));
    }
    Ok(CubeLane::Node(coord_lane(f, pr, v)?, Swz::H00))
}

/// `build_cube_desc` ([`nir::Nir::cube_desc`]) of an fp32 direction as
/// rows, in the builder's order (case41.expect.json: `CUBEFACE1`,
/// `CUBEFACE2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, the `FMA`s,
/// `MUX.i32.bit`): the staging words S and T.
fn cube_rows(p: &mut Prog<'_>, rows: &mut u128, d: [Src; 3]) -> Result<[Src; 2], MaliError> {
    let mut r = |op: Alu, s: &[Src]| -> Result<Src, MaliError> {
        Ok(Src::ssa(row(p, rows, Row { op, bits: 32, width: 1 }, s)?))
    };
    let half = Src::of(Idx::Const(F32_HALF));
    let [x, y, z] = d;
    let max = r(Alu::Cubeface1, &[x, y, z])?;
    let face = r(Alu::Cubeface2, &[x, y, z])?;
    let s = r(Alu::CubeSsel, &[z, x, face])?;
    let t = r(Alu::CubeTsel, &[y, z, face])?;
    let rcp = r(Alu::Frcp, &[max])?;
    let k = r(Alu::Fmul, &[half, rcp])?;
    let s = r(Alu::Ffma, &[s, k, half])?;
    let s = r(Alu::Fclamp(clamp::ZERO_ONE), &[s])?;
    let t = r(Alu::Ffma, &[t, k, half])?;
    let t = r(Alu::Fclamp(clamp::ZERO_ONE), &[t])?;
    Ok([r(Alu::BitfieldSelect, &[Src::of(Idx::Const(nir::CUBE_S_MASK)), s, face])?, t])
}

/// `bifrost_nir_lower_algebraic_late` (bifrost_nir_algebraic.py:31) after
/// the fusion: a product with 2.0 in both halves is `fadd(a, a)`.
fn late_rules(pr: &mut Pairs, roots: [u8; 2]) {
    let mut live = [false; MAX_PAIRS];
    for r in roots {
        mark(pr, r, &mut live);
    }
    for i in 0..pr.n {
        if !live[i] {
            continue;
        }
        if let P::Op(Alu::Fmul, [a, b, _]) = pr.nodes[i] {
            /* a pair's 2.0 in both halves, a lane alone's in its low one
             * ([`lane_tree`]) */
            let two = |x: u8| {
                let k = if pr.scalar[x as usize] { F16X2_TWO & 0xffff } else { F16X2_TWO };
                pr.nodes[x as usize] == P::Leaf(Leaf::Const(k))
            };
            if two(b) {
                pr.nodes[i] = P::Op(Alu::Fadd, [a, a, 0]);
            } else if two(a) {
                pr.nodes[i] = P::Op(Alu::Fadd, [b, b, 0]);
            }
        }
    }
}

/// The nodes a tree uses; a product the fusion took is not one of them.
fn mark(pr: &Pairs, x: u8, live: &mut [bool; MAX_PAIRS]) {
    if live[x as usize] {
        return;
    }
    live[x as usize] = true;
    match pr.nodes[x as usize] {
        P::Op(op, s) => {
            for y in s.iter().take(op.arity()) {
                mark(pr, *y, live);
            }
        }
        P::Leaf(Leaf::Zext(y) | Leaf::Lane { x: y, .. } | Leaf::Part { x: y, .. }) => mark(pr, y, live),
        P::Leaf(Leaf::Pack { x, y }) => {
            mark(pr, x, live);
            mark(pr, y, live);
        }
        P::Leaf(_) => {}
    }
}

/// The fragment program of `f` ([`walk`], [`layout`]) into `p`: the loads
/// and conversions where their first reads stand, the look-up and the
/// run-time indexed read where their IR instructions do, each two-lane
/// operation where its IR operation stands (pair 0, then pair 1), every
/// fneg and fabs as `FABSNEG`, then ATEST and BLEND. `ubo` is the fp16
/// block's layout ([`UboLayout::fs16`]).
pub fn select(p: &mut Prog<'_>, f: &mut Fs<'_>, ubo: &UboLayout) -> Result<(), MaliError> {
    let mut pr = Pairs {
        nodes: [P::Leaf(Leaf::Const(0)); MAX_PAIRS],
        place: [(0, 0); MAX_PAIRS],
        scalar: [false; MAX_PAIRS],
        n: 0,
        cache: [(Val::Undef, Val::Undef, 0); 16],
        ncache: 0,
        hint: None,
        mate: [None; MAX_NODES],
    };
    mates(f, &mut pr.mate);
    let mut roots = [0u8; 2];
    let c = f.colour.ok_or(MaliError::NoColour)?;
    /* each further target's two fp16 words: constants only (fsout_106_mrt3:
     * `MOV.i32` of each word into r0-r1, then its `BLEND.v4.f16`); a
     * uniform's has no reference here */
    let mut more = [(0u8, [0u32; 2]); MAX_TARGETS - 1];
    let mut nmore = 0;
    for (rt, lanes) in targets(f) {
        let mut h = [0u32; 4];
        for (x, v) in h.iter_mut().zip(lanes) {
            let Val::Const(b) = v else { return Err(MaliError::FragOutput) };
            *x = u32::from(f16_rtne(b).ok_or(MaliError::Fp16Inexact)?);
        }
        more[nmore] = (rt, [h[0] | h[1] << 16, h[2] | h[3] << 16]);
        nmore += 1;
    }
    for (k, r) in roots.iter_mut().enumerate() {
        let (a, b) = (c[2 * k], c[2 * k + 1]);
        /* one scalar operation's value in both lanes of the word
         * (`vec4(smoothstep(..))`, gm2_fsmisc_4): the one-lane tree once,
         * the word `MKVEC.v2i16 x.h0, x.h0`, which opt.rs makes
         * `IADD.v2u16 r0, r0^.h00, 0x0`, the other word of the same value
         * its copy (`MOV.i32 r1, r0`, below) */
        /* so is one fp16 texel in both lanes (gm2_texswz_1's `vec4(t.r)`:
         * `IADD.v2u16 r0, r0^.h00, 0x0` of the look-up's word, `MOV.i32
         * r1, r0`) */
        let texel16 = f.texel(a).is_some_and(|(j, _)| f.tex_f16(j));
        if a == b && (f.alu_of(a).is_some() && !f.wide(a) && !vec_tree(f, a) || texel16) {
            let x = lane_tree(f, &mut pr, a, k as u8, ubo)?;
            let p = P::Leaf(Leaf::Pack { x, y: x });
            *r = match (0..pr.n).find(|&i| pr.nodes[i] == p) {
                Some(i) => i as u8,
                None => pr.push(p, (f.key(a), k as u8))?,
            };
            continue;
        }
        *r = match lane_word(f, &mut pr, a, b, k as u8, ubo)? {
            Some(x) => x,
            None => vectorise(f, &mut pr, a, b, k as u8, ubo)?,
        };
    }
    f.npairs = pr.n;
    late_rules(&mut pr, roots);
    for r in roots {
        pr.note(r, Place::Output, &mut f.prods);
    }
    /* a uniform half in both lanes of an output word (`MOV.i32` takes no
     * half swizzle; no reference draws the form Mesa makes of it) */
    let splat = |x: u8| matches!(pr.nodes[x as usize], P::Leaf(Leaf::Uniform { swz: Swz::H00 | Swz::H11, .. }));
    if f.prods & !ADMITTED != 0 || roots.iter().any(|&r| splat(r)) {
        return Err(MaliError::FsOperandForm);
    }
    /* a look-up's word swapped is an output word: an operation reading it
     * (`t.yx * 2.0`, a swizzle on the operation's source) has no reference */
    let swapped = |x: u8| matches!(pr.nodes[x as usize], P::Leaf(Leaf::TexSwapped(..)));
    if pr.nodes[..pr.n].iter().any(|n| matches!(*n, P::Op(op, s) if s[..op.arity()].iter().any(|&x| swapped(x)))) {
        return Err(MaliError::TexLanes);
    }
    /* the texels read with a gap between them (`.rb`): Mesa writes the
     * ones read, packed ([`tex_mask`]), a layout the leaves here do not
     * index ([`Leaf::TexLane`] counts from the first texel); no reference
     * draws it */
    if f.live_look_ups().any(|(_, t)| t.lanes & (t.lanes + 1) != 0) {
        return Err(MaliError::TexLanes);
    }
    /* more than one look-up: each at a vec2 varying read whole or two
     * constants (texmore_2's `texture2D(u_tex0, v_uv)` and `texture2D(
     * u_tex1, v_uv)`); a computed coordinate or a cube among them has no
     * reference */
    let lone = match f.live_look_ups().count() {
        1 => f.live_look_ups().next().map(|(_, t)| *t),
        _ if f.live_look_ups().any(|(_, t)| t.coord == TexCoord::Computed) => return Err(MaliError::TexCount),
        _ => None,
    };
    /* a computed coordinate: one fp16 lane in both (case 121's `vec2(c.w *
     * 0.5)`), its nodes of one lane, converted at the look-up; two lanes
     * have no reference */
    let coord = match lone {
        Some(TexLookup { coord: TexCoord::Computed, cube: None, node, .. }) => {
            let nd = f.nir.nodes[node as usize];
            if nd.src[0] != nd.src[1] {
                return Err(MaliError::TexCoord);
            }
            Some(coord_lane(f, &mut pr, nd.src[0])?)
        }
        _ => None,
    };
    /* a cube's direction: x and y one pair (`nir_opt_vectorize` pairs
     * lanes 0 and 1 of the vec3; case41.expect.json's `FMA.v2f16 r0, r0^,
     * u3.w0, r1^`), z a lane alone (`u2.w1.h0`), converted at the look-up;
     * or raw varying lanes and constants (varyconst_101_cubebias: `v_dir`
     * read by the cube ops alone, which are fp32, so `nir_lower_mediump_io`
     * narrows nothing: `CUBEFACE1 r2, 0x3F800000, r0, r1` of the load's
     * words and the link's constant) */
    let cube = match lone {
        Some(TexLookup { cube: Some(_), node, .. }) => {
            let nd = f.nir.nodes[node as usize];
            let raw = |v: Val| match v {
                Val::Const(c) => Some(RawLane::F32(c)),
                _ => f.input_lane(v).filter(|&(loc, _)| loc != POINT_COORD).map(|(loc, c)| RawLane::Input(loc, c)),
            };
            match (raw(nd.src[0]), raw(nd.src[1]), raw(nd.src[2])) {
                (Some(x), Some(y), Some(z)) => Some(CubeDir::Raw([x, y, z])),
                _ => {
                    let xy = vectorise(f, &mut pr, nd.src[0], nd.src[1], 0, ubo)?;
                    /* the pair's productions are the colour's
                     * ([`ADMITTED`]): a form in a place no reference
                     * draws is refused here as it is there (`a2 * -3.0 +
                     * 0.25`: Mesa's FMA reads -3.0 as a FAU word,
                     * ours its negation, fstex_cubeno_operandform_fmanegk) */
                    pr.note(xy, Place::Output, &mut f.prods);
                    if f.prods & !ADMITTED != 0 {
                        return Err(MaliError::FsOperandForm);
                    }
                    let z = cube_lane(f, &mut pr, nd.src[2], ubo)?;
                    Some(CubeDir::Pair(xy, z))
                }
            }
        }
        _ => None,
    };
    /* the discards' compares: one 16-bit lane each, its operands leaves
     * of one lane (a uniform's half, a constant, [`cmp_leaf`]), in
     * program order */
    let mut discards = [(0u32, Cmpf::Lt, [(0u8, Swz::H00); 2]); MAX_DISCARDS];
    let mut nd = 0;
    for id in 0..f.nir.n {
        if f.nir.nodes[id].kind != Kind::Discard {
            continue;
        }
        let Val::Lane(cn, _) = f.nir.nodes[id].src[0] else { return Err(MaliError::DiscardForm) };
        let cmp = f.nir.nodes[cn as usize];
        let Kind::Alu(Alu::Fcmp(cmpf)) = cmp.kind else { return Err(MaliError::DiscardForm) };
        let a = cmp_leaf(f, &mut pr, cmp.src[0], ubo)?;
        let b = cmp_leaf(f, &mut pr, cmp.src[1], ubo)?;
        *discards.get_mut(nd).ok_or(MaliError::BlockTableFull)? = (f.keys[f.origin[id] as usize], cmpf, [a, b]);
        nd += 1;
    }
    f.npairs = pr.n;
    let alpha = match c[3] {
        Val::Const(b) => Some(f16_rtne(b).ok_or(MaliError::Fp16Inexact)?),
        _ => None,
    };
    f.alpha = alpha;
    /* a load holds the lanes read and no other; a lane converted alone and
     * in a pair is not referenced */
    let mut used = [0u8; MAX_VARYINGS];
    let mut conv_form = [[None::<bool>; 4]; MAX_VARYINGS];
    for x in &pr.nodes[..pr.n] {
        /* an `exp2`'s fp32 words: read as loaded, not converted */
        if let P::Leaf(Leaf::Raw { k, h }) = *x {
            used[k as usize] |= 3 << (2 * h);
            continue;
        }
        let (lanes, pair) = match *x {
            P::Leaf(Leaf::Half { k, h, .. }) => ([(k, 2 * h), (k, 2 * h + 1)], true),
            P::Leaf(Leaf::Splat { k, c } | Leaf::Mk { k, c, .. }) => ([(k, c), (k, c)], false),
            P::Leaf(Leaf::Lanes { k, c, e, d }) => ([(k, c), (e, d)], false),
            _ => continue,
        };
        for (k, c) in lanes {
            used[k as usize] |= 1 << c;
            let cf = &mut conv_form[k as usize][c as usize];
            if cf.is_some_and(|p| p != pair) {
                return Err(MaliError::ColourForm);
            }
            *cf = Some(pair);
        }
    }
    let conv_order = conversion_order(f, &pr, roots, &mut conv_form)?;
    /* a coordinate is read as fp32, not converted (one load may be the
     * coordinate of several look-ups: texmore_2's `@r0:r1`) */
    if f.live_look_ups().any(|(_, t)| t.load.is_some_and(|k| used[k as usize] != 0)) {
        return Err(MaliError::TexCoordShared);
    }
    for (_, t) in f.live_look_ups() {
        if let Some(k) = t.load {
            used[k as usize] = 3;
        }
    }
    /* so are a cube direction's raw lanes (varyconst_101_cubebias) */
    if let Some(CubeDir::Raw(lanes)) = cube {
        for l in lanes {
            let RawLane::Input(loc, c) = l else { continue };
            let (k, lane) = index(f, loc, c)?;
            if used[k as usize] & (1 << lane) != 0 {
                return Err(MaliError::TexCoordShared);
            }
            used[k as usize] |= 1 << lane;
        }
    }
    /* a load holds the lanes read, and beside them only lanes the link
     * keeps ([`FsVarying::whole`]: 78's v4 load, of which lanes 0 and 3
     * are read). A lane past the load is this back end's defect; a lane
     * read through a form these leaves do not name, a form no reference
     * draws */
    for (k, v) in f.varyings[..f.nvar].iter().enumerate() {
        let full = (1u8 << v.comps) - 1;
        if used[k] & !full != 0 {
            return Err(MaliError::Defect(Defect::IrIndex));
        }
        if used[k] == 0 || (used[k] != full && !v.whole) {
            return Err(MaliError::FsVaryingLoad);
        }
    }
    let mut live = [false; MAX_PAIRS];
    let cube_nodes = cube
        .map(|d| match d {
            CubeDir::Pair(xy, z) => [Some(xy), z.node()],
            CubeDir::Raw(_) => [None, None],
        })
        .into_iter()
        .flatten()
        .flatten();
    let cmp_nodes = discards[..nd].iter().flat_map(|d| d.2.map(|x| x.0));
    for r in roots.into_iter().chain(coord).chain(cube_nodes).chain(cmp_nodes) {
        mark(&pr, r, &mut live);
    }
    /* events: (key, pair, what, its index): a load, the look-up, the
     * indexed read, an operation, in this order where their places are
     * equal */
    const LOAD: u8 = 0;
    const TEX: u8 = 1;
    const OFFSET: u8 = 2;
    const UBO: u8 = 3;
    const NODE: u8 = 4;
    const DISCARD: u8 = 5;
    const FRAG: u8 = 6;
    const FRAG_WORD: u8 = 7;
    let mut events =
        [(0u32, 0u8, LOAD, 0u8); MAX_VARYINGS + MAX_PAIRS + MAX_LOOK_UPS + 3 + 2 * MAX_UBO_READS + MAX_DISCARDS];
    let mut ne = 0;
    for (k, d) in discards[..nd].iter().enumerate() {
        events[ne] = (d.0, 0, DISCARD, k as u8);
        ne += 1;
    }
    /* a load only the colour's own conversions read stands before the
     * first of them, at the store ([`conversion_order`]; `nir_opt_move`
     * sinks it there, fsconv_slot: d.y's slot 1 first); any other where
     * its first read stands */
    let raw_read = |k: usize| {
        f.live_look_ups().any(|(_, t)| t.load == Some(k as u8))
            || matches!(cube, Some(CubeDir::Raw(lanes)) if lanes.iter().any(|l| match *l {
                RawLane::Input(loc, c) => index(f, loc, c).is_ok_and(|(j, _)| j as usize == k),
                RawLane::F32(_) => false,
            }))
    };
    for (k, v) in f.varyings[..f.nvar].iter().enumerate() {
        let at_store = conv_order.at_store[..conv_order.n].iter().any(|s| s.0 as usize == k);
        if conv_order.at_load[k][0] == NO_LANE && at_store && !raw_read(k) {
            continue;
        }
        events[ne] = (v.key(), 0, LOAD, k as u8);
        ne += 1;
    }
    /* the narrowed position, where gl_FragCoord is read (its fp32 build,
     * the narrowings and the word: [`frag16`]; the lanes alone for a
     * derivative, [`frag16_lanes`]) */
    let frag_word = (0..pr.n).any(|i| live[i] && pr.nodes[i] == P::Leaf(Leaf::Frag));
    let frag_lane = (0..pr.n).any(|i| live[i] && matches!(pr.nodes[i], P::Leaf(Leaf::FragLane(_))));
    if let (Some([x, _]), true) = (f.frag16, frag_word || frag_lane) {
        events[ne] = (f.key(x), 0, FRAG, 0);
        ne += 1;
        if frag_word {
            events[ne] = (f.key(x), 0, FRAG_WORD, 0);
            ne += 1;
        }
    }
    let key_of = |at: u16| f.keys.get(at as usize).copied().ok_or(MaliError::Defect(Defect::IrIndex));
    for (j, t) in f.live_look_ups() {
        events[ne] = (key_of(t.at)?, 0, TEX, j as u8);
        ne += 1;
    }
    /* the indexed vec4 element the colour reads (its lanes in a live pair),
     * where its IR instruction stands */
    let vec4 = f.nubo == 1 && f.ubo_comps(f.ubo_idx[0]) == 4;
    let ubo_leaf = |i: usize| matches!(pr.nodes[i], P::Leaf(Leaf::Ubo(_) | Leaf::UboLane(_)));
    /* one only the colour's words read is sunk to the store, after the
     * colour's conversions (`nir_opt_move` of `load_ubo`, bifrost_nir.c
     * :380-381; uniidx_j: `V2F32_TO_V2F16` of `v_color.rg`, then the
     * element's address and load) */
    let ubo_at_store = vec4 && (0..pr.n).filter(|&i| ubo_leaf(i)).all(|i| roots.contains(&(i as u8)));
    if vec4 && !ubo_at_store && (0..pr.n).any(ubo_leaf) {
        events[ne] = (key_of(f.origin[f.ubo_idx[0] as usize])?, 0, UBO, 0);
        ne += 1;
    }
    /* each float array read the colour reads: its offset where its IR
     * instruction stands, its load moved down to just before its first
     * reader (`nir_opt_move` of `load_ubo`, bifrost_nir.c; case108_uniidx:
     * the offsets of the first two reads, then both loads, then their sum) */
    for k in 0..f.nubo.min(MAX_UBO_READS) {
        let read = |i: usize| match pr.nodes[i] {
            P::Leaf(Leaf::UboHalf(r) | Leaf::UboWord(r, _)) => r == k as u8,
            _ => false,
        };
        if vec4 || !(0..pr.n).any(|i| live[i] && read(i)) {
            continue;
        }
        let at = key_of(f.origin[f.ubo_idx[k] as usize])?;
        events[ne] = (at, 0, OFFSET, k as u8);
        ne += 1;
        /* a read only the colour's word reads is loaded at the word (after
         * ATEST, [`select`]'s words) */
        let first = (0..pr.n)
            .filter(|&i| match pr.nodes[i] {
                P::Op(op, s) => live[i] && s[..op.arity()].iter().any(|&y| read(y as usize)),
                _ => false,
            })
            .map(|i| pr.place[i])
            .min();
        if let Some(first) = first {
            events[ne] = (first.0, first.1, UBO, k as u8);
            ne += 1;
        }
    }
    for i in 0..pr.n {
        if live[i] && matches!(pr.nodes[i], P::Op(..)) {
            events[ne] = (pr.place[i].0, pr.place[i].1, NODE, i as u8);
            ne += 1;
        }
    }
    let events = &mut events[..ne];
    /* an fneg the fusion made stands before its ffma: the index, last, keeps
     * the order the events were made in (no_std has no stable sort) */
    /* gl_FragCoord.z of a mediump shader after the narrowed x and y read
     * with it: `vec4(u2f32(load_pixel_coord), z, w)`, narrowed lane after
     * lane, and x and y's word after them (deqp_misc_fragcoord_xyz: x, y's
     * `FADD.f32 .h0`, then z's `LD_VAR_SPECIAL` and its own, then the
     * `MKVEC.v2i16` of x and y) */
    let frag_event = events.iter().any(|e| e.2 == FRAG);
    let rank = |e: &(u32, u8, u8, u8)| match e.2 {
        LOAD if frag_event && f.varyings[e.3 as usize].location == FRAG_Z => FRAG + 1,
        FRAG_WORD => FRAG + 2,
        w => w,
    };
    events.sort_unstable_by_key(|e| (e.0, e.1, rank(e), e.3));
    let mut lv = Leaves {
        conv: [[None; 4]; MAX_VARYINGS],
        conv_hi: [0; MAX_VARYINGS],
        raw: [[None; 4]; MAX_VARYINGS],
        tex: [[None; 4]; MAX_LOOK_UPS],
        ubo: *ubo,
        ubo_words: [None; 2],
        ubo_off: [None; MAX_UBO_READS],
        ubo_half: [None; MAX_UBO_READS],
        ubo_lanes: [[None; 4]; MAX_UBO_READS],
        desc: [[None; 2]; MAX_SIZE_QUERIES],
        frag: None,
        frag_lanes: [None; 2],
    };
    let mut val = [None::<Src>; MAX_PAIRS];
    let mut rows = 0u128;
    let mut texel = None;
    for &(_, _, what, x) in events.iter() {
        match what {
            LOAD => {
                load(p, f, x as usize, &mut lv)?;
                for &c in conv_order.at_load[x as usize].iter().take_while(|&&c| c != NO_LANE) {
                    conversion(p, f, x as usize, c, &conv_form, &mut lv, &mut rows)?;
                }
            }
            TEX => {
                /* the coordinate's lane converted (`F16_TO_F32 r2, r2^.h0`),
                 * the one value both staging words */
                let f2f32 = |p: &mut Prog<'_>, rows: &mut u128, s: Src| -> Result<Src, MaliError> {
                    Ok(Src::ssa(row(p, rows, Row { op: Alu::F2f32, bits: 32, width: 1 }, &[s])?))
                };
                let mut staging = [Src::NULL; 3];
                let n = match (coord, cube) {
                    (Some(r), _) => {
                        let x = f2f32(p, &mut rows, operand(&pr, r, &lv, &val)?.swz(Swz::H00))?;
                        staging[..2].copy_from_slice(&[x, x]);
                        2
                    }
                    /* each lane to fp32 (`F16_TO_F32 r1, r0.h0`, `r0^.h1`,
                     * `u2.w1.h0`), then the descriptor; raw lanes as they
                     * are */
                    (None, Some(dir)) => {
                        let d = match dir {
                            CubeDir::Pair(xy, z) => {
                                let w = operand(&pr, xy, &lv, &val)?;
                                let x = f2f32(p, &mut rows, w.swz(Swz::H00))?;
                                let y = f2f32(p, &mut rows, w.swz(Swz::H11))?;
                                let z = match z {
                                    CubeLane::F32(c) => Src::of(Idx::Const(c)),
                                    CubeLane::Node(z, swz) => {
                                        f2f32(p, &mut rows, operand(&pr, z, &lv, &val)?.swz(swz))?
                                    }
                                };
                                [x, y, z]
                            }
                            CubeDir::Raw(lanes) => {
                                let mut d = [Src::NULL; 3];
                                for (s, l) in d.iter_mut().zip(lanes) {
                                    *s = match l {
                                        RawLane::F32(c) => Src::of(Idx::Const(c)),
                                        RawLane::Input(loc, c) => {
                                            let (k, l) = index(f, loc, c)?;
                                            let w = lv.raw[k as usize][l as usize & 3];
                                            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?)
                                        }
                                    };
                                }
                                d
                            }
                        };
                        /* a bias's uniform widened before the descriptor:
                         * the bias argument's `f2f32` is an instruction
                         * before the look-up, while `va_lower_tex` builds
                         * the descriptor and the 8.8 chain at the look-up
                         * (pan_nir_lower_tex.c:805, :827, :936-937). Its
                         * fp32 word, live across the cube ops, makes the
                         * order's peak pressure 7 against the scheduler's 6,
                         * so `bi_pressure_schedule` keeps its own order,
                         * `CUBEFACE2_V9` first (fstex_cubebias_uni.expect
                         * .json; the place is read from these bytes, not
                         * from a Mesa IR dump). varyconst_101_cubebias's
                         * order is the scheduler's either way */
                        let t = f.look_ups().get(x as usize).copied().ok_or(MaliError::Defect(Defect::IrIndex))?;
                        let bias = if t.bias != Val::Undef {
                            let d = f.uniform_dword(t.bias).ok_or(MaliError::Defect(Defect::IrIndex))?;
                            let (w, half, base) = ubo.half(d).ok_or(MaliError::FsUniform)?;
                            let swz = if half == 0 { Swz::H00 } else { Swz::H11 };
                            Some(f2f32(p, &mut rows, Src::of(Idx::Ubo { base, comp: (w - base) as u8 }).swz(swz))?)
                        } else {
                            None
                        };
                        let st = cube_rows(p, &mut rows, d)?;
                        staging[..2].copy_from_slice(&st);
                        /* the bias word after the descriptor
                         * ([`TexLookup::bias`]): the uniform's half widened
                         * (above), then `build_sfixed_8_8` as rows (`FMA.f32
                         * .clamp_m1_1` by 1/16, `FMA.f32` by 4096,
                         * `F32_TO_S32.rtz`, `U16_TO_U32 .h0`) */
                        if let Some(b) = bias {
                            let mut r = |op: Alu, bits: u8, s: &[Src]| -> Result<Src, MaliError> {
                                Ok(Src::ssa(row(p, &mut rows, Row { op, bits, width: 1 }, s)?))
                            };
                            let l = r(Alu::Fmul, 32, &[b, Src::of(Idx::Const(nir::LOD_DIV16))])?;
                            let l = r(Alu::Fclamp(clamp::M1_1), 32, &[l])?;
                            let l = r(Alu::Fmul, 32, &[l, Src::of(Idx::Const(nir::LOD_8_8))])?;
                            let l = r(Alu::F2i, 16, &[l])?;
                            staging[2] = r(Alu::U2u32, 32, &[l])?;
                            3
                        } else {
                            2
                        }
                    }
                    (None, None) => 0,
                };
                tex(p, f, x as usize, &mut lv, (n > 0).then_some(&staging[..n]))?;
                /* a colour that is the texel whole: `bi_emit_tex_valhall`
                 * collects the split words into the look-up's value right
                 * after it (bifrost_compile.c), so its moves stand before
                 * ATEST (case41.expect.json: `MOV.i32 r0, r2^`, `MOV.i32
                 * r1, r3`, then `ATEST ... r3^.h1`). gl_FragColor's store
                 * copies the colour itself ([`Fs::fragcolor`]): the moves
                 * the words' own, after ATEST (fstex_cube_whole: `ATEST`
                 * then `BLEND` of the look-up's `@r0:r1`, the moves gone) */
                let whole = [P::Leaf(Leaf::Tex(x, 0)), P::Leaf(Leaf::Tex(x, 1))];
                if !f.fragcolor && roots.map(|r| pr.nodes[r as usize]) == whole {
                    let mut w = [Src::NULL; 2];
                    for (k, x) in w.iter_mut().enumerate() {
                        *x = Src::ssa(alu(p, Op::MovI32, &[operand(&pr, roots[k], &lv, &val)?])?);
                    }
                    texel = Some(alu(p, Op::Collect, &w)?);
                }
            }
            FRAG => {
                let lanes = frag16_lanes(p, &lv, &mut rows)?;
                lv.frag_lanes = lanes.map(Some);
            }
            FRAG_WORD => {
                let lanes = lv.frag_lanes.map(|l| l.ok_or(MaliError::Defect(Defect::IrIndex)));
                lv.frag = Some(frag16(p, [lanes[0]?, lanes[1]?], &mut rows)?);
            }
            UBO if vec4 => ubo_load(p, f, &mut lv, &mut rows)?,
            OFFSET => ubo_offset(p, f, x as usize, &mut lv, &mut rows)?,
            UBO => ubo_fetch(p, f, x as usize, &mut lv)?,
            /* `demote_if`: `DISCARD.b32` of the 16-bit mask's low half
             * (bifrost_compile.c:1979-1987), fused by opt.rs */
            DISCARD => {
                let (_, cmpf, ops) = discards[x as usize];
                let mut s = [Src::NULL; 2];
                for (y, (n, swz)) in s.iter_mut().zip(ops) {
                    *y = operand(&pr, n, &lv, &val)?.swz(swz);
                }
                let mask = row(p, &mut rows, Row { op: Alu::Fcmp(cmpf), bits: 16, width: 1 }, &s)?;
                p.push(Ins::new(Op::DiscardB32, &[], &[Src::ssa(mask).swz(Swz::H00)]))?;
            }
            _ => op(p, &pr, x, &lv, &mut val, &mut rows)?,
        }
    }
    /* the colour's own lanes, converted at its store
     * (`nir_lower_mediump_io`'s `f2fmp` before the 32-bit store,
     * nir_lower_mediump.c:102-104), after every event above, a discard
     * included (fsconv_discw), each load made before its first */
    for &(k, c) in &conv_order.at_store[..conv_order.n] {
        conversion(p, f, k as usize, c, &conv_form, &mut lv, &mut rows)?;
    }
    if ubo_at_store && (0..pr.n).any(ubo_leaf) {
        ubo_load(p, f, &mut lv, &mut rows)?;
    }
    /* the colour's fp32 uniform words, narrowed at its store as a
     * varying's (fshighp_color: two `V2F32_TO_V2F16`, then ATEST) */
    for r in roots {
        if let P::Leaf(Leaf::Wide { base, comp }) = pr.nodes[r as usize] {
            if val[r as usize].is_none() {
                val[r as usize] = Some(Src::ssa(wide(p, base, comp, &mut rows)?));
            }
        }
    }
    /* ATEST on the alpha: the constant's half, or the high half of the
     * second pair's word (a uniform word's own half when it is a splat;
     * a lane alone's own conversion, fsconv_alpha3's `ATEST .. r2.h0`) */
    let at = match (alpha, pr.nodes[roots[1] as usize]) {
        (None, P::Leaf(Leaf::Lanes { e, d, .. })) => conv_lane(&lv, e, d)?,
        (Some(h), _) => Src::of(Idx::Const(h as u32)).swz(Swz::H00),
        /* a word made after ATEST of the alpha's own conversion: a lane in
         * both halves, or a constant then the lane
         * (deqp_colourpair_rnd_fs_ck_dup's `ATEST .. r0.h0`, then
         * `MKVEC.v2i16` and `IADD.v2u16 r2, r0^.h00`) */
        (None, P::Leaf(Leaf::Splat { k, c } | Leaf::Mk { k, c, first: true, .. })) => conv_lane(&lv, k, c)?,
        (None, P::Leaf(Leaf::Uniform { base, comp, swz })) => {
            Src::of(Idx::Ubo { base, comp }).swz(if swz == Swz::H00 { Swz::H00 } else { Swz::H11 })
        }
        /* a packed word is made after ATEST, which reads its high lane's
         * half (fsarith_77_transc: `ATEST .. r3.h0`, then `MKVEC.v2i16`) */
        (None, P::Leaf(Leaf::Pack { y, .. })) => match pr.nodes[y as usize] {
            /* a texel's own half of the look-up's word (gm2_texswz_1's
             * `ATEST .. r0.h0`) */
            P::Leaf(Leaf::TexLane(..)) => operand(&pr, y, &lv, &val)?,
            _ => operand(&pr, y, &lv, &val)?.swz(Swz::H00),
        },
        /* a swapped word's high lane is the look-up word's low half */
        (None, P::Leaf(Leaf::TexSwapped(..))) => operand(&pr, roots[1], &lv, &val)?.swz(Swz::H00),
        (None, _) => operand(&pr, roots[1], &lv, &val)?.swz(Swz::H11),
    };
    let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), at, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    let mut words = [Src::NULL; 2];
    for k in (0..2).filter(|_| texel.is_none()) {
        /* both words one packed value (`vec4(s)` of a computed scalar,
         * gm2_fsmisc_4): the second the first's copy, `MOV.i32 r1, r0` */
        if k == 1 && roots[1] == roots[0] && matches!(pr.nodes[roots[0] as usize], P::Leaf(Leaf::Pack { .. })) {
            words[1] = Src::ssa(alu(p, Op::MovI32, &[words[0]])?);
            continue;
        }
        let w = &mut words[k];
        *w = match pr.nodes[roots[k] as usize] {
            P::Leaf(Leaf::Mk { k: v, c, half, first }) => Src::ssa(mkvec(p, &lv, v, c, half, first, &mut rows)?),
            P::Leaf(Leaf::Lanes { k: v, c, e, d }) => Src::ssa(mkvec_lanes(p, &lv, [(v, c), (e, d)], &mut rows)?),
            /* one lane in both halves: `MKVEC.v2i16 x.h0, x.h0`, which
             * opt.rs `lower_mkvec_swz` makes `IADD.v2u16 r1, r0.h00, 0x0`
             * (gm2_arithconst_3) */
            P::Leaf(Leaf::Splat { k: v, c }) => Src::ssa(mkvec_lanes(p, &lv, [(v, c), (v, c)], &mut rows)?),
            P::Leaf(Leaf::Zext(x)) => {
                /* a read the word alone reads is loaded here, after ATEST
                 * (`nir_opt_move` sinks it to the colour's store; uniarray_b:
                 * `ATEST`, then `LD_PKA.i16`, then `U16_TO_U32`) */
                if let P::Leaf(Leaf::UboHalf(r)) = pr.nodes[x as usize] {
                    if lv.ubo_half.get(r as usize).copied().flatten().is_none() {
                        ubo_fetch(p, f, r as usize, &mut lv)?;
                    }
                }
                let s = [operand(&pr, x, &lv, &val)?];
                Src::ssa(row(p, &mut rows, Row { op: Alu::U2u32, bits: 32, width: 1 }, &s)?)
            }
            P::Leaf(Leaf::Lane { x, half }) => {
                let s = [operand(&pr, x, &lv, &val)?.swz(Swz::H00), Src::of(Idx::Const(half as u32)).swz(Swz::H00)];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
            P::Leaf(Leaf::Pack { x, y }) => {
                /* each node's lane: a lane alone is its low half, a varying's
                 * lane in its pair's conversion its own half (gm2_fsmisc_2's
                 * `r0^.h1`) */
                let half = |n: u8| match pr.nodes[n as usize] {
                    P::Leaf(Leaf::Half { swz: Swz::H11, .. }) => Swz::H11,
                    P::Leaf(Leaf::TexLane(_, c)) if c % 2 == 1 => Swz::H11,
                    _ => Swz::H00,
                };
                let s = [operand(&pr, x, &lv, &val)?.swz(half(x)), operand(&pr, y, &lv, &val)?.swz(half(y))];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
            P::Leaf(Leaf::Const(c)) => Src::of(Idx::Const(c)),
            /* the look-up's word swapped: `MKVEC.v2i16 w.h1, w.h0`, which
             * opt.rs `lower_mkvec_swz` makes `IADD.v2u16 r0, r0^.h10, 0x0`
             * (gm2_texswz_3) */
            P::Leaf(Leaf::TexSwapped(..)) => {
                let w = operand(&pr, roots[k], &lv, &val)?;
                let s = [w.swz(Swz::H11), w.swz(Swz::H00)];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
            _ => Src::ssa(alu(p, Op::MovI32, &[operand(&pr, roots[k], &lv, &val)?])?),
        };
    }
    let col = match texel {
        Some(c) => c,
        None => alu(p, Op::Collect, &words)?,
    };
    blend(p, 0, col, t, 2, false, false)?;
    for &(rt, w) in &more[..nmore] {
        let col = alu(p, Op::Collect, &[Src::of(Idx::Const(w[0])), Src::of(Idx::Const(w[1]))])?;
        blend(p, rt, col, t, 2, false, false)?;
    }
    f.rows = rows;
    preloads(p)
}

/// One word of a highp shader's colour, two fp32 lanes as fp16 halves
/// (the conversion `pan_nir_lower_fs_outputs` and `nir_lower_mediump_io`
/// leave at the store, each lane an `f2f16` that `nir_opt_vectorize`
/// pairs when both lanes are components of one value in one aligned pair,
/// `bi_vectorize_filter`'s width 2 on v10, bifrost_nir.c:154-160 and
/// nir_opt_vectorize.c:159-175).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Word {
    /// Two constant halves (NIR folds the conversion).
    Const(u32),
    /// Two lanes of one load in one aligned pair (lanes `2h` and `2h + 1`
    /// of a varying's load, or components `2h` and `2h + 1` of a uniform
    /// load, read from the FAU), converted together (`V2F32_TO_V2F16`,
    /// bifrost_compile.c:2771-2777).
    Pair(Val, Val),
    /// Word `h` of look-up `j`'s fp16 result.
    Tex(u8, u8),
    /// That word with its halves swapped (gm2_texswz_4's `.wzyx`).
    TexSwapped(u8, u8),
    /// Each lane converted alone (`FADD.f32` to a low half,
    /// `bi_f32_to_f16_to`, bifrost_compile.c:257-273), then `MKVEC.v2i16`
    /// of the two halves (`bi_make_vec_to`, :837-847), the second a lane
    /// or a constant half.
    Lanes(Val, Val),
}

/// The load a lane of a highp shader is a lane of, if it is one: (a
/// uniform load, its index, the lane), the index a varying's load's
/// ([`index`]) or the uniform node's, the lane the one the pairing goes
/// by (a varying's place in its compacted load, a uniform's component).
fn load_lane(f: &Fs<'_>, v: Val) -> Result<Option<(bool, u16, u8)>, MaliError> {
    let Val::Lane(n, c) = v else { return Ok(None) };
    Ok(match f.nir.nodes[n as usize].kind {
        Kind::Input { loc, .. } if f.nir.nodes[n as usize].bits == 32 => {
            let (k, l) = index(f, loc, c)?;
            Some((false, u16::from(k), l))
        }
        Kind::Uniform { .. } => {
            let (low, at) = uniform_vector(f, n);
            Some((true, low, at + c))
        }
        _ => None,
    })
}

/// The load uniform node `n` is part of after `nir_opt_load_store_vectorize`
/// (bifrost_nir.c:298-310) and its first lane's place in it. Mesa's loads
/// are at their words of the stage's packed block (select.rs
/// [`UboLayout`], a highp shader's, [`Fs::ubo`]: an unused uniform takes
/// no word). The live loads of `n`'s block are walked in word order
/// (`vectorize_sorted_entries`, nir_opt_load_store_vectorize.c:1674-1711),
/// each run's low load taking the next while `mem_vectorize_cb`
/// (bifrost_nir.c:182-199) allows: no hole, at most four 32-bit lanes,
/// their bytes within the alignment of the low load's constant offset
/// (`calc_alignment`, :723-753: the largest power of two dividing it; 0
/// divides by all). So two scalar uniforms at words 0 and 1 are one vec2
/// load (algfold_negneg.expect.json, `uniform float u_x, u_q, u_a, u_b`
/// with `u_x` unused: `V2F32_TO_V2F16 r0, u2.w0, u2.w1`), at words 1 and
/// 2 two loads. A load outside the layout (a mediump shader's: no
/// [`Fs::ubo`]) stays alone.
fn uniform_vector(f: &Fs<'_>, n: u16) -> (u16, u8) {
    let at = |id: usize| match f.nir.nodes[id].kind {
        Kind::Uniform { dword, comps } if f.nir.live[id] && f.nir.block[id] == f.nir.block[n as usize] => {
            f.ubo.word(dword).map(|w| (w, comps))
        }
        _ => None,
    };
    let Some((word, _)) = at(n as usize) else { return (n, 0) };
    /* the run: its low load, word and lanes so far */
    let mut run = (n, word, 0u16);
    let mut prev: Option<u16> = None;
    loop {
        let next = (0..f.nir.n)
            .filter_map(|id| at(id).map(|(w, c)| (id as u16, w, c)))
            .filter(|&(_, w, _)| prev.is_none_or(|p| w > p))
            .min_by_key(|&(_, w, _)| w);
        let Some((id, w, c)) = next else { return (n, 0) };
        let total = run.2 + u16::from(c);
        let align = if run.1 == 0 { u16::MAX } else { 1 << run.1.trailing_zeros() };
        if run.2 == 0 || w != run.1 + run.2 || total > 4 || total > align {
            run = (id, w, u16::from(c));
        } else {
            run.2 = total;
        }
        if w == word {
            return (run.0, (w - run.1) as u8);
        }
        prev = Some(w);
    }
}

/// `nir_opt_move` with `nir_move_load_input` (bifrost_nir.c:375-381,
/// nir_opt_move.c:46-118) over [`select_highp`]'s events, for the loads
/// read in their own block: walked backwards, each load moves to just
/// before its first user there; a load already right before it stays and
/// keeps its own index, so a later one that shares the user goes after it
/// (the [`super::nir`] `opt_move` of the vertex stage). So the loads of an
/// `ffma(a, b, c)` whose `c` stood right before it come `c, a, b`
/// (measured: varyorder_rev.expect.json, `v_b`, `v_uv`, `v_a` for `v_a *
/// vec4(v_uv, 1.0, 1.0) + v_b`). A phi is no user (:93-94); a load no
/// instruction of its block reads is the tail's (`TAIL_LOAD`).
fn move_loads(f: &Fs<'_>, events: &mut [(u8, u32, u8, u16)], scratch: &Arena<'_>) -> Result<(), MaliError> {
    /* [`select_highp`]'s event kinds */
    const LOAD: u8 = 1;
    const TEX: u8 = 2;
    const NODE: u8 = 4;
    const COND: u8 = 5;
    let n = events.len();
    let index = scratch.alloc_filled(n, 0u32).map_err(|_| MaliError::OutOfScratch)?;
    let reads = |e: (u8, u32, u8, u16), k: u16| -> Result<bool, MaliError> {
        match e.2 {
            TEX => {
                /* its varying's load, or a load lane its computed
                 * coordinate is ([`TexCoord::Computed`]) */
                let Some(t) = f.look_ups().get(e.3 as usize) else { return Ok(false) };
                let src = f.nir.nodes[t.node as usize].src;
                for v in &src[..2] {
                    if load_lane(f, *v)?.is_some_and(|(u, j, _)| !u && j == k) {
                        return Ok(true);
                    }
                }
                Ok(t.load.map(u16::from) == Some(k))
            }
            NODE | COND => {
                let id = f.nir.order[e.3 as usize];
                let nd = f.nir.nodes[id as usize];
                /* a look-up among the nodes ([`multi16`]) at its varying */
                if nd.kind == Kind::Tex && multi16(f) {
                    let t = f.look_up_of(id).map(|j| f.look_ups[j]);
                    return Ok(t.is_some_and(|t| t.load.map(u16::from) == Some(k)));
                }
                let Kind::Alu(op) = nd.kind else { return Ok(false) };
                for v in &nd.src[..op.arity()] {
                    if load_lane(f, *v)?.is_some_and(|(u, j, _)| !u && j == k) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    };
    let mut next = 1u32;
    let mut pos = n;
    while pos > 0 {
        pos -= 1;
        index[pos] = next;
        next += 1;
        let e = events[pos];
        if e.2 != LOAD {
            continue;
        }
        /* the first user: the earliest one after it in its block (the
         * indices fall along the events, so it has the largest) */
        let mut first = None;
        for (q, &u) in events.iter().enumerate().skip(pos + 1) {
            if u.0 != e.0 {
                break;
            }
            if reads(u, e.3)? {
                first = Some(q);
                break;
            }
        }
        let Some(mut at) = first else { continue };
        let fi = index[at];
        while at > pos + 1 && index[at - 1] == fi {
            at -= 1;
        }
        if at == pos + 1 {
            continue;
        }
        /* out of `pos`, in before `at` */
        events[pos..at].rotate_left(1);
        index[pos..at].rotate_left(1);
        index[at - 1] = fi;
    }
    Ok(())
}

/// The colour word of lanes `a` and `b` (a pair of the colour).
fn word_of(f: &Fs<'_>, a: Val, b: Val) -> Result<Word, MaliError> {
    let half = |bits: u32| f16_exact(bits).ok_or(MaliError::Fp16Inexact);
    /* the fp16 look-up's words are the colour's; an fp32 texel is a lane
     * like any other ([`Fs::look_up`]) */
    let f16 = |v: Val| f.texel(v).is_some_and(|(j, _)| f.tex_f16(j));
    if f16(a) || f16(b) {
        return match (f.texel(a), f.texel(b)) {
            (Some((t, i)), Some((u, j))) if t == u && i % 2 == 0 && j == i + 1 => Ok(Word::Tex(t as u8, i / 2)),
            (Some((t, i)), Some((u, j))) if t == u && j % 2 == 0 && i == j + 1 && f16(a) => {
                Ok(Word::TexSwapped(t as u8, j / 2))
            }
            /* the texels of two 2D array look-ups: their halves made one
             * word (case 42's `vec4(t.rgb, f.a)`, `MKVEC.v2i16 r3^.h0,
             * r1.h0`) */
            (Some((t, _)), Some((u, _))) if f.look_ups[t].array && f.look_ups[u].array => Ok(Word::Lanes(a, b)),
            _ => Err(MaliError::TexLanes),
        };
    }
    Ok(match (a, b) {
        (Val::Const(x), Val::Const(y)) => Word::Const(half(x)? as u32 | (half(y)? as u32) << 16),
        /* a mediump colour of masks ([`mask_colour`]): a constant, then a
         * select's half, `MKVEC.v2i16 u2.w1.h0, r0^.h0` with the constant
         * a FAU word (intbig_case72.expect.json, red the folded loop) */
        (Val::Const(_), _) if !f.prec.highp => Word::Lanes(a, b),
        (Val::Const(_), _) => return Err(MaliError::ColourPair),
        /* a lane and 0.0: `MKVEC.v2i16 x, #0`, which bi_lower_mkvec_swz makes
         * `U16_TO_U32 x` (opt.rs; every s4_* reference draws it) */
        (_, Val::Const(y)) if half(y)? == 0 => Word::Lanes(a, b),
        /* one conversion of a lane twice: its half in both lanes of the
         * word, `MKVEC.v2i16 x.h0, x.h0` (varyconst_185_kindcost_fmul:
         * `IADD.v2u16 r1, r0.h00, 0x0`, opt.rs `lower_mkvec_swz`) */
        _ if a == b => Word::Lanes(a, b),
        _ => match (load_lane(f, a)?, load_lane(f, b)?) {
            (Some((u, k, x)), Some((u1, k1, y))) if u == u1 && k == k1 => {
                if x % 2 == 0 && y == x + 1 {
                    Word::Pair(a, b)
                } else if x / 2 == y / 2 && !u {
                    /* one aligned pair of a varying's load the other way
                     * round: vectorised too, the sources in the colour's
                     * order (gm2_fclane_1's `vec4(vd.g, vd.r, ..)`:
                     * `V2F32_TO_V2F16.discard r0, r1^, r0^`) */
                    Word::Pair(a, b)
                } else if x / 2 == y / 2 {
                    /* a uniform's pair the other way round: not drawn by a
                     * reference */
                    return Err(MaliError::ColourForm);
                } else {
                    Word::Lanes(a, b)
                }
            }
            _ => Word::Lanes(a, b),
        },
    })
}

/// The fp32 source of lane `v` of a highp shader: a constant, a load's
/// lane, a uniform's word of the FAU (`bi_opt_push_ubo`'s move, which
/// copy propagation folds into the reader), an operation's result.
fn src32(f: &Fs<'_>, lv: &Leaves, val: &[Option<Src>], v: Val) -> Result<Src, MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    match v {
        Val::Const(c) => Ok(Src::of(Idx::Const(c))),
        Val::Lane(n, c) => match f.nir.nodes[n as usize].kind {
            Kind::Input { loc, .. } => {
                let (k, l) = index(f, loc, c)?;
                lv.raw[k as usize][l as usize & 3].map(Src::ssa).ok_or(defect)
            }
            /* a 16-bit value's half: the low one of a scalar (a narrowing's
             * `FADD.f32 rN.h0`, a compare's mask), lane `c`'s of a vector of
             * two ([`Fs::alu16`]); a vector's user that reads it whole sets
             * `.h01` ([`select_highp`]) */
            Kind::Alu(_) if f.nir.nodes[n as usize].bits == 16 => {
                let s = val.get(n as usize).copied().flatten().ok_or(defect)?;
                Ok(s.swz(if f.nir.nodes[n as usize].width == 2 && c == 1 { Swz::H11 } else { Swz::H00 }))
            }
            /* a phi of fp16 values holds its value in the low half (a
             * texel's copy, a sum; [`kept16`]) */
            Kind::Phi if f.nir.nodes[n as usize].bits == 16 => {
                Ok(val.get(n as usize).copied().flatten().ok_or(defect)?.swz(Swz::H00))
            }
            Kind::Alu(_) | Kind::Phi => val.get(n as usize).copied().flatten().ok_or(defect),
            /* a vec4 array read's loaded lane, a float array read's loaded
             * word (`select_highp`'s `UBO`) */
            Kind::UniformIdx { comps: 4, .. } if f.nir.nodes[n as usize].bits == 32 => {
                let k = f.ubo_idx[..f.nubo].iter().position(|&u| u == n).ok_or(defect)?;
                lv.ubo_lanes[k][c as usize & 3].map(Src::ssa).ok_or(defect)
            }
            Kind::UniformIdx { .. } if f.nir.nodes[n as usize].bits == 32 => {
                val.get(n as usize).copied().flatten().ok_or(defect)
            }
            /* an fp16 uniform's half (a mediump shader's colour of masks,
             * [`mask_colour`]: case 72's `F16_TO_S32.rtz r0, u2.w0.h0`), as
             * the pair path reads it ([`Leaf::Uniform`]) */
            Kind::Uniform { dword, .. } if f.nir.nodes[n as usize].bits == 16 => {
                let (w, half, base) = lv.ubo.half(dword).ok_or(MaliError::FsUniform)?;
                /* a pair of lanes ([`vectorise_int_sources`]) is one word */
                if f.nir.nodes[n as usize].width == 2 && half != 0 {
                    return Err(MaliError::FsUniform);
                }
                let swz = if half == 0 { Swz::H00 } else { Swz::H11 };
                Ok(Src::of(Idx::Ubo { base, comp: (w - base) as u8 }).swz(swz))
            }
            Kind::Uniform { dword, .. } => {
                let base = lv.ubo.word(dword).ok_or(defect)?;
                Ok(Src::of(Idx::Ubo { base, comp: c }))
            }
            /* a preloaded word ([`Kind::Preload`]), its `MOV.i32` made by
             * select.rs `preloads` */
            Kind::Preload { reg } => Ok(Src::of(Idx::Reg(reg))),
            /* the preloaded position's half (`bi_preload`, select.rs
             * `preloads`): x low, y high */
            Kind::PixelCoord => {
                Ok(Src::of(Idx::Reg(super::ra::POSITION_XY)).swz(if c == 0 { Swz::H00 } else { Swz::H11 }))
            }
            /* a texel of the fp32 look-up: its own word ([`tex`]) */
            Kind::Tex if f.nir.nodes[n as usize].bits == 32 => {
                let j = f.look_up_of(n).ok_or(defect)?;
                lv.tex.get(j).and_then(|w| w[c as usize & 3]).map(Src::ssa).ok_or(defect)
            }
            /* an fp16 texel: its half of the register the write mask packs
             * it in ([`tex_mask`]; a 2D array look-up's, case 42, and a
             * look-up's under an `if` Mesa keeps, cflow_148_texbranch) */
            Kind::Tex if f.look_up_of(n).is_some_and(|j| f.tex_f16(j)) => {
                let j = f.look_up_of(n).ok_or(defect)?;
                let mask = tex_mask(&f.look_ups[j], true);
                let at = (mask & ((1u8 << c) - 1)).count_ones() as usize;
                let w = lv.tex.get(j).and_then(|w| w[(at / 2) & 3]).ok_or(defect)?;
                Ok(Src::ssa(w).swz(if at.is_multiple_of(2) { Swz::H00 } else { Swz::H11 }))
            }
            /* a descriptor word ([`tex_desc`]): the first eight bytes'
             * two, by the size query's place; a 16-bit field's own value */
            Kind::TexDesc { byte: 0, .. } => {
                let k = f.size_of(n).ok_or(defect)?;
                lv.desc.get(k).and_then(|d| d[c as usize & 1]).map(Src::ssa).ok_or(defect)
            }
            Kind::TexDesc { .. } => val.get(n as usize).copied().flatten().ok_or(defect),
            _ => Err(MaliError::MixedPrecision),
        },
        /* the halves of a pair's source ([`vectorize16`]): an fp16 value's
         * word, or a uniform's word of the fp16 block */
        Val::Pair(n, swz) => match f.nir.nodes[n as usize] {
            NirNode { kind: Kind::Uniform { dword, .. }, bits: 16, .. } => {
                let (w, _, base) = lv.ubo.half(dword).ok_or(MaliError::FsUniform)?;
                Ok(Src::of(Idx::Ubo { base, comp: (w - base) as u8 }).swz(swz))
            }
            NirNode { kind: Kind::Alu(_), bits: 16, .. } => {
                val.get(n as usize).copied().flatten().map(|s| s.swz(swz)).ok_or(defect)
            }
            _ => Err(defect),
        },
        _ => Err(defect),
    }
}

/// The mediump shader `f` computes its colour from 32-bit integer masks and
/// constants alone (facing.frag: selects on gl_FrontFacing): no fp16 value,
/// no load. NIR then holds what a highp shader's holds, the selects of
/// constants narrowed at the store (`f2f16(bcsel(c, a, b))` folded to a
/// 16-bit select, nir_opt_constant_folding.c), so the program is
/// [`select_highp`]'s (builtin_49.expect.json).
///
/// The masks may also be integer compares of uniforms (case 72: `int(u_in.x)
/// == 2`, `F16_TO_S32` of the uniform's fp16 half) or a bool uniform's word
/// itself (case 112): uniform words read by the selects, no load either,
/// the colour still constants and selects of constants
/// (intbig_case72/112.expect.json).
///
/// The integers may also be `F16_TO_S32` of fp16 products of varying lanes
/// (int_compare's `int(3.0 * v_color.r)` and `.g`), when those pair
/// ([`int_source_pairs`]).
///
/// Or the colour is fp32 values of integers narrowed at the store (case
/// 79: `float(q.x) / 4.0`, the quotients of `nir_lower_idiv`'s sequence,
/// [`Fs::idiv`]), the integers `F16_TO_S32` of fp16 uniform lanes (`u_d.xy
/// * 10.0` and `abs(u_d.zw)` in pairs, `abs` of one such lane alone).
pub fn mask_colour(f: &Fs<'_>) -> bool {
    let int = |op: Alu| matches!(op, Alu::Icmp(_) | Alu::B2f | Alu::Bcsel | Alu::F2i32 | Alu::F2i | Alu::Inot);
    let nodes = &f.nir.nodes[..f.nir.n];
    let fp32 = |v: &Val| f.fp32_lane(*v);
    let lane = |v: &Val| match *v {
        Val::Const(_) => true,
        Val::Lane(n, _) => matches!(nodes[n as usize], NirNode { kind: Kind::Alu(op), bits: 32, .. } if int(op)),
        _ => false,
    };
    let pairs = int_source_pairs(f);
    let paired = |id: usize| {
        pairs.iter().flatten().flatten().any(|&(a, b)| a as usize == id || b as usize == id) || lone_abs(f, id)
    };
    /* the integer arithmetic of a division, and its fp32 results */
    let arith = |op: Alu| op.is_int() || matches!(op, Alu::I2f | Alu::U2f | Alu::Frcp | Alu::Fmul | Alu::Fadd);
    let divides = f.colour.is_some_and(|c| c.iter().any(fp32));
    let masks = masks16(f);
    f.colour.is_some_and(|c| c.iter().all(|v| lane(v) || fp32(v)) && !c.iter().all(|v| matches!(v, Val::Const(_))))
        && pairs.is_some()
        && nodes.iter().enumerate().all(|(id, nd)| match nd.kind {
            Kind::Preload { .. } | Kind::Uniform { .. } => true,
            Kind::Input { loc, .. } => (loc as usize) < MAX_VARYINGS && nd.bits == 32,
            Kind::Alu(Alu::Fcmp(_) | Alu::Ior | Alu::Iand) if nd.bits == 16 && masks => true,
            Kind::Alu(op) => (nd.bits == 32 && (int(op) || (divides && arith(op)))) || paired(id),
            _ => false,
        })
}

/// The mediump shader `f` reduces fp16 compares' masks with `any` or `all`
/// ([`Fs::mask_op`]): 16-bit `ior` / `iand` nodes, which [`select_highp`]
/// selects as `LSHIFT_OR.v2i16` / `LSHIFT_AND.v2i16` after the compares
/// pair (intbool_76_boolany).
pub fn masks16(f: &Fs<'_>) -> bool {
    !f.prec.highp
        && f.nir.nodes[..f.nir.n]
            .iter()
            .any(|nd| matches!(nd, NirNode { kind: Kind::Alu(Alu::Ior | Alu::Iand), bits: 16, .. }))
}

/// The mediump shader `f` makes a 2D array look-up ([`TexLookup::array`]):
/// its coordinates are fp32 and integer values of fp16 ones (case 42's
/// `F16_TO_F32` of `uv`, `F32_TO_S32` of `uv.x * float(sz.x)`), NIR's at
/// 32 bits as a highp shader's, so the program is [`select_highp`]'s, the
/// fp16 values read by those conversions in pairs ([`int_source_pairs`]),
/// the colour the look-ups' fp16 texels.
pub fn array_look_ups(f: &Fs<'_>) -> bool {
    !f.prec.highp && f.live_look_ups().any(|(_, t)| t.array)
}

/// The mediump shader `f` unrolled a loop whose test and run-time `break`
/// are one terminator (`complex_unroll_single_terminator`, [`walk_guard`],
/// [`merged_bound`]): the guards are regions of the graph, which
/// [`select_highp`] flattens as it does a highp shader's
/// (cflow_71_deadloop), its fp16 lanes then paired ([`vectorize16`]).
pub fn guarded16(f: &Fs<'_>) -> bool {
    !f.prec.highp && f.nir.ifs[..f.nir.nifs].iter().any(|r| r.single)
}

/// The mediump ES 3.00 shader `f` reads the highp gl_FragCoord ([`Fs::fc32`]):
/// its island of fp32 operations, their narrowings and the fp16 vectors on
/// those are a highp shader's graph with fp16 islands the other way round,
/// so the program is [`select_highp`]'s (case 218 shadowcmp,
/// fsarith_218_shadowcmp.expect.json; docs/notes/mali-fragcoord-fp16-ops.md).
pub fn frag32(f: &Fs<'_>) -> bool {
    !f.prec.highp && (f.prec.frag_coord_highp || f.fc_copy) && f.frag_xy.is_some()
}

/// The mediump shader `f` keeps an `if` as blocks with a branch: a look-up
/// under it, which `nir_opt_peephole_select` may not speculate
/// (`block_check_for_allowed_instrs`, nir_opt_peephole_select.c:319), so
/// the `if` is a region of the graph ([`walk_if`]) that [`select_highp`]
/// lays out as a highp shader's (cflow_148_texbranch: the two `if`s with
/// a look-up in an arm, and the 24 guards of the unrolled loop with two
/// look-ups a pass). Its fp16 lanes pair as a ladder's ([`vectorize16`]).
pub fn kept16(f: &Fs<'_>) -> bool {
    /* a `textureSize`'s null test ([`Fs::tex_size`]) is Mesa's own `if`,
     * which the peephole flattens unless the side loads (case 42: a `CSEL`
     * pair; a mediump one kept is refused after the peephole) */
    !f.prec.highp && f.nir.live_ifs().any(|r| !r.single && !r.inot)
}

/// TEX_MULTI (plan #367 lower-fma, docs/notes/mali-tex-multi.md): the
/// mediump shader `f` makes several 2D look-ups, at least one at a computed
/// coordinate, in straight-line code. Its program is [`select_highp`]'s:
/// the varying lanes converted ([`convert_inputs16`]), each computed
/// coordinate the look-up's own operation of two lanes ([`pair_coords`]),
/// the look-ups among the nodes in program order, the colour's fp16 lanes
/// paired ([`vectorize16`]) and each look-up's texels read as its words.
/// Only the forms measured against Mesa compile ([`multi_forms`]).
pub fn multi16(f: &Fs<'_>) -> bool {
    let plain = |t: &TexLookup| {
        t.cube.is_none() && !t.array && !t.d3 && !t.fetch && !t.shadow && !t.zero && t.bias == Val::Undef
    };
    !f.prec.highp
        && f.live_look_ups().count() > 1
        && f.live_look_ups().any(|(_, t)| t.coord == TexCoord::Computed)
        && f.live_look_ups().all(|(_, t)| plain(t))
        && f.nir.nifs == 0
        && f.nir.nloops == 0
        && f.discards == 0
}

/// The pair of fp16 lanes `(a, b)` as one source of a two-lane node: two
/// constants one word of halves, the two lanes of an aligned pair of one
/// value that value's word, two one-lane operations alike a new operation
/// of two lanes over their sources' pairs, inserted into the order before
/// position `at` (returned moved past the insertions). Any other pair:
/// None.
fn coord_pair(
    f: &mut Fs<'_>,
    a: Val,
    b: Val,
    at: &mut usize,
    origin: u16,
    depth: u8,
) -> Result<Option<Val>, MaliError> {
    let half = |v: u32| f16_rtne(v).map(u32::from).ok_or(MaliError::Fp16Inexact);
    match (a, b) {
        (Val::Const(p), Val::Const(q)) => return Ok(Some(Val::Const(half(p)? | half(q)? << 16))),
        (Val::Lane(n, c), Val::Lane(m, d)) if n == m && c % 2 == 0 && d == c + 1 => {
            let nd = f.nir.nodes[n as usize];
            return Ok(match (nd.kind, nd.bits, nd.width) {
                (Kind::Alu(_), 16, 2) if c == 0 => Some(Val::Pair(n, Swz::H01)),
                _ => None,
            });
        }
        _ => {}
    }
    let (Val::Lane(n, 0), Val::Lane(m, 0)) = (a, b) else { return Ok(None) };
    let (x, y) = (f.nir.nodes[n as usize], f.nir.nodes[m as usize]);
    let (Kind::Alu(op), 16, 1) = (x.kind, x.bits, x.width) else { return Ok(None) };
    if y.kind != x.kind || y.bits != 16 || y.width != 1 || depth > 8 {
        return Ok(None);
    }
    if !matches!(op, Alu::Fadd | Alu::Fmul | Alu::Ffma | Alu::Fneg | Alu::Fabs) {
        return Ok(None);
    }
    let mut src = [Val::Undef; 3];
    for k in 0..op.arity() {
        match coord_pair(f, x.src[k], y.src[k], at, origin, depth + 1)? {
            Some(v) => src[k] = v,
            None => return Ok(None),
        }
    }
    let id = f.nir.insert_before(*at, NirNode { kind: x.kind, src, bits: 16, width: 2 })?;
    f.origin[id as usize] = origin;
    *at += 1;
    Ok(Some(Val::Pair(id, Swz::H01)))
}

/// TEX_MULTI's coordinates ([`multi16`]): each computed coordinate of two
/// fp16 lanes is the look-up's own operation of two lanes, as NIR keeps the
/// GLSL vector operation (`nir_lower_alu_width` leaves a 16-bit vec2,
/// `bi_vectorize_filter`), not lanes the walk's CSE shares between look-ups
/// (gm2_texmulti_2's `v_uv + vec2(-0.25, 0.0)` and `v_uv + vec2(0.25,
/// 0.0)`: two `FADD_IMM.v2f16` of the converted pair, each `y + 0.0` its
/// own). A coordinate whose lanes do not pair has no reference and is
/// refused as before ([`MaliError::TexCount`]).
fn pair_coords(f: &mut Fs<'_>) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    /* a varying's conversion ([`convert_inputs16`]) stands at its first
     * reader: GLSL's lower_precision puts the `f2fmp` in the expression
     * that reads the lane, so a look-up at the varying itself before it
     * reads the load as it is first (gm2_texmulti_1: `TEX_SINGLE ...
     * @r0:r1`, then `V2F32_TO_V2F16 r0, r0, r1`) */
    let mut k = 0;
    while k < f.nir.len {
        let id = f.nir.order[k];
        let nd = f.nir.nodes[id as usize];
        let input = matches!(nd.src[0], Val::Lane(n, _) if matches!(f.nir.nodes[n as usize].kind, Kind::Input { .. }));
        if nd.kind != Kind::Alu(Alu::F2f16) || nd.bits != 16 || !input {
            k += 1;
            continue;
        }
        let first = f.nir.uses(id).filter_map(|(u, _)| f.nir.order[..f.nir.len].iter().position(|&o| o == u)).min();
        match first {
            Some(to) if to > k + 1 => {
                f.nir.order.copy_within(k + 1..to, k);
                f.nir.order[to - 1] = id;
                f.origin[id as usize] = f.origin[f.nir.order[to] as usize];
            }
            _ => k += 1,
        }
    }
    for j in 0..f.nlook {
        let t = f.look_ups[j];
        if !t.live() || t.coord != TexCoord::Computed {
            continue;
        }
        let nd = f.nir.nodes[t.node as usize];
        let mut at = f.nir.order[..f.nir.len].iter().position(|&x| x == t.node).ok_or(defect)?;
        let origin = [nd.src[0], nd.src[1]]
            .iter()
            .filter_map(|v| match *v {
                Val::Lane(n, _) => Some(f.origin[n as usize]),
                _ => None,
            })
            .max()
            .unwrap_or(f.origin[t.node as usize]);
        let Some(Val::Pair(id, _)) = coord_pair(f, nd.src[0], nd.src[1], &mut at, origin, 0)? else {
            return Err(MaliError::TexCount);
        };
        f.nir.nodes[t.node as usize].src[..2].copy_from_slice(&[Val::Lane(id, 0), Val::Lane(id, 1)]);
    }
    Ok(())
}

/// The forms of TEX_MULTI ([`multi16`]) measured byte-equal to Mesa 26.2.1's
/// drm-shim (tools/mali/testdata/gm2_texmulti_1..3), checked once
/// [`pair_coords`] made the coordinates pairs: each computed coordinate
/// the converted vec2 varying plus a constant pair (`FADD_IMM.v2f16`), the
/// colour sums and products of texels and constants. A coordinate of
/// another form (a uniform step, gm2_texmulti_4; one computed from the
/// varying, terrain's tilt-shift, gm2_texmulti_5 and _6), and any other
/// operation, uniform or value, is refused as before
/// ([`MaliError::TexCount`]); a product of a constant product with another
/// constant is the constant-product gap ([`MaliError::ArithConstant`]).
fn multi_forms(f: &Fs<'_>) -> Result<(), MaliError> {
    let node = |v: Val| match v {
        Val::Lane(n, _) | Val::Pair(n, _) => Some(f.nir.nodes[n as usize]),
        _ => None,
    };
    let conv = |v: Val| {
        matches!(node(v), Some(NirNode { kind: Kind::Alu(Alu::F2f16), bits: 16, width: 2, src, .. })
            if matches!(node(src[0]), Some(NirNode { kind: Kind::Input { .. }, .. })))
    };
    /* a colour operand: a constant, a texel, or a colour operation */
    let colour = |v: Val| match node(v) {
        None => matches!(v, Val::Const(_)),
        Some(NirNode { kind: Kind::Tex, .. }) => true,
        Some(NirNode { kind: Kind::Alu(Alu::Fadd | Alu::Fmul), bits: 16, src, .. }) => !conv(src[0]),
        _ => false,
    };
    for &id in &f.nir.order[..f.nir.len] {
        let nd = f.nir.nodes[id as usize];
        let read_by_tex = f.nir.uses(id).any(|(u, _)| f.nir.nodes[u as usize].kind == Kind::Tex);
        let ok = match nd.kind {
            Kind::Input { .. } | Kind::Store { .. } | Kind::Tex => true,
            Kind::Alu(Alu::F2f16) => conv(Val::Lane(id, 0)),
            /* a coordinate: `FADD_IMM.v2f16` of the converted pair */
            Kind::Alu(Alu::Fadd) if read_by_tex => {
                nd.bits == 16
                    && nd.width == 2
                    && conv(nd.src[0])
                    && matches!(nd.src[0], Val::Pair(_, Swz::H01))
                    && matches!(nd.src[1], Val::Const(_))
            }
            /* a product of a product with a constant and another constant
             * (gm2_constfold_4's `t * Kernel1 * Kernel1`): Mesa folds the
             * two constants where the walk keeps both products when their
             * product is inexact in fp16 ([`Fs::op`]), a constant-product
             * gap, not this family's */
            Kind::Alu(Alu::Fmul) if nd.src[..2].iter().any(|&v| matches!(v, Val::Const(_))) => {
                let inner = |v: Val| {
                    matches!(node(v), Some(NirNode { kind: Kind::Alu(Alu::Fmul), src, .. })
                        if src[..2].iter().any(|s| matches!(s, Val::Const(_))))
                };
                if nd.src[..2].iter().any(|&v| inner(v)) {
                    return Err(MaliError::ArithConstant);
                }
                nd.bits == 16 && nd.src[..2].iter().all(|&v| colour(v))
            }
            Kind::Alu(Alu::Fadd | Alu::Fmul) => nd.bits == 16 && nd.src[..2].iter().all(|&v| colour(v)),
            _ => false,
        };
        if !ok {
            return Err(MaliError::TexCount);
        }
    }
    Ok(())
}

/// The forms [`kept16`]'s route transcribes (cflow_148_texbranch): fp16
/// sums, products and compares over uniform lanes, constants, texels and
/// each other, the negation of one, the look-ups' copies and the joins'
/// phis, and look-ups of a 2D texture at a constant coordinate or one
/// computed from those. A varying, a discard, an integer or a select has no
/// reference under a kept branch ([`MaliError::ControlFlow`]).
fn branch_forms(f: &Fs<'_>) -> Result<(), MaliError> {
    for &id in &f.nir.order[..f.nir.len] {
        let nd = f.nir.nodes[id as usize];
        let ok = match nd.kind {
            Kind::Alu(Alu::Fadd | Alu::Fmul | Alu::Ffma | Alu::Fneg | Alu::Fcmp(_) | Alu::Mov) => nd.bits == 16,
            Kind::Uniform { .. } | Kind::Phi => nd.bits == 16,
            Kind::Store { .. } => true,
            Kind::Tex => {
                let plain = |t: &TexLookup| t.cube.is_none() && !t.array && !t.fetch;
                nd.bits == 16
                    && f.look_up_of(id)
                        .is_some_and(|j| plain(&f.look_ups[j]) && !matches!(f.look_ups[j].coord, TexCoord::Varying(_)))
            }
            _ => false,
        };
        if !ok {
            return Err(MaliError::ControlFlow);
        }
    }
    Ok(())
}

/// A look-up's coordinate whose two lanes are one fp16 value (`vec2(0.5) -
/// i * vec2(u.w)`: the walk's CSE made one scalar node of the two equal
/// lanes) is NIR's operation of two lanes: `nir_lower_alu_width` keeps a
/// 16-bit vector of two (`bi_vectorize_filter`, bifrost_nir.c:176-179),
/// and `nir_opt_shrink_vectors` does not merge equal lanes. The node and
/// every fp16 operation under it become nodes of two lanes, each source
/// the value's halves ([`Val::Pair`]) or a constant pair, and the look-up
/// reads lane 0 and lane 1 (cflow_148_texbranch: `FADD.v2f16 r3,
/// 0x38000000.h11, u2.w1.h11`, then `F16_TO_F32` of `.h1` and `.h0`). So
/// widened before [`vectorize16`], which pairs one-lane nodes only
/// (`nir_opt_vectorize` leaves a vector alone): the passes' products
/// `k * u.w` stay each pass's own.
fn revector_coords(f: &mut Fs<'_>) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let half = |b: u32| f16_rtne(b).map(u32::from).ok_or(MaliError::Fp16Inexact);
    let mut stack = [0u16; MAX_LOOK_UPS];
    let mut sp = 0;
    for j in 0..f.nlook {
        let t = f.look_ups[j];
        if !t.live() || t.coord != TexCoord::Computed || t.cube.is_some() || t.array {
            continue;
        }
        let nd = f.nir.nodes[t.node as usize];
        let Val::Lane(n, 0) = nd.src[0] else { continue };
        if nd.src[1] != nd.src[0] || !f.is_f16(nd.src[0]) || f.nir.nodes[n as usize].width != 1 {
            continue;
        }
        f.nir.nodes[t.node as usize].src[1] = Val::Lane(n, 1);
        if sp == stack.len() {
            return Err(MaliError::TooManyValues);
        }
        stack[sp] = n;
        sp += 1;
    }
    while sp > 0 {
        sp -= 1;
        let n = stack[sp] as usize;
        let nd = f.nir.nodes[n];
        let (Kind::Alu(op), 16, 1) = (nd.kind, nd.bits, nd.width) else { continue };
        let mut src = [Val::Undef; 3];
        for s in 0..op.arity() {
            src[s] = match nd.src[s] {
                Val::Const(k) => Val::Const(half(k)? | half(k)? << 16),
                /* an fp16 operation under it is widened too (once: a
                 * product both coordinates of a pass read), and read whole;
                 * a leaf (a uniform lane, a texel) is its half in both lanes */
                v @ Val::Lane(m, _) => match f.nir.nodes[m as usize] {
                    NirNode { kind: Kind::Alu(_), bits: 16, width: 1, .. } => {
                        if sp == stack.len() {
                            return Err(MaliError::TooManyValues);
                        }
                        stack[sp] = m;
                        sp += 1;
                        Val::Pair(m, Swz::H01)
                    }
                    NirNode { kind: Kind::Alu(_), bits: 16, width: 2, .. } => Val::Pair(m, Swz::H01),
                    _ => {
                        let h = lane_half(f, v).ok_or(defect)?;
                        Val::Pair(m, if h == 0 { Swz::H00 } else { Swz::H11 })
                    }
                },
                _ => return Err(defect),
            };
        }
        f.nir.nodes[n] = NirNode { kind: nd.kind, src, bits: 16, width: 2 };
    }
    Ok(())
}

/// The forms [`guarded16`]'s route transcribes (cflow_71_deadloop): fp16
/// sums, products, `fceil`, `fmin`, compares and the selects of the
/// flattening, over uniform lanes, constants and each other. Mesa folds a
/// constant into a product or a sum that already has one
/// (nir_opt_algebraic.py:2406-2411). [`Fs::op`] folds the product (`(acc *
/// 2.0) * 0.5` is `acc`, case 70's loopcarry,
/// fsarith_70_loopcarry.expect.json), so a product's chain is left here
/// only as a guard; the sum's has no reference and is refused
/// ([`MaliError::ArithConstant`]), and any other operation has no
/// reference here ([`MaliError::Arithmetic`]).
fn ladder_forms(f: &Fs<'_>) -> Result<(), MaliError> {
    let with_const = |op: Alu, v: Val| match v {
        Val::Lane(n, _) => matches!(
            f.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(o), bits: 16, src, .. }
                if o == op && src[..2].iter().any(|s| matches!(s, Val::Const(_)))
        ),
        _ => false,
    };
    for &id in &f.nir.order[..f.nir.len] {
        let nd = f.nir.nodes[id as usize];
        match nd.kind {
            Kind::Alu(op @ (Alu::Fadd | Alu::Fmul)) if nd.bits == 16 => {
                let k = nd.src[..2].iter().position(|s| matches!(s, Val::Const(_)));
                if k.is_some_and(|k| with_const(op, nd.src[1 - k])) {
                    return Err(MaliError::ArithConstant);
                }
            }
            Kind::Alu(Alu::Fceil | Alu::Fmin | Alu::Fcmp(_) | Alu::Bcsel) if nd.bits == 16 => {}
            Kind::Uniform { .. } | Kind::Phi | Kind::Store { .. } => {}
            _ => return Err(MaliError::Arithmetic),
        }
    }
    Ok(())
}

/// What `nir_opt_vectorize` compares of an fp16 lane `v` read by an
/// operation (`alu_srcs_equal`, nir_opt_vectorize.c:166-175): a constant,
/// or the value it is a lane of and that lane's aligned pair. A uniform's
/// component is a lane of its load (its uniform's first word in the fp16
/// block), the pair its word.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LaneKey {
    Const,
    Def(u32, u16),
}

fn lane_key(f: &Fs<'_>, v: Val) -> Option<LaneKey> {
    match v {
        Val::Const(_) => Some(LaneKey::Const),
        Val::Lane(n, c) => match f.nir.nodes[n as usize] {
            NirNode { kind: Kind::Uniform { dword, .. }, bits: 16, .. } => {
                let (w, _, base) = f.ubo.half(dword)?;
                Some(LaneKey::Def(1 << 16 | u32::from(base), w))
            }
            NirNode { bits: 16, .. } => Some(LaneKey::Def(u32::from(n), u16::from(c / 2))),
            _ => None,
        },
        _ => None,
    }
}

/// The half lane `v` of [`lane_key`] stands in: a uniform's in its word, a
/// lane's in its value's.
fn lane_half(f: &Fs<'_>, v: Val) -> Option<u8> {
    match v {
        Val::Lane(n, c) => match f.nir.nodes[n as usize].kind {
            Kind::Uniform { dword, .. } => f.ubo.half(dword).map(|(_, h, _)| h),
            _ => Some(c % 2),
        },
        _ => None,
    }
}

/// `nir_opt_vectorize` (nir_opt_vectorize.c:650-710) with
/// `bi_vectorize_filter`'s width 2 at 16 bits (bifrost_nir.c:106-179) over
/// the fp16 operations of a mediump shader [`select_highp`] flattens
/// ([`guarded16`]): in program order, each one-lane operation looks for
/// an earlier one left unpaired whose operation is the same and whose every
/// source is a lane of the same value in the same aligned pair, or a
/// constant where the other's is one (`instrs_equal`, :226-272); found,
/// the two are one operation of two lanes where the earlier stood
/// (`instr_try_combine_alu`, :548-612), the earlier's readers reading its
/// lane 0 and the later's its lane 1 (`rewrite_uses`, :333-391). A source
/// of the pair is the value's halves through a swizzle ([`Val::Pair`]),
/// two constants one word of two fp16 halves. Every constant of an fp16
/// node is its fp16 value after (`_mesa_float_to_half`). Measured:
/// cflow_71_deadloop's `FMA.v2f16 r0, u2.w0, u2.w1, ..` of `u_k.x * 3.0`
/// and `u_k.y * 2.0`, and its compares of passes 0 and 1, 2 and 3, 4 and
/// 5, in that order (`FCMP_OR.v2f16.lt.m1 r3, u4.w0, r1.h00, 0x0`).
fn vectorize16(f: &mut Fs<'_>) -> Result<(), MaliError> {
    let pairable = |f: &Fs<'_>, id: usize| match f.nir.nodes[id] {
        NirNode { kind: Kind::Alu(op), bits: 16, width: 1, src } => {
            !matches!(op, Alu::Vec2 | Alu::F2f16 | Alu::Frcp | Alu::Frsq)
                && src[..op.arity()].iter().all(|&v| lane_key(f, v).is_some())
        }
        _ => false,
    };
    let same = |f: &Fs<'_>, a: usize, b: usize| {
        let (x, y) = (f.nir.nodes[a], f.nir.nodes[b]);
        let Kind::Alu(op) = x.kind else { return false };
        x.kind == y.kind && (0..op.arity()).all(|s| lane_key(f, x.src[s]) == lane_key(f, y.src[s]))
    };
    let half = |b: u32| f16_rtne(b).map(u32::from).ok_or(MaliError::Fp16Inexact);
    let swz = |lo: u8, hi: u8| match (lo, hi) {
        (0, 0) => Swz::H00,
        (1, 1) => Swz::H11,
        (0, _) => Swz::H01,
        _ => Swz::H10,
    };
    let mut pending = [0u16; MAX_NODES];
    let mut np = 0;
    let mut k = 0;
    while k < f.nir.len {
        let b = f.nir.order[k] as usize;
        k += 1;
        if !pairable(f, b) {
            continue;
        }
        let Some(j) = (0..np).find(|&j| same(f, pending[j] as usize, b)) else {
            pending[np] = b as u16;
            np += 1;
            continue;
        };
        let a = pending[j] as usize;
        pending.copy_within(j + 1..np, j);
        np -= 1;
        if !f.nir.dominates(f.nir.block[a], f.nir.block[b]) {
            pending[np] = b as u16;
            np += 1;
            continue;
        }
        let (x, y) = (f.nir.nodes[a], f.nir.nodes[b]);
        let Kind::Alu(op) = x.kind else { return Err(MaliError::Defect(Defect::IrIndex)) };
        let mut src = [Val::Undef; 3];
        for s in 0..op.arity() {
            src[s] = match (x.src[s], y.src[s]) {
                (Val::Const(p), Val::Const(q)) => Val::Const(half(p)? | half(q)? << 16),
                /* two texels of an fp16 look-up's word ([`multi16`]): the
                 * word is the `vec2` of its texels ([`Fs::vec16`]), made
                 * once, right after the look-up, which no instruction
                 * stands for */
                (p @ Val::Lane(n, c), Val::Lane(m, d))
                    if n == m && f.nir.nodes[n as usize].kind == Kind::Tex && c / 2 == d / 2 && multi16(f) =>
                {
                    let (lo, hi) = (lane_half(f, p).unwrap_or(0), d % 2);
                    let w = c / 2 * 2;
                    let v = NirNode {
                        kind: Kind::Alu(Alu::Vec2),
                        src: [Val::Lane(n, w), Val::Lane(n, w + 1), Val::Undef],
                        bits: 16,
                        width: 2,
                    };
                    let id = match (0..f.nir.n).find(|&i| f.nir.nodes[i] == v) {
                        Some(i) => i as u16,
                        None => {
                            let pos = f.nir.order[..f.nir.len]
                                .iter()
                                .position(|&o| o == n)
                                .ok_or(MaliError::Defect(Defect::IrIndex))?;
                            let id = f.nir.insert_before(pos + 1, v)?;
                            f.origin[id as usize] = f.origin[n as usize];
                            if pos + 1 < k {
                                k += 1;
                            }
                            id
                        }
                    };
                    Val::Pair(id, swz(lo, hi))
                }
                (p @ Val::Lane(n, _), q) => {
                    let (lo, hi) = (lane_half(f, p), lane_half(f, q));
                    Val::Pair(n, swz(lo.ok_or(MaliError::Defect(Defect::IrIndex))?, hi.unwrap_or(0)))
                }
                _ => return Err(MaliError::Defect(Defect::IrIndex)),
            };
        }
        f.nir.nodes[a] = NirNode { kind: x.kind, src, bits: 16, width: 2 };
        for id in 0..f.nir.n {
            for s in f.nir.nodes[id].src.iter_mut() {
                if *s == Val::Lane(b as u16, 0) {
                    *s = Val::Lane(a as u16, 1);
                }
            }
        }
        for r in f.nir.ifs[..f.nir.nifs].iter_mut() {
            if r.cond == Val::Lane(b as u16, 0) {
                r.cond = Val::Lane(a as u16, 1);
            }
        }
        f.nir.order.copy_within(k..f.nir.len, k - 1);
        f.nir.len -= 1;
        k -= 1;
    }
    /* the one-lane nodes' constants */
    for id in 0..f.nir.n {
        let nd = f.nir.nodes[id];
        if let (Kind::Alu(op), 16, 1) = (nd.kind, nd.bits, nd.width) {
            for s in 0..op.arity() {
                if let Val::Const(c) = nd.src[s] {
                    f.nir.nodes[id].src[s] = Val::Const(half(c)?);
                }
            }
        }
    }
    Ok(())
}

/// `fmul(a, 2.0)` -> `fadd(a, a)` (bifrost_nir_algebraic.py:31, the late
/// rules, [`nir::late_rules`]) on the fp16 nodes [`vectorize16`] left, whose
/// constants are fp16 halves by then: a pair whose constant is 2.0 in both
/// halves (`ceil(u_k.x * 2.0)` beside `u_k.y * 2.0`: `FADD.v2f16 r0, u2.w0,
/// u2.w0`, tools/mali/testdata/deadloop_mul2), or a lone product.
fn late_rules16(f: &mut Fs<'_>) {
    const F16_TWO: u32 = 0x4000;
    for k in 0..f.nir.len {
        let x = f.nir.order[k] as usize;
        let nd = f.nir.nodes[x];
        let two = match (nd.kind, nd.bits, nd.width) {
            (Kind::Alu(Alu::Fmul), 16, 1) => Val::Const(F16_TWO),
            (Kind::Alu(Alu::Fmul), 16, 2) => Val::Const(F16_TWO | F16_TWO << 16),
            _ => continue,
        };
        if let Some(k) = nd.src[..2].iter().position(|v| *v == two) {
            let a = nd.src[1 - k];
            f.nir.nodes[x] = NirNode { kind: Kind::Alu(Alu::Fadd), src: [a, a, Val::Undef], ..nd };
        }
    }
}

/// The late `fadd(fmul(a, b), c)` -> `ffma(a, b, c)` (nir_opt_algebraic.py:
/// 3816-3820) on the fp16 lanes of a shader with 2D array look-ups
/// ([`array_look_ups`]), before they pair ([`vectorise_int_sources`]):
/// case 42's `v_xy * 0.5 + 0.5`, one `FMA.v2f16` of the converted pair
/// (intbig_case42.expect.json). The product is left to the dead code.
pub fn fuse16(f: &mut Fs<'_>) {
    if !array_look_ups(f) {
        return;
    }
    for x in 0..f.nir.n {
        let NirNode { kind: Kind::Alu(Alu::Fadd), src, bits: 16, width: 1 } = f.nir.nodes[x] else { continue };
        for s in 0..2 {
            let Val::Lane(m, 0) = src[s] else { continue };
            let fm = f.nir.nodes[m as usize];
            if !matches!(fm, NirNode { kind: Kind::Alu(Alu::Fmul), bits: 16, width: 1, .. })
                || f.nir.uses(m).count() != 1
            {
                continue;
            }
            f.nir.nodes[x].kind = Kind::Alu(Alu::Ffma);
            f.nir.nodes[x].src = [fm.src[0], fm.src[1], src[1 - s]];
            break;
        }
    }
}

/// Node `id` is `fabs` of one lane of an fp16 node, read by `f2i32`s
/// alone: what `nir_opt_vectorize` finds no partner for (case 79's
/// `abs(u_d.x * 10.0)`, `FADD.v2f16 r2, r4^.abs.h00` of the product pair).
fn lone_abs(f: &Fs<'_>, id: usize) -> bool {
    let nd = f.nir.nodes[id];
    let source = match nd.src[0] {
        Val::Lane(n, _) => {
            matches!(f.nir.nodes[n as usize], NirNode { kind: Kind::Alu(op), bits: 16, .. } if !op.is_int())
        }
        _ => false,
    };
    matches!(nd, NirNode { kind: Kind::Alu(Alu::Fabs), bits: 16, width: 1, .. })
        && source
        && f.nir
            .uses(id as u16)
            .all(|(u, _)| matches!(f.nir.nodes[u as usize], NirNode { kind: Kind::Alu(Alu::F2i32), bits: 32, .. }))
}

/// The fp16 nodes of a colour of masks ([`mask_colour`]), each the source
/// of `F16_TO_S32`s alone, in pairs: lanes `2p` and `2p + 1` of one varying
/// under the same operation and constants, which `nir_opt_vectorize` makes
/// one `vec2` operation (bi_vectorize_filter's width two at 16 bits) on the
/// pair `nir_lower_mediump_io` converts (intloop_int_compare.expect.json:
/// `V2F32_TO_V2F16 r0, r0^, r1^`, `FMA.v2f16 r0, u2.w0, r0^, 0x0.neg`, then
/// `F16_TO_S32.rtz` of `r0.h0` and `r0^.h1`), or two consecutive dwords of
/// an fp16 uniform, one word (intbig_case79.expect.json: `FMA.v2f16 r4,
/// u2.w0, r4^, 0x0.neg` of `u_d.xy * 10.0`, `FADD.v2f16 r0, u2.w1.abs,
/// 0x0.neg` of `abs(u_d.zw)`). A lone `fabs` of such a lane stays a lane
/// ([`lone_abs`]). Pairs in node order; `None` when a node pairs with none
/// (any other fp16 value: no reference).
fn int_source_pairs(f: &Fs<'_>) -> Option<[Option<(u16, u16)>; 4]> {
    let nodes = &f.nir.nodes[..f.nir.n];
    let mut out = [None; 4];
    let mut no = 0;
    let mut pending: Option<u16> = None;
    let uniform = |n: u16| match nodes[n as usize] {
        NirNode { kind: Kind::Uniform { dword, comps: 1 }, bits: 16, width: 1, .. } => Some(dword),
        _ => None,
    };
    for (id, nd) in nodes.iter().enumerate() {
        let Kind::Alu(op) = nd.kind else { continue };
        if nd.bits != 16 || op.is_int() || lone_abs(f, id) {
            continue;
        }
        /* a product [`fuse16`] fused into its sum reads nothing now */
        if f.nir.uses(id as u16).next().is_none() && array_look_ups(f) {
            continue;
        }
        /* read by `f2i32`s alone (the colour reads no fp16 lane), or by a
         * lone `fabs` of it */
        let readers = f.nir.uses(id as u16).all(|(u, _)| {
            matches!(f.nir.nodes[u as usize], NirNode { kind: Kind::Alu(Alu::F2i32 | Alu::F2f32), bits: 32, .. })
                || lone_abs(f, u as usize)
        });
        if nd.width != 1 || !matches!(op, Alu::Fmul | Alu::Fadd | Alu::Ffma | Alu::Fabs) || !readers {
            return None;
        }
        let Some(a) = pending.take() else {
            pending = Some(id as u16);
            continue;
        };
        let x = nodes[a as usize];
        let lanes = (0..op.arity()).all(|k| match (x.src[k], nd.src[k]) {
            (Val::Const(c), Val::Const(d)) => c == d && f16_exact(c).is_some(),
            (Val::Lane(n, c), Val::Lane(m, d)) if matches!(nodes[n as usize].kind, Kind::Input { .. }) => {
                n == m && c % 2 == 0 && d == c + 1
            }
            (Val::Lane(n, 0), Val::Lane(m, 0)) => {
                matches!((uniform(n), uniform(m)), (Some(p), Some(q)) if q == p + 1)
            }
            _ => false,
        });
        /* one varying or uniform operand, the others constants */
        let one = (0..op.arity()).filter(|&k| matches!(nd.src[k], Val::Lane(..))).count() == 1;
        if x.kind != nd.kind || !lanes || !one || no == out.len() {
            return None;
        }
        out[no] = Some((a, id as u16));
        no += 1;
    }
    pending.is_none().then_some(out)
}

/// [`int_source_pairs`] made `vec2` operations in place: the first node of
/// a varying's pair becomes the conversion of its two lanes (`f2f16` of a
/// `vec2`), the second the operation on it, its constants fp16 (in both
/// halves as a lane's), and the `f2i32`s read its halves. A uniform's pair
/// is one word already: the first node becomes the operation on it (a
/// 16-bit uniform node of two lanes, read whole), the second is left to
/// the dead code, its readers moved to the first's high half.
pub fn vectorise_int_sources(f: &mut Fs<'_>) -> Result<(), MaliError> {
    let pairs = int_source_pairs(f).ok_or(MaliError::Defect(Defect::IrIndex))?;
    for (a, b) in pairs.into_iter().flatten() {
        let nd = f.nir.nodes[b as usize];
        let Kind::Alu(op) = nd.kind else { return Err(MaliError::Defect(Defect::IrIndex)) };
        let mut src = [Val::Undef; 3];
        let mut lanes = [Val::Undef; 2];
        let mut word = None;
        for k in 0..op.arity() {
            src[k] = match f.nir.nodes[a as usize].src[k] {
                Val::Const(c) => Val::Const(u32::from(f16_exact(c).ok_or(MaliError::Fp16Inexact)?)),
                Val::Lane(n, c) if matches!(f.nir.nodes[n as usize].kind, Kind::Input { .. }) => {
                    lanes = [Val::Lane(n, c), Val::Lane(n, c + 1)];
                    Val::Lane(a, 0)
                }
                Val::Lane(n, _) => {
                    let Kind::Uniform { dword, .. } = f.nir.nodes[n as usize].kind else {
                        return Err(MaliError::Defect(Defect::IrIndex));
                    };
                    let u =
                        NirNode { kind: Kind::Uniform { dword, comps: 2 }, src: [Val::Undef; 3], bits: 16, width: 2 };
                    let w = match f.nir.nodes[..f.nir.n].iter().position(|x| *x == u) {
                        Some(i) => i as u16,
                        None => f.nir.push_node(u)?,
                    };
                    word = Some(w);
                    Val::Lane(w, 0)
                }
                v => v,
            };
        }
        if word.is_some() {
            f.nir.nodes[a as usize] = NirNode { kind: nd.kind, src, bits: 16, width: 2 };
            for id in 0..f.nir.n {
                for s in 0..3 {
                    if f.nir.nodes[id].src[s] == Val::Lane(b, 0) {
                        f.nir.nodes[id].src[s] = Val::Lane(a, 1);
                    }
                }
            }
            continue;
        }
        f.nir.nodes[a as usize] =
            NirNode { kind: Kind::Alu(Alu::F2f16), src: [lanes[0], lanes[1], Val::Undef], bits: 16, width: 2 };
        f.nir.nodes[b as usize] = NirNode { kind: nd.kind, src, bits: 16, width: 2 };
        f.origin[b as usize] = f.origin[a as usize];
        for id in 0..f.nir.n {
            if !matches!(f.nir.nodes[id].kind, Kind::Alu(Alu::F2i32 | Alu::F2f32)) {
                continue;
            }
            match f.nir.nodes[id].src[0] {
                Val::Lane(n, 0) if n == a => f.nir.nodes[id].src[0] = Val::Lane(b, 0),
                Val::Lane(n, 0) if n == b => f.nir.nodes[id].src[0] = Val::Lane(b, 1),
                _ => {}
            }
        }
    }
    Ok(())
}

/// The mediump shader `f` computes its colour from booleans: dEQP's
/// shader-library result check, `vec4(RES, RES, RES, 1.0)` of an `isOk`
/// chain, and the operator suite's `float(res)` / `vec3(bvec3)` of a
/// compare (deqp_intval_reslib_float_fs, reslib_bvec_fs, reslib_mixed_fs,
/// opfs_cmp_ivec3, opfs_equal_bvec2, opfs_logical_xor, opfs_not). Each
/// colour lane is a constant or the `b2f` of a boolean (a 16-bit mask of
/// fp16 compares and their logic, or a 32-bit mask of integer compares),
/// at least one such; the graph holds 32-bit integer operations, fp16
/// operations (the compares' operands and masks), and the inputs,
/// uniforms and constants they read: no look-up, no branch, no discard.
/// Mesa's program is a highp shader's over these ([`select_highp`]: the
/// integer rows, the masks' `LSHIFT_*.v2i16`, the colour's `MUX.v2i16` at
/// the store), the varying lanes its fp16 operations read converted first
/// ([`convert_inputs16`]) and the fp16 lanes paired ([`vectorize16`]).
pub fn bool_colour(f: &Fs<'_>) -> bool {
    if f.prec.highp {
        return false;
    }
    let Some(c) = f.colour else { return false };
    let b2f = |v: Val| match v {
        Val::Lane(n, _) => matches!(f.nir.nodes[n as usize], NirNode { kind: Kind::Alu(Alu::B2f), width: 1, .. }),
        _ => false,
    };
    if !c.iter().all(|&v| matches!(v, Val::Const(_)) || b2f(v)) || !c.iter().any(|&v| b2f(v)) {
        return false;
    }
    let live = |k: usize| f.live[k / 64] >> (k % 64) & 1 != 0;
    /* an fp32 operand of an fp32 compare (the walk's compare arm: an
     * integer's float, an fp16 lane widened), read by such compares alone
     * (deqp_intval_reslib_int_vs: `S32_TO_F32`, `F16_TO_F32`, then
     * `FCMP_OR.f32`) */
    let cmp_operand = |k: usize| {
        let mut readers = (0..f.nir.n).filter(|&j| live(j)).filter(|&j| {
            let nd = f.nir.nodes[j];
            matches!(nd.kind, Kind::Alu(_)) && nd.src.iter().any(|s| matches!(*s, Val::Lane(n, _) if n as usize == k))
        });
        readers.clone().next().is_some()
            && readers.all(|j| matches!(f.nir.nodes[j], NirNode { kind: Kind::Alu(Alu::Fcmp(_)), bits: 32, .. }))
    };
    (0..f.nir.n).filter(|&k| live(k)).all(|k| match f.nir.nodes[k] {
        NirNode { kind: Kind::Preload { .. } | Kind::Uniform { .. }, .. } => true,
        NirNode { kind: Kind::Input { loc, .. }, bits: 32, .. } => (loc as usize) < MAX_VARYINGS,
        NirNode { kind: Kind::Alu(Alu::I2f | Alu::U2f | Alu::F2f32), bits: 32, .. } => cmp_operand(k),
        NirNode { kind: Kind::Alu(op), bits: 32, .. } => op.is_int() || matches!(op, Alu::B2f | Alu::Bcsel),
        NirNode { kind: Kind::Alu(op), bits: 16, .. } => {
            matches!(
                op,
                Alu::Fadd
                    | Alu::Fmul
                    | Alu::Ffma
                    | Alu::Fneg
                    | Alu::Fabs
                    | Alu::Fmin
                    | Alu::Fmax
                    | Alu::Fcmp(_)
                    | Alu::Icmp(_)
                    | Alu::Ior
                    | Alu::Iand
                    | Alu::Ixor
                    | Alu::Inot
                    | Alu::B2f
                    | Alu::Bcsel
                    | Alu::F2f16
                    | Alu::Vec2
            )
        }
        _ => false,
    }) && f.nlook == 0
        && f.discards == 0
        && f.nir.nifs == 0
        && f.nir.nloops == 0
}

/// The mediump shader `f` computes its colour from integers: the operator
/// suite's `vec4(res) * scale + bias` over `res = in0 op in1` of
/// `int(v_in0.z)` and the like (deqp_intval_opfs_add_int_mediump,
/// opfs_mul_ivec4_highp, opfs_sub_assign_ivec2_lowp, opfs_div_int_mediump).
/// The integers and their floats are 32-bit (lower_precision lowers no
/// integer, nor a conversion from one); each is narrowed where it is
/// stored to the mediump `color` ([`Fs::narrowed`]), and the colour's
/// arithmetic is two-lane fp16 operations over `vec2`s of those halves and
/// constants ([`Fs::alu16`]), each colour lane one of theirs or a
/// constant. No fp16 operation reads a varying's lane (that is the pair
/// graph's shape, [`select`]); no look-up, branch or discard. Mesa's
/// program is a highp shader's over these ([`select_highp`]: the integer
/// rows, `FADD.f32 .h0` narrowings, `MKVEC.v2i16` / `U16_TO_U32` pairs and
/// `FMA.v2f16`).
pub fn int_colour16(f: &Fs<'_>) -> bool {
    if f.prec.highp {
        return false;
    }
    let Some(c) = f.colour else { return false };
    let pair_lane = |v: Val| match v {
        Val::Lane(n, _) => matches!(f.nir.nodes[n as usize], NirNode { kind: Kind::Alu(_), bits: 16, width: 2, .. }),
        _ => false,
    };
    if !c.iter().all(|&v| matches!(v, Val::Const(_)) || pair_lane(v)) || !c.iter().any(|&v| pair_lane(v)) {
        return false;
    }
    let live = |k: usize| f.live[k / 64] >> (k % 64) & 1 != 0;
    let narrowed_int = (0..f.nir.n).any(|k| {
        live(k)
            && matches!(f.nir.nodes[k], NirNode { kind: Kind::Alu(Alu::F2f16), bits: 16, width: 1, src }
                if f.int_float(src[0]))
    });
    let arith =
        |op: Alu| matches!(op, Alu::I2f | Alu::U2f | Alu::Frcp | Alu::Fmul | Alu::Fadd | Alu::Ffma | Alu::Bcsel);
    narrowed_int
        && (0..f.nir.n).filter(|&k| live(k)).all(|k| match f.nir.nodes[k] {
            NirNode { kind: Kind::Preload { .. } | Kind::Uniform { .. }, .. } => true,
            NirNode { kind: Kind::Input { loc, .. }, bits: 32, .. } => (loc as usize) < MAX_VARYINGS,
            NirNode { kind: Kind::Alu(op), bits: 32, .. } => op.is_int() || arith(op),
            NirNode { kind: Kind::Alu(op), bits: 16, src, .. } => {
                matches!(op, Alu::F2f16 | Alu::Vec2 | Alu::Fadd | Alu::Fmul | Alu::Ffma | Alu::Fneg)
                    && src[..op.arity()].iter().all(|&v| f.input_lane(v).is_none())
            }
            _ => false,
        })
        && f.nlook == 0
        && f.discards == 0
        && f.nir.nifs == 0
        && f.nir.nloops == 0
}

/// The compares `lanes` (lane order) of an `all` / `any` of an equality in
/// `lower_reduction`'s order ([`walk_nodes`]'s Bany/Ball arm): before merge
/// `accs[0]` the last two lanes' (the last first), before each later merge
/// the next lane down. Each compare that is a node of the graph is taken
/// out of the order and put back there; any other lane (a constant, a
/// folded value) is left alone.
fn reduction_order(f: &mut Fs<'_>, lanes: &[Val], accs: &[Val]) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let n = lanes.len();
    let node = |v: Val| match v {
        Val::Lane(x, 0) => Some(x),
        _ => None,
    };
    for (k, acc) in accs.iter().enumerate() {
        let before: &[usize] = if k == 0 { &[n - 1, n - 2] } else { &[n - 2 - k] };
        let a = node(*acc).ok_or(defect)?;
        for &i in before {
            let Some(e) = node(lanes[i]) else { continue };
            if !matches!(f.nir.nodes[e as usize].kind, Kind::Alu(Alu::Icmp(_) | Alu::Fcmp(_))) {
                continue;
            }
            let Some(from) = f.nir.order[..f.nir.len].iter().position(|&x| x == e) else { continue };
            f.nir.order.copy_within(from + 1..f.nir.len, from);
            let to = f.nir.order[..f.nir.len - 1].iter().position(|&x| x == a).ok_or(defect)?;
            f.nir.order.copy_within(to..f.nir.len - 1, to + 1);
            f.nir.order[to] = e;
        }
    }
    Ok(())
}

/// `nir_opt_constant_folding` of one lane of a 16-bit `fmul` or `fadd` of
/// two constants (`p`, `q` fp16 halves, [`Fs::alu16`]'s operands): the
/// result rounded to fp16, as the walk holds a constant (its fp32 bits,
/// [`super::select::f16_bits`]).
fn fold16(a: AluOp, p: Val, q: Val) -> Result<u32, MaliError> {
    let (Val::Const(p), Val::Const(q)) = (p, q) else { return Err(MaliError::Defect(Defect::IrIndex)) };
    let h = |b: u32| f32::from_bits(super::select::f16_bits(b as u16));
    let r = if a == AluOp::Fmul { h(p) * h(q) } else { h(p) + h(q) };
    f16_rtne(r.to_bits()).map(super::select::f16_bits).ok_or(MaliError::Fp16Inexact)
}

/// `nir_lower_mediump_io`'s conversion of each varying lane a mediump
/// shader's fp16 operations read, made explicit for [`select_highp`]
/// (nir_lower_mediump.c: the `f2fmp` right after the load, `nir_opt_move`
/// keeping the load just before it): the lanes of one aligned pair both
/// read are one conversion of two lanes (`nir_opt_vectorize` pairs the two
/// `f2f16`, `V2F32_TO_V2F16 r0, r0^, r1^`), a lane read alone its own
/// (`FADD.f32 r1.h0, r2^, 0x0.neg`), in lane order after the load
/// (deqp_intval_reslib_bvec_fs: x and y paired, z alone;
/// deqp_intval_opfs_equal_bvec2: `v_in0`'s y and w each alone, `v_in1`'s x
/// and y paired). The fp16 readers then read the conversion's lanes, which
/// [`vectorize16`] pairs by ([`lane_key`]). An integer's conversion of a
/// lane (`f2i32`, [`Fs::int_lane`]) keeps reading the word as loaded.
///
/// Lanes the link laid out as one ([`layout`]: Mesa's `deduplicate_outputs`
/// makes the fragment shader read the first of equal lanes) are one load,
/// so their conversions are one (`nir_opt_cse`): a varying whose lanes read
/// are all that one lane has one conversion of it alone, which every reader
/// reads (deqp_varylink_swz_xx: `out0 = in0.xx`, then `FADD.f32 r0.h0, r0^,
/// 0x0.neg` and the subtraction's `r0^.h00`). Equal lanes beside another
/// lane read, or of two varyings, have no reference: refused by name.
pub fn convert_inputs16(f: &mut Fs<'_>) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    f.inputs16 = true;
    /* the lanes read: bit c of word (input node) */
    let mut read = [0u8; MAX_NODES];
    for id in 0..f.nir.n {
        let nd = f.nir.nodes[id];
        let Kind::Alu(op) = nd.kind else { continue };
        if nd.bits != 16 {
            continue;
        }
        for &v in &nd.src[..op.arity()] {
            if let Some((loc, c)) = f.input_lane(v) {
                if (loc as usize) < MAX_VARYINGS {
                    let Val::Lane(n, _) = v else { return Err(defect) };
                    read[n as usize] |= 1 << (c & 3);
                }
            }
        }
    }
    /* the lanes read, as laid out: (slot, lane), the input node */
    let mut places = [((0u8, 0u8), 0usize); 4 * MAX_VARYINGS];
    let mut nplaces = 0;
    for n in 0..f.nir.n {
        if read[n] == 0 {
            continue;
        }
        let Kind::Input { loc, .. } = f.nir.nodes[n].kind else { return Err(defect) };
        let of = *f.lane_of.get(loc as usize).ok_or(defect)?;
        /* the lanes read that the link laid out as an earlier lane read */
        let mut merged = 0u8;
        for c in (0..4).filter(|&c| read[n] & 1 << c != 0) {
            let Some(at) = of[c] else { continue };
            match places[..nplaces].iter().find(|x| x.0 == at) {
                Some(&(_, m)) if m == n => merged |= 1 << c,
                Some(_) => return Err(MaliError::VaryingDuplicate),
                None => {
                    *places.get_mut(nplaces).ok_or(defect)? = (at, n);
                    nplaces += 1;
                }
            }
        }
        let first = read[n].trailing_zeros() as usize;
        if merged != 0 && read[n] != merged | 1 << first {
            return Err(MaliError::VaryingDuplicate);
        }
        /* the one lane converted, and read in place of the merged ones */
        let lanes = if merged != 0 { 1 << first } else { read[n] };
        let at = f.origin[n];
        /* in the order right after the load, lane by lane: a node's place
         * among those of one key is its place in the order */
        let mut after = f.nir.order[..f.nir.len].iter().position(|&x| x as usize == n).ok_or(defect)? + 1;
        let mut conv = [Val::Undef; 4];
        let mut c = 0;
        while c < 4 {
            let pair = c % 2 == 0 && lanes & 3 << c == 3 << c;
            if lanes & 1 << c == 0 {
                c += 1;
                continue;
            }
            let (src, width) = if pair {
                ([Val::Lane(n as u16, c as u8), Val::Lane(n as u16, c as u8 + 1), Val::Undef], 2)
            } else {
                ([Val::Lane(n as u16, c as u8), Val::Undef, Val::Undef], 1)
            };
            let nd = NirNode { kind: Kind::Alu(Alu::F2f16), src, bits: 16, width };
            let id = match f.nir.nodes[..f.nir.n].iter().position(|x| *x == nd) {
                Some(i) => i as u16,
                None => {
                    let id = f.nir.insert_before(after, nd)?;
                    f.nir.block[id as usize] = f.nir.block[n];
                    f.origin[id as usize] = at;
                    after += 1;
                    id
                }
            };
            conv[c] = Val::Lane(id, 0);
            if pair {
                conv[c + 1] = Val::Lane(id, 1);
                c += 2;
            } else {
                c += 1;
            }
        }
        for c in (0..4).filter(|&c| merged & 1 << c != 0) {
            conv[c] = conv[first];
        }
        for id in 0..f.nir.n {
            let nd = f.nir.nodes[id];
            let Kind::Alu(op) = nd.kind else { continue };
            if nd.bits != 16 || op == Alu::F2f16 {
                continue;
            }
            for s in 0..op.arity() {
                if let Val::Lane(m, c) = f.nir.nodes[id].src[s] {
                    if m as usize == n && conv[c as usize & 3] != Val::Undef {
                        f.nir.nodes[id].src[s] = conv[c as usize & 3];
                    }
                }
            }
        }
    }
    Ok(())
}

/// The mediump shader `f` converted varying lanes for its fp16 operations
/// ([`convert_inputs16`]): their lanes pair as a ladder's ([`vectorize16`])
/// and the compares move to their readers ([`nir::move_compares`]). Not
/// the pairs [`vectorise_int_sources`] converts for a colour of masks,
/// whose route stays as it is (intbig_case42, intloop_int_compare).
pub fn inputs16(f: &Fs<'_>) -> bool {
    f.inputs16
}

/// The 32-bit compare `c` a mediump colour's select reads, emitted where
/// the select's conversion stands when [`select_highp`] left it for there.
fn store_cmp(
    p: &mut Prog<'_>,
    f: &Fs<'_>,
    lv: &Leaves,
    val: &mut [Option<Src>],
    rows: &mut u128,
    c: usize,
) -> Result<(), MaliError> {
    let nd = f.nir.nodes[c];
    let (Kind::Alu(op @ (Alu::Icmp(_) | Alu::Fcmp(_))), 32) = (nd.kind, nd.bits) else { return Ok(()) };
    if val.get(c).copied().flatten().is_some() {
        return Ok(());
    }
    let s = [src32(f, lv, val, nd.src[0])?, src32(f, lv, val, nd.src[1])?];
    let r = row(p, rows, Row { op, bits: 32, width: 1 }, &s)?;
    *val.get_mut(c).ok_or(MaliError::Defect(Defect::IrIndex))? = Some(Src::ssa(r));
    Ok(())
}

/// The fragment program of a highp shader `f` ([`walk`], [`layout`]) into
/// `p`: the late fusion and rules of the vertex stage on its fp32 nodes
/// (nir.rs `fuse`, `late_rules`), each load where its first read stands and
/// each node where its IR operation does, as its fp32 row, then the
/// colour's conversions ([`Word`]), ATEST on the alpha and BLEND.
pub fn select_highp(p: &mut Prog<'_>, f: &mut Fs<'_>, ubo: &UboLayout, scratch: &Arena<'_>) -> Result<(), MaliError> {
    let oom = |_| MaliError::OutOfScratch;
    let defect = MaliError::Defect(Defect::IrIndex);
    f.ubo = *ubo;
    let mut c = f.colour.ok_or(MaliError::NoColour)?;
    let store0 = f.nir.n;
    /* the colour's stores, then the further targets': the fusion counts
     * them as uses */
    for (out, lanes) in core::iter::once((0, c)).chain(targets(f)) {
        for (k, v) in lanes.iter().enumerate() {
            let st = NirNode {
                kind: Kind::Store { out, word: k as u8 },
                src: [*v, Val::Undef, Val::Undef],
                bits: 32,
                width: 1,
            };
            f.nir.push_node(st)?;
        }
    }
    let first = f.nir.n;
    b2f_products(&mut f.nir);
    /* the GL front end's loop then `bi_optimize_loop` (bifrost_nir.c:
     * 202-281): DCE, then the `if`s whose sides and phis died go
     * (`nir_opt_dead_cf`), then the small ones become selects
     * (`nir_opt_peephole_select` at the front end's limit with uniform
     * reads allowed, then at bifrost's with them refused; counted on the
     * unfused graph as NIR counts them); the late fusion follows. An
     * unrolled loop's guards the front end's call leaves are inverted
     * before bifrost's ([`nir::Nir::inverted_guards`]); once the blocks
     * merged, the selects of constants fold and equal compares are one */
    nir::dce(&mut f.nir, None);
    f.nir.dead_cf();
    if guarded16(f) {
        ladder_forms(f)?;
    }
    if kept16(f) {
        branch_forms(f)?;
        revector_coords(f)?;
    }
    if multi16(f) {
        pair_coords(f)?;
        nir::dce(&mut f.nir, None);
        multi_forms(f)?;
    }
    let mut inots = f.nir.inverted_guards();
    f.nir.invert_ladder(&mut inots)?;
    f.nir.peephole_select(nir::PEEPHOLE_LIMIT_GL, true, &inots);
    f.nir.invert_guards(&inots)?;
    f.nir.peephole_select(nir::PEEPHOLE_LIMIT, false, &[false; nir::MAX_IFS]);
    /* a `textureSize`'s null test kept as a branch (its side loads the
     * depth or the layer count, [`Fs::tex_size`]) in a mediump shader: no
     * reference (texmore_46 and texmore_48 are highp) */
    if !f.prec.highp && f.nir.live_ifs().any(|r| r.inot) {
        return Err(MaliError::TexKind);
    }
    /* a guarded pass too large for the peephole stays a branch (femtovg's
     * filter_image) */
    f.nir.lay_out_inverted_guards();
    f.nir.fold_const_selects();
    f.nir.cse_compares();
    nir::dce(&mut f.nir, None);
    /* a mediump ladder's fp16 lanes pair in bifrost's optimisation loop,
     * before the late rules (bifrost_nir.c:228-229, 335-344); so do a
     * mediump shader's with kept branches ([`kept16`]: the two `if`s'
     * compares of cflow_148_texbranch, and its guards' two by two) */
    let ladder = guarded16(f) || kept16(f);
    /* so do the compares a mediump `any` / `all` reduces ([`masks16`]:
     * intbool_76_boolany's `greaterThan` of a vec4, two `FCMP_OR.v2f16`) */
    /* and the fp16 operations on converted varying lanes of a colour of
     * booleans ([`bool_colour`], [`convert_inputs16`]: deqp_intval_reslib_
     * float_fs's `isOk` of a vec4, each operation one `.v2f16` of two lanes) */
    let masks = masks16(f) || inputs16(f);
    let pairs16 = ladder || masks;
    if pairs16 {
        vectorize16(f)?;
    }
    /* an unrolled pass's fp16 product that found no other lane to pair
     * with: where Mesa schedules the passes' sums against the compares
     * then is not measured, and ours differs (`ceil(u_k.z * 3.0)` and
     * `ceil(3.0 * u_k.x)` leave `u_k.y * 2.0` alone, case 70's loop with
     * `acc * 3.0` leaves each pass's `k * 1.5`; tools/mali/testdata/
     * deadloop_*): only pairs are, cflow_71_deadloop's */
    if guarded16(f)
        && f.nir.order[..f.nir.len].iter().any(|&id| {
            matches!(f.nir.nodes[id as usize], NirNode { kind: Kind::Alu(Alu::Fmul), bits: 16, width: 1, .. })
        })
    {
        return Err(MaliError::GuardMediump);
    }
    nir::fuse(&mut f.nir);
    nir::dot_addend(&mut f.nir)?;
    nir::late_rules(&mut f.nir);
    /* TEX_MULTI's colour lanes as the pairing left them (its stores'
     * sources: a lane paired is the pair's half) */
    if multi16(f) {
        for (k, x) in c.iter_mut().enumerate() {
            *x = f.nir.nodes[store0 + k].src[0];
        }
    }
    if pairs16 {
        late_rules16(f);
    }
    nir::bcsel_fadd(&mut f.nir)?;
    if ladder {
        nir::bcsel_fadd16(&mut f.nir, scratch)?;
    }
    nir::dce(&mut f.nir, None);
    /* the ladder is one block now: its compares move to their selects */
    /* and on a colour of integers' floats ([`int_colour16`]), whose `vec2`s
     * are copies the pass moves; twice, the link's run then the compile's
     * (nir.rs [`nir::move_compares`]) */
    let ints = int_colour16(f);
    if (ladder && f.nir.live_ifs().next().is_none()) || masks || ints {
        nir::move_compares(&mut f.nir, scratch, ints)?;
        nir::move_compares(&mut f.nir, scratch, ints)?;
    }
    clamp_forms(&f.nir)?;
    shrink_uniforms(f);
    /* an fneg the fusion made stands where its ffma does, a select
     * `bcsel_fadd` made where its fadd does: each before the node after it */
    for k in (0..f.nir.len).rev() {
        let id = f.nir.order[k] as usize;
        if id >= first {
            let next = f.nir.order[..f.nir.len].get(k + 1).copied().ok_or(defect)?;
            f.origin[id] = f.origin[next as usize];
        }
    }
    /* events: (block, key, what, its index): a phi (first in its block), a
     * load, the look-up, a node; the nodes of one place in their program
     * order; then, at a block's end, the compare its `if` tests and, after
     * it, a load no node of the block reads. A block's events are emitted
     * together, the blocks in layout order (`emit_cf_list`), each `if`'s
     * branch and jump at its blocks' ends ([`end_block`]) */
    const PHI: u8 = 0;
    const LOAD: u8 = 1;
    const TEX: u8 = 2;
    const UBO: u8 = 3;
    const NODE: u8 = 4;
    const COND: u8 = 5;
    const TAIL_LOAD: u8 = 6;
    const OFFSET: u8 = 7;
    let events = scratch
        .alloc_filled(MAX_VARYINGS + MAX_NODES + MAX_LOOK_UPS + 2 * MAX_UBO_READS, (0u8, 0u32, 0u8, 0u16))
        .map_err(oom)?;
    /* the BIR blocks mirror the graph's, with each live `if`'s edges in
     * `emit_if`'s order; a block nothing refers to after the flattening is
     * dead (`merge_block` left it empty) */
    for _ in 1..f.nir.nblocks {
        p.new_block()?;
    }
    for b in 1..f.nir.nblocks {
        let referenced = f
            .nir
            .live_ifs()
            .any(|r| [r.before, r.then_first, r.then_last, r.else_first, r.else_last, r.after].contains(&b))
            || f.nir.loops[..f.nir.nloops].iter().any(|r| [r.pre, r.header, r.latch, r.exit].contains(&b))
            || f.nir.order[..f.nir.len].iter().any(|&id| f.nir.block[id as usize] == b);
        p.blocks[b as usize].dead = !referenced;
    }
    /* the edges in the order Mesa adds them: each region's as it is closed
     * (an inner one first). A side that ends in a `break` enters the
     * loop's exit, not the join (`unconditional_jumps` culls the edge
     * `emit_if` adds after it, bi_block_add_successor); a kept loop's are
     * the block before it into the header, then the latch's back edge */
    for seq in 0..f.nir.nseq {
        if let Some(r) = f.nir.ifs[..f.nir.nifs].iter().find(|r| r.live && r.seq == seq) {
            match r.brk {
                Some(lp) => p.add_successor(r.then_last, f.nir.loops[lp as usize].exit)?,
                None => p.add_successor(r.then_last, r.after)?,
            }
            p.add_successor(r.else_last, r.after)?;
            p.add_successor(r.before, r.else_first)?;
            p.add_successor(r.before, r.then_first)?;
        }
        if let Some(r) = f.nir.loops[..f.nir.nloops].iter().find(|r| r.seq == seq) {
            p.add_successor(r.pre, r.header)?;
            p.add_successor(r.latch, r.header)?;
        }
    }
    /* a load's block: the nearest one dominating every reader's
     * (`nir_opt_sink` with `nir_move_load_input`, bifrost_nir.c:380), and
     * whether a reader stands in that block. One without goes to the
     * block's end, after the compare its `if` tests: the link-time move
     * (bifrost_nir.c:886-890, without `nir_move_load_input`) puts the
     * compare, the `if`'s condition, at the tail, then the compile-time
     * move (:381, with it) moves the load, which no instruction of its
     * block reads, to the tail past the compare (nir_opt_move.c:109-121).
     * The scheduler then takes the compare first among the heads (its
     * delta 0, the first head in instruction order) and so lays it out
     * last: `LD_VAR_BUF_IMM`, `FCMP_OR`, `BRANCHZ`. Measured:
     * fvg_fill_color_glyph_nodiscard.expect.json (the `.v4` load read
     * only inside the `if` and after it) */
    let mut load_block = [0u8; MAX_VARYINGS];
    let mut load_local = [false; MAX_VARYINGS];
    for (k, lb) in load_block.iter_mut().enumerate().take(f.nvar) {
        let mut block: Option<u8> = None;
        let mut readers = 0u32;
        for &id in &f.nir.order[..f.nir.len] {
            let nd = f.nir.nodes[id as usize];
            let arity = match nd.kind {
                Kind::Alu(op) => op.arity(),
                Kind::Phi => 2,
                Kind::Tex => 3,
                Kind::Store { .. } | Kind::Discard => 1,
                _ => 0,
            };
            for v in &nd.src[..arity] {
                if load_lane(f, *v)?.is_some_and(|(u, j, _)| !u && j as usize == k) {
                    let b = f.nir.block[id as usize];
                    block = Some(block.map_or(b, |x| f.nir.lca(x, b)));
                    readers |= 1 << (b & 31);
                }
            }
        }
        *lb = block.unwrap_or(0);
        load_local[k] = readers & (1 << (*lb & 31)) != 0;
    }
    /* the condition a block's live `if` tests, and its key */
    let cond_of = |b: u8| -> Option<u16> {
        f.nir.live_ifs().find(|r| r.before == b).and_then(|r| match r.cond {
            Val::Lane(n, _) => Some(n),
            _ => None,
        })
    };
    /* a load only the colour's conversions read stands right before the
     * first of them: `nir_opt_move` (bifrost_nir.c:375-381) moves a
     * load_input to its first use, here the f2f16 `nir_lower_mediump_io`
     * puts at the store (measured: s4_fragcoord.expect.json's
     * `LD_VAR_BUF_IMM` after gl_FragCoord's two conversions) */
    let mut deferred = [false; MAX_VARYINGS];
    for (k, d) in deferred.iter_mut().enumerate().take(f.nvar) {
        let mut read = false;
        for &id in &f.nir.order[..f.nir.len] {
            let nd = f.nir.nodes[id as usize];
            let arity = match nd.kind {
                Kind::Alu(op) => op.arity(),
                Kind::Phi => 2,
                Kind::Tex => 3,
                Kind::Discard => 1,
                _ => continue,
            };
            for v in &nd.src[..arity] {
                read |= load_lane(f, *v)?.is_some_and(|(u, j, _)| !u && j as usize == k);
            }
        }
        *d = !read && !f.live_look_ups().any(|(_, t)| t.load.map(usize::from) == Some(k));
    }
    /* `nir_opt_move` with `nir_move_comparisons` (bifrost_nir.c:376-381):
     * a comparison stands right before its first user (the `bcsel` a
     * flattened `if` left, or a `b2f`); one the `if` itself reads keeps
     * its place, before the branch */
    /* A `b2i32` is a copy to that pass (nir_opt_sink.c:72, `nir_move_copies`)
     * and moves to its first user too: on a colour of booleans over
     * converted varying lanes ([`bool_colour`]), deqp_intval_reslib_mixed_fs's
     * `CSEL.u32.eq` right before the compare that reads it. The pass walks
     * the block backwards, so a user that moved has moved already: the
     * compare that `b2i32` reads follows it to the new place
     * (reslib_mixed_fs's `FCMP_OR.v2f16.ne` after the `ICMP_AND`) */
    let copies = inputs16(f) || int_colour16(f);
    fn moved_key(f: &Fs<'_>, id: usize, copies: bool, depth: u8) -> u32 {
        let own = f.keys[f.origin[id] as usize];
        let movable = match f.nir.nodes[id].kind {
            Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_)) => true,
            Kind::Alu(Alu::B2i) => copies && f.nir.nodes[id].bits == 32,
            /* a `vec2` of fp16 halves ([`int_colour16`]: right before the
             * `FMA.v2f16` that reads it) */
            Kind::Alu(Alu::Vec2) => copies && f.nir.nodes[id].bits == 16,
            _ => false,
        };
        if !movable || depth > 8 {
            return own;
        }
        f.nir.uses(id as u16).map(|(u, _)| moved_key(f, u as usize, copies, depth + 1)).min().unwrap_or(own)
    }
    let key_of = |id: usize| -> u32 { moved_key(f, id, copies, 0) };
    let mut ne = 0;
    for (k, v) in f.varyings[..f.nvar].iter().enumerate() {
        if deferred[k] {
            continue;
        }
        let b = load_block[k];
        events[ne] = match cond_of(b) {
            Some(c) if !load_local[k] => (b, key_of(c as usize), TAIL_LOAD, k as u16),
            _ => (b, v.key(), LOAD, k as u16),
        };
        ne += 1;
    }
    /* a cube's look-up reads the descriptor nodes its IR instruction made
     * ([`nir::Nir::cube_desc`]): it stands among them, in program order */
    let cube = f.live_look_ups().any(|(_, t)| t.cube.is_some());
    if cube && f.live_look_ups().count() > 1 {
        return Err(MaliError::TexCount);
    }
    /* so does a 2D array's, after the conversions of its coordinate made
     * where it stands (case 42's `F16_TO_F32` of the layer) */
    let inline = cube || f.live_look_ups().any(|(_, t)| t.array) || multi16(f);
    /* and a look-up of an unrolled pass, among the pass's nodes, which all
     * stand where the loop's last instruction does ([`walk_loop`]) */
    let unrolled = |n: u16| f.origin[n as usize] != f.look_up_of(n).map_or(u16::MAX, |j| f.look_ups[j].at);
    for (j, t) in f.live_look_ups().filter(|(_, t)| !inline && !unrolled(t.node)) {
        events[ne] = (f.nir.block[t.node as usize], f.keys.get(t.at as usize).copied().ok_or(defect)?, TEX, j as u16);
        ne += 1;
    }
    for k in 0..f.nir.len {
        let id = f.nir.order[k] as usize;
        match f.nir.nodes[id].kind {
            Kind::Tex if inline || unrolled(id as u16) => {
                events[ne] = (f.nir.block[id], key_of(id), NODE, k as u16);
                ne += 1;
            }
            /* a descriptor load stands where its `textureSize` does
             * ([`Fs::tex_size`]), a field's just before its `u2u32` */
            Kind::TexDesc { .. } => {
                events[ne] = (f.nir.block[id], key_of(id), NODE, k as u16);
                ne += 1;
            }
            /* a `demote_if` stands where its `if` did, after the compare it
             * tests (whose key is its first user's: this one's) */
            Kind::Alu(_) | Kind::Discard => {
                /* the compare a live `if` tests ends its block (above) */
                let b = f.nir.block[id];
                let what = if cond_of(b) == Some(id as u16) { COND } else { NODE };
                events[ne] = (b, key_of(id), what, k as u16);
                ne += 1;
            }
            Kind::Phi => {
                events[ne] = (f.nir.block[id], 0, PHI, k as u16);
                ne += 1;
            }
            /* a float array read (uniarray_highp): its offset where its IR
             * instruction stands, its load moved down to just before its
             * first reader (`nir_opt_move` of `load_ubo`, as the mediump
             * shader's, [`select`]: the offsets of the first two reads, then
             * both loads, then their sum). A read no operation reads (the
             * colour's store alone) has no reference */
            Kind::UniformIdx { .. } => {
                let first = f
                    .nir
                    .uses(id as u16)
                    .filter(|&(u, _)| matches!(f.nir.nodes[u as usize].kind, Kind::Alu(_)))
                    .map(|(u, _)| (f.nir.block[u as usize], key_of(u as usize)))
                    .min()
                    .ok_or(MaliError::UniformIndexed)?;
                if f.nir.uses(id as u16).any(|(u, _)| !matches!(f.nir.nodes[u as usize].kind, Kind::Alu(_))) {
                    return Err(MaliError::UniformIndexed);
                }
                events[ne] = (f.nir.block[id], key_of(id), OFFSET, k as u16);
                events[ne + 1] = (first.0, first.1, UBO, k as u16);
                ne += 2;
            }
            _ => {}
        }
    }
    let events = &mut events[..ne];
    events.sort_unstable_by_key(|e| *e);
    move_loads(f, events, scratch)?;
    let mut lv = Leaves {
        conv: [[None; 4]; MAX_VARYINGS],
        conv_hi: [0; MAX_VARYINGS],
        raw: [[None; 4]; MAX_VARYINGS],
        tex: [[None; 4]; MAX_LOOK_UPS],
        ubo: *ubo,
        ubo_words: [None; 2],
        ubo_off: [None; MAX_UBO_READS],
        ubo_half: [None; MAX_UBO_READS],
        ubo_lanes: [[None; 4]; MAX_UBO_READS],
        desc: [[None; 2]; MAX_SIZE_QUERIES],
        frag: None,
        frag_lanes: [None; 2],
    };
    let val = scratch.alloc_filled(MAX_NODES, None::<Src>).map_err(oom)?;
    let mut rows = 0u128;
    let none = [[None; 4]; MAX_VARYINGS];
    /* the 16-bit masks widened for 32-bit selects: (the compare's node,
     * the block of the widening, its value) */
    let mut wide = [(0u16, 0u8, 0u16); 8];
    let mut nwide = 0;
    /* a mediump colour of masks ([`mask_colour`]): each select the colour
     * alone reads is its 16-bit select at the store (`f2f16(bcsel)`
     * folded), and `nir_move_comparisons` puts its compare right before it
     * (intloop_int_compare.expect.json: each `ICMP_OR` then its `MUX.v2i16`,
     * in lane order): the 32-bit select is not emitted, its compare is at
     * the store ([`store_cmp`]) */
    let store_select = |id: usize| {
        !f.prec.highp
            && matches!(f.nir.nodes[id], NirNode { kind: Kind::Alu(Alu::B2f | Alu::Bcsel), bits: 32, .. })
            && f.nir.uses(id as u16).all(|(u, _)| matches!(f.nir.nodes[u as usize].kind, Kind::Store { .. }))
    };
    let at_store = |id: usize| {
        store_select(id)
            || (matches!(f.nir.nodes[id], NirNode { kind: Kind::Alu(Alu::Icmp(_) | Alu::Fcmp(_)), bits: 32, .. })
                && f.nir.uses(id as u16).count() > 0
                && f.nir.uses(id as u16).all(|(u, _)| store_select(u as usize)))
    };
    let mut late = [(0u16, Val::Undef); MAX_LOCALS * 4];
    let mut nlate = 0;
    let mut block = 0u8;
    for &(b, _, what, x) in events.iter() {
        while block < b {
            end_block(p, f, &lv, val, block)?;
            block += 1;
            p.cur = block;
        }
        match what {
            LOAD | TAIL_LOAD => convert(p, f, x as usize, &none, None, &mut lv, &mut rows)?,
            TEX => tex_highp(p, f, x as usize, &mut lv, val, &mut rows)?,
            /* a float array read's offset, held in its node's value until
             * its load ([`select::ubo_offset`], [`select::ubo_fetch`]) */
            OFFSET => {
                let id = f.nir.order[x as usize] as usize;
                let Kind::UniformIdx { dword, comps } = f.nir.nodes[id].kind else { return Err(defect) };
                let idx = src32(f, &lv, val, f.nir.nodes[id].src[0])?;
                let off = if comps == 4 {
                    super::select::vec4_offset(p, ubo, dword, idx, &mut rows)?
                } else {
                    super::select::ubo_offset(p, ubo, dword, idx, &mut rows)?
                };
                val[id] = Some(Src::ssa(off));
            }
            /* a vec4 element's lanes from the lowest read to the highest
             * (unitype_100.expect.json: `IADD_IMM.i32 r0, r0^, #0x8` then
             * `LD_PKA.i64` of `fcols[fidx].ba`) */
            UBO if matches!(f.nir.nodes[f.nir.order[x as usize] as usize].kind, Kind::UniformIdx { comps: 4, .. }) => {
                let id = f.nir.order[x as usize] as usize;
                let off = val[id].and_then(|s| s.ssa_value()).ok_or(defect)?;
                let (lo, n) = super::select::lanes_read(&f.nir, id as u16)?;
                let w = super::select::vec4_fetch(p, off, lo, n, &mut rows)?;
                let k = f.ubo_idx[..f.nubo].iter().position(|&u| u as usize == id).ok_or(defect)?;
                for c in 0..n as usize {
                    lv.ubo_lanes[k][lo as usize + c] = Some(w[c]);
                }
            }
            UBO => {
                let id = f.nir.order[x as usize] as usize;
                let off = val[id].and_then(|s| s.ssa_value()).ok_or(defect)?;
                val[id] = Some(Src::ssa(super::select::ubo_fetch(p, off)?));
            }
            /* a loop header's phi reads the value of the body's end, made
             * later: filled once the body is selected (`late`) */
            PHI => {
                let id = f.nir.order[x as usize] as usize;
                let nd = f.nir.nodes[id];
                let header = f.nir.loops[..f.nir.nloops].iter().any(|r| r.header == b);
                /* a phi's source is the value's register whole: an fp16
                 * value's half is where its readers find it ([`src32`]), the
                 * copy out of SSA takes no swizzle */
                let s0 = src32(f, &lv, val, nd.src[0])?.swz(Swz::H01);
                let s1 = if header { Src::NULL } else { src32(f, &lv, val, nd.src[1])?.swz(Swz::H01) };
                let r = alu(p, Op::Phi, &[s0, s1])?;
                val[id] = Some(Src::ssa(r));
                if header {
                    if nlate == late.len() {
                        return Err(MaliError::TooManyValues);
                    }
                    late[nlate] = (r, nd.src[1]);
                    nlate += 1;
                }
            }
            _ => {
                let id = f.nir.order[x as usize] as usize;
                let nd = f.nir.nodes[id];
                /* `demote_if`: `DISCARD.b32` of the compare's mask
                 * (bifrost_compile.c:1979-1987), fused by opt.rs */
                if nd.kind == Kind::Discard {
                    let s = src32(f, &lv, val, nd.src[0])?;
                    p.push(Ins::new(Op::DiscardB32, &[], &[s]))?;
                    continue;
                }
                if nd.kind == Kind::Tex {
                    let j = f.look_up_of(id as u16).ok_or(defect)?;
                    tex_highp(p, f, j, &mut lv, val, &mut rows)?;
                    continue;
                }
                /* `pan_nir_load_va_desc`'s `load_ssbo` ([`tex_desc`]) */
                if let Kind::TexDesc { .. } = nd.kind {
                    tex_desc(p, f, &mut lv, val, id as u16)?;
                    continue;
                }
                if at_store(id) {
                    continue;
                }
                let Kind::Alu(op) = nd.kind else { return Err(defect) };
                /* a derivative's operand is widened and read whole by the
                 * pair graph's [`op`] alone (fsarith_138_deriv): on this
                 * route (a mediump shader's flattened regions, its masks,
                 * an array look-up, the fp32 island of gl_FragCoord) no
                 * reference draws one */
                if is_deriv(op) {
                    return Err(MaliError::FsOperandForm);
                }
                /* the `vec2` of an fp16 look-up's two texels is its word
                 * ([`Fs::vec16`]): no instruction, the readers take it whole */
                if op == Alu::Vec2 && nd.bits == 16 && nd.width == 2 {
                    let (x, y) = (src32(f, &lv, val, nd.src[0])?, src32(f, &lv, val, nd.src[1])?);
                    if f.texel(nd.src[0]).is_some()
                        && x.swz == Swz::H00
                        && y.swz == Swz::H11
                        && x.swz(Swz::H01) == y.swz(Swz::H01)
                    {
                        val[id] = Some(x.swz(Swz::H01));
                        continue;
                    }
                }
                let mut s = [Src::NULL; 3];
                /* the conversion of a varying's two lanes reads both
                 * ([`vectorise_int_sources`], `V2F32_TO_V2F16`) */
                let arity = if op == Alu::F2f16 && nd.width == 2 { 2 } else { op.arity() };
                for (k, y) in s.iter_mut().enumerate().take(arity) {
                    let v = nd.src[k];
                    *y = src32(f, &lv, val, v)?;
                    /* an fp16 operation's sources ([`Fs::alu16`]): a vector
                     * of two read whole, a constant's half in both lanes as
                     * a lane's is (`bi_alu_src_index`) */
                    if nd.bits == 16 {
                        let whole = matches!(v, Val::Lane(m, _) if nd.width == 2 && f.nir.nodes[m as usize].width == 2);
                        if whole {
                            *y = y.swz(Swz::H01);
                        } else if matches!(v, Val::Const(_)) {
                            /* a pair [`vectorize16`] made holds a word of two
                             * halves */
                            *y = y.swz(if pairs16 && nd.width == 2 { Swz::H01 } else { Swz::H00 });
                        } else if matches!(op, Alu::Icmp(_))
                            && matches!(v, Val::Lane(m, _) if f.nir.nodes[m as usize].bits == 32 && nd.width == 1)
                        {
                            /* a 32-bit boolean (a bool uniform's word) a
                             * 16-bit compare takes: resized to the mask's
                             * width (`make_sources_canonical`'s `i2i16`), its
                             * low half (deqp_intval_reslib_bvec_fs's
                             * `ICMP_OR.v2u16.eq.m1 r1, r1^.h00, u3.w0.h00, 0x0`).
                             * A conversion of a 32-bit float (the cube level's
                             * `F32_TO_S32.rtz`, cubelodf) reads it whole */
                            *y = y.swz(Swz::H00);
                        }
                    }
                }
                /* `b2i32` of a 16-bit mask keeps its source's width (no arm
                 * of `lower_alu_instr` resizes a `b2i32`): the 32-bit `MUX`
                 * reads the half replicated into a word (`bi_lower_swizzle`'s
                 * `MKVEC.v2i16 m.h0, m.h0`, which opt.rs makes `IADD.v2u16
                 * r0, r0^.h00, 0x0`; deqp_intval_reslib_mixed_fs) */
                if op == Alu::B2i && nd.bits == 32 {
                    if let Val::Lane(c, _) = nd.src[0] {
                        if f.nir.nodes[c as usize].bits == 16 {
                            let h = s[0].swz(Swz::H00);
                            s[0] = Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &[h, h])?);
                        }
                    }
                }
                /* the other way, a 16-bit `bcsel_pan` of a 32-bit compare's
                 * mask reads its low half (`i2i16`, a swizzle; case 218's
                 * `MUX.v2i16 r0, r1^.h00, r0^.h00, r2^.h00`) */
                if op == Alu::Bcsel && nd.bits == 16 {
                    if let Val::Lane(c, _) = nd.src[0] {
                        if f.nir.nodes[c as usize].bits == 32 {
                            s[0] = s[0].swz(Swz::H00);
                        }
                    }
                }
                /* `pan_nir_lower_bool_to_bitsize`: a 32-bit `bcsel_pan` of a
                 * 16-bit compare's mask reads it through `i2i32`, made before
                 * the select and CSE'd (fbload_predif_fs_coord.stderr: one
                 * `%12 = i2i32 %11` for both selects, `S16_TO_S32 r3, r0.h0`) */
                if op == Alu::Bcsel && nd.bits == 32 {
                    if let Val::Lane(c, _) = nd.src[0] {
                        if f.nir.nodes[c as usize].bits == 16 {
                            let b = f.nir.block[id];
                            let seen = wide[..nwide].iter().find(|w| w.0 == c && f.nir.dominates(w.1, b));
                            s[0] = Src::ssa(match seen {
                                Some(w) => w.2,
                                None => {
                                    if nwide == wide.len() {
                                        return Err(MaliError::TooManyValues);
                                    }
                                    let r = row(p, &mut rows, Row { op: Alu::I2i32, bits: 32, width: 1 }, &s[..1])?;
                                    wide[nwide] = (c, b, r);
                                    nwide += 1;
                                    r
                                }
                            });
                        }
                    }
                }
                val[id] = Some(Src::ssa(row(p, &mut rows, Row { op, bits: nd.bits, width: nd.width }, &s)?));
            }
        }
    }
    /* the blocks after the last event: their branches, then the colour's
     * conversions and the stores in the last live one */
    let last = (0..f.nir.nblocks).rev().find(|&b| !p.blocks[b as usize].dead).unwrap_or(0);
    while block < last {
        end_block(p, f, &lv, val, block)?;
        block += 1;
        p.cur = block;
    }
    for &(r, v) in &late[..nlate] {
        let s = src32(f, &lv, val, v)?;
        let k = p.as_slice().iter().position(|i| i.op == Op::Phi && i.dest[0].idx == Idx::Ssa(r, 0)).ok_or(defect)?;
        p.ins[k].src[1] = s;
    }
    /* a highp ES 3.00 output is stored at fp32 (`bi_emit_fragment_out` of
     * an `nir_type_float32` store): its four words as computed, ATEST on
     * the alpha's, `BLEND.v4.f32` of the collected vector (case114_f.
     * expect.json: `ATEST ... r3`, `BLEND.slot2.v4.f32.end @r0:r1:r2:r3`;
     * alu_divloop's constant alpha `0x3F800000`). A deferred load stands
     * before them: its first use is the store (`nir_opt_move`). A `highp
     * uvec4` output's words are stored as they are, `BLEND.v4.u32`, its
     * ATEST on the constant alpha 1.0 (fbload_fbload_int.expect.json) */
    let u32 = f.prec.colour_u32;
    if f.prec.colour_f32 || u32 {
        /* `nir_opt_move` puts a deferred load before its first reader: the
         * alpha's is ATEST (`atest_pan`), the others' the colour's `vec4`,
         * which NIR has after it (intbig_case79.stderr; unitype_100.expect
         * .json's flat `LD_VAR_BUF_IMM` of `v_col.rg` after `ATEST`) */
        let mut load = |p: &mut Prog<'_>, lv: &mut Leaves, rows: &mut u128, v: Val| -> Result<(), MaliError> {
            if let Some((false, k, _)) = load_lane(f, v)? {
                if deferred[k as usize] {
                    deferred[k as usize] = false;
                    convert(p, f, k as usize, &none, None, lv, rows)?;
                }
            }
            Ok(())
        };
        if !u32 {
            load(p, &mut lv, &mut rows, c[3])?;
        }
        let mut s = [Src::NULL; 4];
        for (x, v) in s.iter_mut().zip(c) {
            *x = match src32(f, &lv, val, v) {
                Ok(x) => x,
                Err(_) if load_lane(f, v)?.is_some() => Src::NULL,
                Err(e) => return Err(e),
            };
        }
        /* the look-up's own vector stored whole: `bi_emit_tex_valhall`
         * collects its def right after the split (the `vec4` the store
         * reads), before ATEST; any other colour is collected at the store
         * (fbload_mipgen.stderr's `COLLECT.i32 %32^, %33^, %34^, %35` then
         * `ATEST`, against fbload_isamp's `ATEST` then `COLLECT`). Its place
         * moves the pressure the scheduler weighs (sched.rs) */
        let whole = f.look_ups().iter().any(|t| (0..4).all(|i| c[i] == Val::Lane(t.node, i as u8)));
        let early = if whole { Some(alu(p, Op::Collect, &s)?) } else { None };
        let alpha = if u32 { Src::of(Idx::Const(F32_ONE)) } else { s[3] };
        let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), alpha, Src::of(Idx::Special(Special::AtestDatum, false))])?;
        for (x, v) in s.iter_mut().zip(c) {
            if x.idx == Idx::Null {
                load(p, &mut lv, &mut rows, v)?;
                *x = src32(f, &lv, val, v)?;
            }
        }
        let col = match early {
            Some(col) => col,
            None => alu(p, Op::Collect, &s)?,
        };
        blend(p, 0, col, t, 4, !u32, u32)?;
        /* each further target: its words collected and its own BLEND on
         * the one coverage (fsout_55_es3mrt.expect.json) */
        for (rt, lanes) in targets(f) {
            let mut s = [Src::NULL; 4];
            for (x, v) in s.iter_mut().zip(lanes) {
                *x = src32(f, &lv, val, v)?;
            }
            let col = alu(p, Op::Collect, &s)?;
            blend(p, rt, col, t, 4, !u32, u32)?;
        }
        f.rows = rows;
        return preloads(p);
    }
    /* a mediump colour of masks with further targets: no reference */
    if targets(f).next().is_some() {
        return Err(MaliError::FragOutput);
    }
    let words = [word_of(f, c[0], c[1])?, word_of(f, c[2], c[3])?];
    /* a lane converted alone and in a pair is not referenced */
    for w in words {
        if let Word::Lanes(a, b) = w {
            for x in [a, b] {
                let Some((u, k, l)) = load_lane(f, x)? else { continue };
                for o in words {
                    let Word::Pair(pa, _) = o else { continue };
                    if load_lane(f, pa)?.is_some_and(|(u1, k1, l1)| u1 == u && k1 == k && l1 / 2 == l / 2) {
                        return Err(MaliError::ColourForm);
                    }
                }
            }
        }
    }
    /* the conversions at the store, in lane order; a lane converted once */
    let mut conv = [(Val::Undef, Src::NULL); 4];
    let mut nconv = 0;
    let mut pair = [0u16; 2];
    /* a deferred load, before the first conversion that reads it */
    let mut load = |p: &mut Prog<'_>, lv: &mut Leaves, rows: &mut u128, x: Val| -> Result<(), MaliError> {
        if let Some((false, k, _)) = load_lane(f, x)? {
            if deferred[k as usize] {
                deferred[k as usize] = false;
                convert(p, f, k as usize, &none, None, lv, rows)?;
            }
        }
        Ok(())
    };
    for (k, w) in words.iter().enumerate() {
        match *w {
            Word::Pair(a, b) => {
                load(p, &mut lv, &mut rows, a)?;
                let s = [src32(f, &lv, val, a)?, src32(f, &lv, val, b)?];
                pair[k] = row(p, &mut rows, Row { op: Alu::F2f16, bits: 16, width: 2 }, &s)?;
            }
            Word::Lanes(a, b) => {
                for x in [a, b] {
                    if matches!(x, Val::Const(_)) || conv[..nconv].iter().any(|e| e.0 == x) {
                        continue;
                    }
                    load(p, &mut lv, &mut rows, x)?;
                    /* `f2fmp(b2f32(c))` is `b2f16(c)` (nir_opt_algebraic.py:1980):
                     * the conversion of a boolean lane is its 16-bit MUX, read
                     * as the mask's low half. An fp16 lane ([`Fs::alu16`]) is
                     * stored as it is. `f2f16` of a select of two constants
                     * is folded into a select of the two converted constants
                     * (`nir_try_constant_fold_alu`, nir_opt_constant_folding.c:
                     * 58-150, `_mesa_float_to_half` rounding to nearest
                     * even): a 16-bit select (fbload_predif_fs_coord.
                     * expect.json: `MUX.v2i16 r0, u3.w1, 0x34000000.h11,
                     * r0^.h00`). It reads the mask's low half: a 16-bit
                     * mask's own, a 32-bit one's through `i2i16`
                     * (`pan_nir_lower_bool_to_bitsize`), a `MOV.i32` the copy
                     * propagation takes (bifrost_compile.c:3228-3234; the
                     * `b2f16` of a 32-bit compare, s4_flt.expect.json's
                     * `FCMP_OR.f32` then `MUX.v2i16 .., r0^.h00`). A select
                     * with a side that is no constant is not folded, and
                     * narrowed after it (s4_b2f.expect.json: `CSEL.u32.eq`
                     * then `FADD.f32 r0.h0`) */
                    if let Some((Alu::B2f | Alu::Bcsel, [Val::Lane(c, _), ..])) = f.alu_of(x) {
                        store_cmp(p, f, &lv, val, &mut rows, c as usize)?;
                    }
                    let h00 = |r: u16| Src::ssa(r).swz(Swz::H00);
                    let r = match f.alu_of(x) {
                        _ if f.is_f16(x) || f.texel(x).is_some_and(|(j, _)| f.tex_f16(j)) => src32(f, &lv, val, x)?,
                        Some((Alu::B2f, [c, ..])) => {
                            let s = [src32(f, &lv, val, c)?];
                            h00(row(p, &mut rows, Row { op: Alu::B2f, bits: 16, width: 1 }, &s)?)
                        }
                        Some((Alu::Bcsel, [c, Val::Const(t), Val::Const(e)])) => {
                            let k = |b: u32| -> Result<Src, MaliError> {
                                Ok(Src::of(Idx::Const(u32::from(f16_rtne(b).ok_or(MaliError::Fp16Inexact)?)))
                                    .swz(Swz::H00))
                            };
                            let s = [src32(f, &lv, val, c)?.swz(Swz::H00), k(t)?, k(e)?];
                            h00(row(p, &mut rows, Row { op: Alu::Bcsel, bits: 16, width: 1 }, &s)?)
                        }
                        _ => {
                            let s = [src32(f, &lv, val, x)?];
                            h00(row(p, &mut rows, Row { op: Alu::F2f16, bits: 16, width: 1 }, &s)?)
                        }
                    };
                    conv[nconv] = (x, r);
                    nconv += 1;
                }
            }
            Word::Const(_) | Word::Tex(..) | Word::TexSwapped(..) => {}
        }
    }
    let half = |x: Val| -> Result<Src, MaliError> {
        Ok(match x {
            Val::Const(b) => Src::of(Idx::Const(f16_exact(b).ok_or(MaliError::Fp16Inexact)? as u32)).swz(Swz::H00),
            _ => conv[..nconv].iter().find(|e| e.0 == x).ok_or(defect)?.1,
        })
    };
    let tex_word = |j: u8, h: u8| lv.tex.get(j as usize).and_then(|w| w[h as usize & 3]).ok_or(defect);
    /* ATEST on the alpha: the constant's half, or its converted half */
    let at = match words[1] {
        _ if matches!(c[3], Val::Const(_)) => half(c[3])?,
        Word::Lanes(_, b) => half(b)?,
        Word::Pair(..) => Src::ssa(pair[1]).swz(Swz::H11),
        Word::Tex(j, h) => Src::ssa(tex_word(j, h)?).swz(Swz::H11),
        /* the swapped word's high lane is the look-up word's low half
         * (gm2_texswz_4's `ATEST .. r0.h0`) */
        Word::TexSwapped(j, h) => Src::ssa(tex_word(j, h)?).swz(Swz::H00),
        Word::Const(_) => return Err(defect),
    };
    let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), at, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    let mut out = [Src::NULL; 2];
    for (k, o) in out.iter_mut().enumerate() {
        *o = match words[k] {
            Word::Const(w) => Src::of(Idx::Const(w)),
            Word::Pair(..) => Src::ssa(alu(p, Op::MovI32, &[Src::ssa(pair[k])])?),
            Word::Tex(j, h) => Src::ssa(alu(p, Op::MovI32, &[Src::ssa(tex_word(j, h)?)])?),
            /* `MKVEC.v2i16 w.h1, w.h0`: opt.rs makes it `IADD.v2u16 r1,
             * r1^.h10, 0x0` (gm2_texswz_4) */
            Word::TexSwapped(j, h) => {
                let w = Src::ssa(tex_word(j, h)?);
                let s = [w.swz(Swz::H11), w.swz(Swz::H00)];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
            Word::Lanes(a, b) => {
                let s = [half(a)?, half(b)?];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
        };
    }
    let col = alu(p, Op::Collect, &out)?;
    blend(p, 0, col, t, 2, false, false)?;
    f.rows = rows;
    preloads(p)
}

/// The outputs past location 0 the shader stores ([`Fs::targets`]), as
/// (render target, lanes), in target order: the order of their `BLEND`s,
/// as `pan_nir_lower_fs_outputs` emits one per written slot.
fn targets(f: &Fs<'_>) -> impl Iterator<Item = (u8, [Val; 4])> {
    f.targets.into_iter().enumerate().filter_map(|(k, t)| t.map(|l| (k as u8 + 1, l)))
}

/// The `BLEND` of render target `rt`: the colour vector `col` (in r0..,
/// [`super::ra`] precolours it), the coverage `cov` ATEST wrote, the
/// target's descriptor (`blend_descriptor_<rt>`, the only field naming the
/// target, docs/notes/mali-fragment-outputs.md "The BLEND word").
pub(super) fn blend(
    p: &mut Prog<'_>,
    rt: u8,
    col: u16,
    cov: u16,
    sr_count: u8,
    f32: bool,
    u32: bool,
) -> Result<u16, MaliError> {
    alu(
        p,
        Op::Blend { sr_count, f32, u32 },
        &[
            Src::ssa(col),
            Src::ssa(cov),
            Src::of(Idx::Special(Special::BlendDescriptor(rt), false)),
            Src::of(Idx::Special(Special::BlendDescriptor(rt), true)),
            Src::NULL,
        ],
    )
}

/// Look-up `j` of a highp shader ([`tex`]): a computed coordinate's two
/// values, or a cube's S, T and explicit level's word, selected before it.
/// A mediump shader's computed coordinate ([`kept16`]) is fp16 lanes,
/// which the look-up reads converted (`F16_TO_F32` of each, lower_precision's
/// `f162f` at the edge of the lowered tree; cflow_148_texbranch's
/// `F16_TO_F32 r4, r3.h1` and `F16_TO_F32 r3, r3^.h0` before each pass's
/// `TEX_SINGLE`).
fn tex_highp(
    p: &mut Prog<'_>,
    f: &Fs<'_>,
    j: usize,
    lv: &mut Leaves,
    val: &[Option<Src>],
    rows: &mut u128,
) -> Result<(), MaliError> {
    let t = f.look_ups().get(j).copied().ok_or(MaliError::Defect(Defect::IrIndex))?;
    let nd = f.nir.nodes[t.node as usize];
    let mut staging = [Src::NULL; 3];
    let n = match t {
        TexLookup { cube: Some(Cube { lod: CubeLod::Explicit }), .. }
        | TexLookup { array: true, .. }
        | TexLookup { d3: true, .. }
        | TexLookup { shadow: true, .. } => 3,
        TexLookup { cube: Some(_), bias, .. } if bias != Val::Undef => 3,
        TexLookup { coord: TexCoord::Computed, .. } => 2,
        _ => 0,
    };
    for (k, x) in staging.iter_mut().enumerate().take(n) {
        let v = nd.src[k];
        *x = src32(f, lv, val, v)?;
        let f16 = matches!(v, Val::Lane(m, _) if f.nir.nodes[m as usize].bits == 16);
        if f16 {
            *x = Src::ssa(row(p, rows, Row { op: Alu::F2f32, bits: 32, width: 1 }, &[*x])?);
        }
    }
    tex(p, f, j, lv, (n > 0).then_some(&staging[..n]))
}

/// The table handle `pan_nir_load_va_desc` loads a texture descriptor
/// through: the texture table (`PAN_TABLE_TEXTURE`, 4) in the low byte of
/// `pan_res_handle(62, 0)` (pan_nir.h:67-71; intbig_case42.expect.json's
/// FAU word `3e000004`).
const TEX_TABLE_HANDLE: u32 = (62 << 24) | 4;

/// A read of a texture descriptor for `textureSize` ([`Kind::TexDesc`],
/// node `id`): `LD_PKA` of the table handle at the descriptor's offset (32
/// bytes each, [`layout`] numbers them) plus the field's byte. The first
/// eight bytes are `LD_PKA.i64`, split into two words
/// (intbig_case42.expect.json: `LD_PKA.i64.unsigned.slot1 @r2:r3, 0x0,
/// u2.w1`; texmore_46's second descriptor `IADD_IMM.i32 r1, 0x0, #0x20`
/// then `LD_PKA.i64 ... r1^, u2.w0`), or, when nothing reads the second
/// word (a 3D texture's size read at its depth alone, texmore_48),
/// `nir_opt_shrink_vectors` keeps the first half only: `LD_PKA.i16` of one
/// register (`LD_PKA.i16.unsigned.slot0 @r0, 0x0, u2.w1`). A second word
/// read at its low half alone would be three halves (`LD_PKA.i48`), which
/// no reference draws. A 16-bit field (the depth, the layer count) is
/// `LD_PKA.i16` at its byte (texmore_46's `IADD_IMM.i32 r3, 0x0, #0x38`
/// then `LD_PKA.i16.unsigned.slot2 @r3, r3^, u2.w0`).
fn tex_desc(p: &mut Prog<'_>, f: &Fs<'_>, lv: &mut Leaves, val: &mut [Option<Src>], id: u16) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let Kind::TexDesc { sampler, byte } = f.nir.nodes[id as usize].kind else { return Err(defect) };
    let index = f.sampler_index(sampler)?;
    let off = Src::of(Idx::Const(32 * u32::from(index) + u32::from(byte)));
    let handle = Src::of(Idx::Const(TEX_TABLE_HANDLE));
    if byte != 0 {
        let r = alu(p, Op::LdPka { words: 1, half: true }, &[off, handle])?;
        *val.get_mut(id as usize).ok_or(defect)? = Some(Src::ssa(r));
        return Ok(());
    }
    let k = f.size_of(id).ok_or(defect)?;
    let halves =
        f.nir.uses(id).fold(0u8, |m, (u, s)| match (f.nir.nodes[u as usize].src[s], f.nir.nodes[u as usize].kind) {
            (Val::Lane(_, 1), Kind::Alu(Alu::Extract16(h))) => m | 1 << (2 + h),
            (Val::Lane(_, 1), _) => m | 0b1100,
            _ => m | 1,
        });
    let words = match halves {
        0b0001 => 1,
        h if h & 0b1000 != 0 => 2,
        _ => return Err(MaliError::TexKind),
    };
    let r = alu(p, Op::LdPka { words, half: words == 1 }, &[off, handle])?;
    let d = lv.desc.get_mut(k).ok_or(defect)?;
    *d = if words == 2 {
        let w = split(p, r, 2)?;
        [Some(w[0]), Some(w[1])]
    } else {
        [Some(r), Some(r)]
    };
    Ok(())
}

/// The texels look-up `t` writes: those the colour reads
/// (`nir_def_components_read`, bifrost_compile.c:3522; case 42's `.a` of
/// the fetch, texmore_2's `.rgb`, cflow_148_texbranch's `.r`, `.g` and `.b`
/// alone), an fp32 look-up's from the lowest one read (unitype_100.expect
/// .json's `.ba`). The pair path packs the texels as a prefix
/// ([`Leaf::TexLane`]): a mediump look-up read at lanes with a gap is
/// refused there ([`select`]).
fn tex_mask(t: &TexLookup, f16: bool) -> u8 {
    let tail = (1u8 << (8 - t.lanes.leading_zeros())) - 1;
    if t.array || f16 || t.lanes == 0 {
        t.lanes
    } else {
        tail & !((1u8 << t.lanes.trailing_zeros()) - 1)
    }
}

/// What ends block `b` of a highp shader (`emit_if`, bifrost_compile.c:
/// 3697-3735): before an `if`, `BRANCHZ.i16.eq cond.h0` to the else side's
/// first block (taken when the mask's low half is zero); at the end of a
/// then side, the jump to the join (`bi_jump`, a `BRANCHZ.i16.eq` on the
/// zero constant after `va_lower_isel`), or to its loop's exit for a then
/// side that ends in a `break` (`bi_emit_jump`). The else side falls
/// through. A kept loop's latch jumps back to its header (`emit_loop`).
fn end_block(p: &mut Prog<'_>, f: &Fs<'_>, lv: &Leaves, val: &[Option<Src>], b: u8) -> Result<(), MaliError> {
    let jump = |p: &mut Prog<'_>, target: u8| -> Result<(), MaliError> {
        p.push(Ins::new(Op::Branchz { target, eq: true }, &[], &[Src::of(Idx::Const(0))]))?;
        Ok(())
    };
    for r in f.nir.live_ifs() {
        if r.before == b {
            /* the mask's low half; the high one of a 16-bit compare of two
             * lanes whose second this `if` tests (`nir_opt_vectorize` pairs
             * the guards' compares two by two, cflow_148_texbranch's
             * `BRANCHZ.eq.reconverge r2^.h1, offset:482`) */
            let c = src32(f, lv, val, r.cond)?;
            let cond = if c.swz == Swz::H11 { c } else { c.swz(Swz::H00) };
            p.push(Ins::new(Op::Branchz { target: r.else_first, eq: true }, &[], &[cond]))?;
        } else if r.then_last == b {
            match r.brk {
                Some(lp) => jump(p, f.nir.loops[lp as usize].exit)?,
                None => jump(p, r.after)?,
            }
        }
    }
    for r in f.nir.loops[..f.nir.nloops].iter().filter(|r| r.latch == b) {
        jump(p, r.header)?;
    }
    Ok(())
}

/// A front-end operation on integers or booleans, or a conversion between
/// them and floats: a mediump shader's [`Fs::int_lane`].
fn int_op(a: AluOp) -> bool {
    use AluOp::*;
    matches!(
        a,
        Iadd | Imul
            | Idiv
            | Ineg
            | Ishl
            | Ishr
            | Ushr
            | Iand
            | Ior
            | Ixor
            | Ilt
            | Ile
            | Ult
            | Ule
            | Ieq
            | Ine
            | B2i
            | B2f
            | I2b
            | F2b
            | Bnot
            | Inot
            | F2i
            | I2f
            | U2f
    )
}

/// The sources a front-end operation reads: the front end's own
/// [`AluOp::arity`] (`iand`, `ior` and the other two-source integer ops
/// read two; docs/notes/mali-indexed-locals.md 7.2).
pub(super) fn ir_arity(a: AluOp) -> usize {
    a.arity()
}

/// [`emit::emit`], noting the row in `rows`.
pub(super) fn row(p: &mut Prog<'_>, rows: &mut u128, r: Row, s: &[Src]) -> Result<u16, MaliError> {
    *rows |= emit::bit(r);
    emit::emit(p, r, s)
}

/// `fmul(b2f(c), b)` is `bcsel(c, b, 0.0)` (nir_opt_algebraic.py:1695)
/// unless the product is only used by an fadd (which fuses it), measured
/// on s4_b2f: `float(v.x > 0.5) * v.y` is one `CSEL`. The product's node
/// becomes the select, where it stands.
fn b2f_products(nir: &mut Nir<'_>) {
    for k in 0..nir.len {
        let x = nir.order[k];
        let nd = nir.nodes[x as usize];
        if nd.kind != Kind::Alu(Alu::Fmul) || nir.only_used_by_fadd(x) {
            continue;
        }
        let b2f = |v: Val| match v {
            Val::Lane(n, 0) if nir.nodes[n as usize].kind == Kind::Alu(Alu::B2f) => Some(nir.nodes[n as usize].src[0]),
            _ => None,
        };
        let (cond, other) = match (b2f(nd.src[0]), b2f(nd.src[1])) {
            (Some(c), None) => (c, nd.src[1]),
            (None, Some(c)) => (c, nd.src[0]),
            _ => continue,
        };
        nir.nodes[x as usize] = NirNode { kind: Kind::Alu(Alu::Bcsel), src: [cond, other, Val::Const(0)], ..nd };
    }
}

/// The clamp forms Mesa's late rules make of a `fmax`/`fmin` with a
/// constant (bifrost_nir_algebraic.py:35-36: `fmin(fmax(a, -1.0), 1.0)` and
/// `fmax(fmin(a, 1.0), -1.0)` are `fsat_signed`; `fmax(a, 0.0)` is
/// `fclamp_pos`, which [`Fs::lane32`] makes), and a negated `b2f` in a
/// product (nir_opt_algebraic.py:1696): each a `FCLAMP`/`bcsel` no
/// reference draws yet, refused.
fn clamp_forms(nir: &Nir<'_>) -> Result<(), MaliError> {
    const F32_ONE: u32 = 0x3f80_0000;
    const F32_MINUS_ONE: u32 = 0xbf80_0000;
    let is_alu = |v: Val, op: Alu| matches!(v, Val::Lane(n, 0) if nir.nodes[n as usize].kind == Kind::Alu(op));
    let has = |nd: &NirNode, c: u32| nd.src[..2].contains(&Val::Const(c));
    /* `v` is `op` with the constant `c` as a source */
    let is_with = |v: Val, op: Alu, c: u32| match v {
        Val::Lane(n, 0) => nir.nodes[n as usize].kind == Kind::Alu(op) && has(&nir.nodes[n as usize], c),
        _ => false,
    };
    for &x in &nir.order[..nir.len] {
        let nd = nir.nodes[x as usize];
        let bad = match nd.kind {
            Kind::Alu(Alu::Fmax) => {
                has(&nd, F32_MINUS_ONE) && nd.src[..2].iter().any(|v| is_with(*v, Alu::Fmin, F32_ONE))
            }
            Kind::Alu(Alu::Fmin) => {
                has(&nd, F32_ONE) && nd.src[..2].iter().any(|v| is_with(*v, Alu::Fmax, F32_MINUS_ONE))
            }
            Kind::Alu(Alu::Fmul) => nd.src[..2].iter().any(|v| match *v {
                Val::Lane(n, 0) => {
                    let m = nir.nodes[n as usize];
                    m.kind == Kind::Alu(Alu::Fneg) && is_alu(m.src[0], Alu::B2f)
                }
                _ => false,
            }),
            _ => false,
        };
        if bad {
            return Err(MaliError::AlgebraicForm);
        }
    }
    Ok(())
}

/// What the operations read: each load's fp16 conversions and fp32 lanes
/// (a coordinate), the look-up's two result words, and where each uniform
/// dword is in Mesa's block (a highp shader's fp32 uniform words).
struct Leaves {
    /// Each lane's conversion: a pair's register at both of its lanes.
    conv: [[Option<u16>; 4]; MAX_VARYINGS],
    /// The lanes read as the high half of their pair's register ([`conv_lane`]).
    conv_hi: [u8; MAX_VARYINGS],
    raw: [[Option<u16>; 4]; MAX_VARYINGS],
    /// Each look-up's words ([`Fs::look_ups`]): two fp16 pairs, or four
    /// fp32 lanes.
    tex: [[Option<u16>; 4]; MAX_LOOK_UPS],
    ubo: UboLayout,
    /// The run-time indexed element's two words ([`ubo_load`]).
    ubo_words: [Option<u16>; 2],
    /// Each float array read's byte offset ([`ubo_offset`]) and loaded
    /// half ([`ubo_fetch`]).
    ubo_off: [Option<u16>; MAX_UBO_READS],
    ubo_half: [Option<u16>; MAX_UBO_READS],
    /// Each vec4 array read's loaded lanes in a highp shader
    /// ([`super::select::vec4_fetch`]), by its place in [`Fs::ubo_idx`].
    ubo_lanes: [[Option<u16>; 4]; MAX_UBO_READS],
    /// Each size query's first two descriptor words ([`Kind::TexDesc`] at
    /// byte 0, [`tex_desc`]; by its place in [`Fs::sizes`]): one register
    /// read as both when only the null test's half is loaded.
    desc: [[Option<u16>; 2]; MAX_SIZE_QUERIES],
    /// The narrowed position's word ([`Leaf::Frag`], [`frag16`]).
    frag: Option<u16>,
    /// Its x and y each narrowed alone ([`Leaf::FragLane`], [`frag16_lanes`]).
    frag_lanes: [Option<u16>; 2],
}

/// The read of a vec4 array element at a run-time index, as Mesa's BIR has
/// it for the fp16 block (tools/mali/testdata/uniidx_i.expect.json,
/// docs/notes/mali-uniform-index.md): the byte offset `index << 3` (the
/// element's two words; `LSHIFT_OR.i32 r2, u2.w0, 0x3020100.b3, 0x0`, the
/// pushed int word shifted by the table's byte, which `va_repair_fau`
/// moves to a register), then `LD_PKA.i64 @r2:r3, r2, 0x0` from UBO 0 (the
/// mode descriptor the constant 0), split into its two words. The array
/// starts the block (its byte offset is the shift alone; any other base has
/// no reference and is refused at the walk, [`MaliError::UniformIndexed`]).
fn ubo_load(p: &mut Prog<'_>, f: &Fs<'_>, lv: &mut Leaves, rows: &mut u128) -> Result<(), MaliError> {
    if f.nubo != 1 {
        return Err(MaliError::Defect(Defect::IrIndex));
    }
    let nd = f.nir.nodes[f.ubo_idx[0] as usize];
    let Kind::UniformIdx { dword, comps } = nd.kind else { return Err(MaliError::Defect(Defect::IrIndex)) };
    if lv.ubo.word(dword) != Some(0) {
        return Err(MaliError::UniformIndexed);
    }
    /* the index: the int uniform's pushed word */
    let idx = match nd.src[0] {
        Val::Lane(i, 0) => match f.nir.nodes[i as usize].kind {
            Kind::Uniform { dword: d, .. } => {
                let base = lv.ubo.word(d).ok_or(MaliError::Defect(Defect::IrIndex))?;
                Src::of(Idx::Ubo { base, comp: 0 })
            }
            _ => return Err(MaliError::UniformIndexed),
        },
        _ => return Err(MaliError::UniformIndexed),
    };
    /* the element's bytes: two fp16 components a word */
    let words = u32::from(comps).div_ceil(2);
    let shift = (4 * words).trailing_zeros();
    if words != 2 || 4 * words != 1 << shift {
        return Err(MaliError::UniformIndexed);
    }
    let off = row(p, rows, Row { op: Alu::Ishl, bits: 32, width: 1 }, &[idx, Src::of(Idx::Const(shift))])?;
    let ld = alu(p, Op::LdPka { words: words as u8, half: false }, &[Src::ssa(off), Src::of(Idx::Const(0))])?;
    let w = split(p, ld, words as usize)?;
    lv.ubo_words = [Some(w[0]), Some(w[1])];
    Ok(())
}

/// The byte offset of float array read `k` (case108_uniidx,
/// docs/notes/mali-uniform-arrays.md): NIR's `ishl idx, 2` (one dword an
/// element: `LSHIFT_OR.i32 r1, u2.w0, 0x3020100.b2, 0x0`), then `iadd
/// base, that` for an array past the block's first word (`IADD_IMM.i32 r1,
/// r1^, #0x10` after `va_fuse_add_imm`); the index the int uniform's pushed
/// word.
fn ubo_offset(p: &mut Prog<'_>, f: &Fs<'_>, k: usize, lv: &mut Leaves, rows: &mut u128) -> Result<(), MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let n = *f.ubo_idx.get(k).ok_or(defect)?;
    let nd = f.nir.nodes[n as usize];
    let Kind::UniformIdx { dword, comps } = nd.kind else { return Err(defect) };
    let base = lv.ubo.word(dword).ok_or(defect)?;
    /* an fp16 element of one or two lanes is a word, of three or four two
     * (`LSHIFT_OR.i32 r1, u2.w0, r0.b33, 0x0`, deqp_uniarray_dyn_vec4_fs) */
    let shift = if comps <= 2 { 2 } else { 3 };
    let idx = match nd.src[0] {
        Val::Lane(i, 0) => match f.nir.nodes[i as usize].kind {
            Kind::Uniform { dword: d, .. } => Src::of(Idx::Ubo { base: lv.ubo.word(d).ok_or(defect)?, comp: 0 }),
            _ => return Err(MaliError::UniformIndexed),
        },
        _ => return Err(MaliError::UniformIndexed),
    };
    let mut off = row(p, rows, Row { op: Alu::Ishl, bits: 32, width: 1 }, &[idx, Src::of(Idx::Const(shift))])?;
    if base > 0 {
        let s = [Src::of(Idx::Const(4 * u32::from(base))), Src::ssa(off)];
        off = row(p, rows, Row { op: Alu::Iadd, bits: 32, width: 1 }, &s)?;
    }
    lv.ubo_off[k] = Some(off);
    Ok(())
}

/// Float array read `k`'s load at its offset ([`ubo_offset`]): `LD_PKA.i16`
/// of UBO 0, the element's fp16 half into the low half of a register.
fn ubo_fetch(p: &mut Prog<'_>, f: &Fs<'_>, k: usize, lv: &mut Leaves) -> Result<(), MaliError> {
    let off = lv.ubo_off.get(k).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex))?;
    let comps = f.ubo_idx.get(k).map_or(1, |&n| f.ubo_comps(n));
    let s = [Src::ssa(off), Src::of(Idx::Const(0))];
    match comps {
        1 => lv.ubo_half[k] = Some(alu(p, Op::LdPka { words: 1, half: true }, &s)?),
        /* a vec2's fp16 pair, `LD_PKA.i32`; a vec3's six bytes, `LD_PKA.i48`
         * (two registers, the z in the second's low half); a vec4's eight,
         * `LD_PKA.i64` (deqp_uniarray_dyn_vec{2,3,4}_fs) */
        2 => lv.ubo_lanes[k][0] = Some(alu(p, Op::LdPka { words: 1, half: false }, &s)?),
        3 | 4 => {
            let ld = alu(p, Op::LdPka { words: 2, half: comps == 3 }, &s)?;
            let w = split(p, ld, 2)?;
            lv.ubo_lanes[k][0] = Some(w[0]);
            lv.ubo_lanes[k][1] = Some(w[1]);
        }
        _ => return Err(MaliError::Defect(Defect::IrIndex)),
    }
    Ok(())
}

/// Load `k` at its byte offset, then its conversions in lane order: a
/// converted pair (`V2F32_TO_V2F16`), or a lane alone (`FADD.f32` to a
/// half), the `f2f16` rows. A coordinate's lanes stay fp32.
fn convert(
    p: &mut Prog<'_>,
    f: &Fs<'_>,
    k: usize,
    form: &[[Option<bool>; 4]; MAX_VARYINGS],
    order: Option<&[u8; 4]>,
    lv: &mut Leaves,
    rows: &mut u128,
) -> Result<(), MaliError> {
    load(p, f, k, lv)?;
    /* in `order` ([`conversion_order`]), else in lane order */
    let mut starts = [NO_LANE; 4];
    match order {
        Some(o) => starts = *o,
        None => {
            let (mut c, mut n) = (0, 0);
            while c < f.varyings[k].comps as usize {
                if let Some(pair) = form[k][c] {
                    starts[n] = c as u8;
                    n += 1;
                    c += if pair { 2 } else { 1 };
                } else {
                    c += 1;
                }
            }
        }
    }
    for &c in starts.iter().take_while(|&&c| c != NO_LANE) {
        conversion(p, f, k, c, form, lv, rows)?;
    }
    Ok(())
}

/// Load `k` (`LD_VAR_BUF_IMM`, or `LD_VAR_SPECIAL` for gl_PointCoord and
/// gl_FragCoord.z) and its lanes split, once.
fn load(p: &mut Prog<'_>, f: &Fs<'_>, k: usize, lv: &mut Leaves) -> Result<(), MaliError> {
    if lv.raw[k][0].is_some() {
        return Ok(());
    }
    let v = f.varyings[k];
    let ld = p.temp()?;
    let op = match v.location {
        POINT_COORD => Op::LdVarSpecialF32 { name: VaryingName::Point, vecsize: v.comps },
        FRAG_Z => Op::LdVarSpecialF32 { name: VaryingName::FragZ, vecsize: v.comps },
        _ => Op::LdVarBufImmF32 { index: v.offset as u8, vecsize: v.comps, flat: v.flat },
    };
    /* a flat load reads no barycentric (`bi_emit_load_vary`: the source
     * is zero, varyconst_117_uniidxf.expect.json's `.src_flat32 ..., 0x0`) */
    let at = if v.flat { Idx::Lut(0) } else { Idx::Reg(61) };
    p.push(Ins::new(op, &[Dest::ssa(ld)], &[Src::of(at)]))?;
    let lanes = split(p, ld, v.comps as usize)?;
    for c in 0..v.comps as usize {
        lv.raw[k][c] = Some(lanes[c]);
    }
    Ok(())
}

/// The fp16 conversion of load `k` whose first lane is `c`: the aligned
/// pair `c`, `c + 1` as one (`V2F32_TO_V2F16`) when `form` says so, else
/// lane `c` alone (`FADD.f32 .h0`); the load made first if it is not yet.
fn conversion(
    p: &mut Prog<'_>,
    f: &Fs<'_>,
    k: usize,
    c: u8,
    form: &[[Option<bool>; 4]; MAX_VARYINGS],
    lv: &mut Leaves,
    rows: &mut u128,
) -> Result<(), MaliError> {
    load(p, f, k, lv)?;
    let c = c as usize;
    let comps = f.varyings[k].comps as usize;
    let lane = |c: usize| lv.raw[k].get(c).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex));
    match form[k].get(c).copied().flatten() {
        Some(true) if c + 1 < comps => {
            let s = [Src::ssa(lane(c)?), Src::ssa(lane(c + 1)?)];
            let r = row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 2 }, &s)?;
            lv.conv[k][c] = Some(r);
            lv.conv[k][c + 1] = Some(r);
            lv.conv_hi[k] |= 1 << (c + 1);
        }
        Some(false) => {
            let s = [Src::ssa(lane(c)?)];
            lv.conv[k][c] = Some(row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 1 }, &s)?);
        }
        _ => return Err(MaliError::Defect(Defect::IrIndex)),
    }
    Ok(())
}

/// No lane: the end of a load's conversion order ([`convert`]).
const NO_LANE: u8 = u8::MAX;

/// Look-up `j` (`bi_emit_tex_valhall`): the coordinate's two fp32 lanes
/// collected as the staging words, the handle's two words (its descriptor
/// index's, [`tex_handle`]), the result split into its words: two fp16
/// pairs, or four fp32 lanes when the texels are stored to a highp local
/// ([`Fs::look_ups`]; the register format `.32` and four destination
/// registers, `bi_count_write_registers`).
fn tex(p: &mut Prog<'_>, f: &Fs<'_>, j: usize, lv: &mut Leaves, computed: Option<&[Src]>) -> Result<(), MaliError> {
    let t = f.look_ups().get(j).copied().ok_or(MaliError::Defect(Defect::IrIndex))?;
    let k = t.load;
    let f16 = f.tex_f16(j);
    /* the staging sources: the coordinate's lanes as loaded, or its two
     * constants (`bi_src_index` of the `load_const`, which the constant
     * passes make FAU words and `bi_lower_vector` moves into the staging
     * registers: tex_coord_const.expect.json's `MOV.i32 r0, u2.w0`,
     * `MOV.i32 r1, u2.w0`) */
    let mut staging = [Src::NULL; 3];
    let n = match (t.coord, k) {
        (TexCoord::Varying(_), Some(k)) => {
            let lane = |c: usize| lv.raw[k as usize][c & 3].ok_or(MaliError::Defect(Defect::IrIndex));
            let x = usize::from(t.load_lane);
            staging[..2].copy_from_slice(&[Src::ssa(lane(x)?), Src::ssa(lane(x + 1)?)]);
            2
        }
        (TexCoord::Const(x, y), None) => {
            staging[..2].copy_from_slice(&[Src::of(Idx::Const(x)), Src::of(Idx::Const(y))]);
            2
        }
        /* a computed coordinate (or a cube's descriptor and level): its
         * values as selected (the caller's) */
        (TexCoord::Computed, None) => {
            let c = computed.ok_or(MaliError::Defect(Defect::IrIndex))?;
            staging.get_mut(..c.len()).ok_or(MaliError::Defect(Defect::IrIndex))?.copy_from_slice(c);
            c.len()
        }
        _ => return Err(MaliError::Defect(Defect::IrIndex)),
    };
    let sr = alu(p, Op::Collect, &staging[..n])?;
    /* a cube's level mode follows the call (`EXPLICIT` or `ZERO_LOD`,
     * pan_nir_lower_tex.c:927-937), its dimension `cube`; a 2D, 2D array
     * or 3D look-up here is `texture`'s computed level, `textureLod(..,
     * 0.0)`'s level 0, or `texelFetch`'s */
    let plain = match (t.array, t.d3) {
        (true, _) => super::bir::TexDim::D2Array,
        (_, true) => super::bir::TexDim::D3,
        _ => super::bir::TexDim::D2,
    };
    let (lod, dim) = match t.cube {
        None if t.fetch => (super::bir::LodMode::Fetch, plain),
        None if t.zero => (super::bir::LodMode::Zero, plain),
        Some(_) if t.bias != Val::Undef => (super::bir::LodMode::ComputedBias, super::bir::TexDim::Cube),
        Some(Cube { lod: CubeLod::Explicit }) => (super::bir::LodMode::Explicit, super::bir::TexDim::Cube),
        Some(Cube { lod: CubeLod::Zero }) => (super::bir::LodMode::Zero, super::bir::TexDim::Cube),
        Some(Cube { lod: CubeLod::Computed }) => (super::bir::LodMode::Computed, super::bir::TexDim::Cube),
        None => (super::bir::LodMode::Computed, plain),
    };
    let tail = (1u8 << (8 - t.lanes.leading_zeros())) - 1;
    let mask = tex_mask(&t, f16);
    let op = Op::TexSingle { sr_count: n as u8, mask, f16, skip: false, lod, dim, shadow: t.shadow };
    let srcs = [Src::ssa(sr), Src::of(Idx::Const(tex_handle(t.index.into()))), Src::of(Idx::Const(0))];
    let r = alu(p, op, &srcs)?;
    let words = lv.tex.get_mut(j).ok_or(MaliError::Defect(Defect::IrIndex))?;
    *words = [None; 4];
    /* fp16 texels packed two a register in mask order (`bi_make_vec_to`'s
     * channels, bifrost_compile.c:3563-3573): the fetch's `.a` alone is
     * the low half of its one register */
    if f16 && mask != tail {
        let n = (mask.count_ones() as usize).div_ceil(2);
        let w = if n == 1 { [r; 4] } else { split(p, r, n)? };
        for (i, x) in words.iter_mut().enumerate().take(n) {
            *x = Some(w[i]);
        }
        return Ok(());
    }
    /* a prefix of texels: the result's words in order (`bi_emit_cached_split`
     * of `bitcount(mask) * bit_size`, bifrost_compile.c:3563: one word is
     * not split, texmore_48's `.r` and `.b`) */
    if mask == tail {
        let chans = mask.count_ones() as usize;
        let n = if f16 { chans.div_ceil(2) } else { chans };
        let w = if n == 1 { [r; 4] } else { split(p, r, n)? };
        for (i, x) in w.iter().enumerate().take(n) {
            words[i] = Some(*x);
        }
        return Ok(());
    }
    let n = mask.count_ones() as usize;
    let w = if n == 1 { [r; 4] } else { split(p, r, n)? };
    let mut k = 0;
    for (c, x) in words.iter_mut().enumerate() {
        if mask & (1 << c) != 0 {
            *x = Some(w[k]);
            k += 1;
        }
    }
    Ok(())
}

/// The narrowed position's lanes in a mediump shader ([`Fs::frag_coord16`]):
/// x and y at fp32 in NIR's order (`u2f32` of both halves of the preloaded
/// pixel, their `+ 0.5`, y's `ffma` with `gl_FbWposYTransform`), each
/// narrowed (`f2f16`) into its own register (prec_84.expect.json:
/// `U16_TO_F32` x2, `FADD_IMM` x2, `FMA.f32`, `FADD.f32 .h0` x2;
/// fsarith_138_deriv's words 2-5 and 13-15, each lane a derivative's
/// operand, [`Leaf::FragLane`]).
fn frag16_lanes(p: &mut Prog<'_>, lv: &Leaves, rows: &mut u128) -> Result<[u16; 2], MaliError> {
    let defect = MaliError::Defect(Defect::IrIndex);
    let px = |c: Swz| Src::of(Idx::Reg(super::ra::POSITION_XY)).swz(c);
    let wpos = |c: u8| -> Result<Src, MaliError> {
        Ok(Src::of(Idx::Ubo { base: lv.ubo.word(FB_WPOS_Y_TRANSFORM).ok_or(defect)?, comp: c }))
    };
    let r32 = |op: Alu| Row { op, bits: 32, width: 1 };
    let ux = row(p, rows, r32(Alu::U2f32), &[px(Swz::H00)])?;
    let uy = row(p, rows, r32(Alu::U2f32), &[px(Swz::H11)])?;
    let half = Src::of(Idx::Const(F32_HALF));
    let x = row(p, rows, r32(Alu::Fadd), &[Src::ssa(ux), half])?;
    let y = row(p, rows, r32(Alu::Fadd), &[Src::ssa(uy), half])?;
    let y = row(p, rows, r32(Alu::Ffma), &[Src::ssa(y), wpos(0)?, wpos(1)?])?;
    let n16 = Row { op: Alu::F2f16, bits: 16, width: 1 };
    let x = row(p, rows, n16, &[Src::ssa(x)])?;
    let y = row(p, rows, n16, &[Src::ssa(y)])?;
    Ok([x, y])
}

/// The narrowed position's word ([`Leaf::Frag`]): `vec2` of the two lanes'
/// halves ([`frag16_lanes`]; prec_84.expect.json's `MKVEC.v2i16 r0, r0^.h0,
/// r1^.h0`).
fn frag16(p: &mut Prog<'_>, lanes: [u16; 2], rows: &mut u128) -> Result<u16, MaliError> {
    let s = [Src::ssa(lanes[0]).swz(Swz::H00), Src::ssa(lanes[1]).swz(Swz::H00)];
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)
}

fn conv_of(lv: &Leaves, k: u8, c: u8) -> Result<u16, MaliError> {
    lv.conv.get(k as usize).and_then(|x| x[c as usize & 3]).ok_or(MaliError::Defect(Defect::IrIndex))
}

/// Converted lane `c` of load `k` as a half: the low half of its own
/// conversion, or the high half of its pair's (deqp_mixprec_rot3's
/// `MKVEC.v2i16 r2, r0.h1, ...`).
fn conv_lane(lv: &Leaves, k: u8, c: u8) -> Result<Src, MaliError> {
    let hi = lv.conv_hi.get(k as usize).is_some_and(|m| m & 1 << (c & 3) != 0);
    Ok(Src::ssa(conv_of(lv, k, c)?).swz(if hi { Swz::H11 } else { Swz::H00 }))
}

/// `vec2` of a converted lane and a constant half (`MKVEC.v2i16`), the
/// constant the low half when `first`.
fn mkvec(
    p: &mut Prog<'_>,
    lv: &Leaves,
    k: u8,
    c: u8,
    half: u16,
    first: bool,
    rows: &mut u128,
) -> Result<u16, MaliError> {
    let x = conv_lane(lv, k, c)?;
    let konst = Src::of(Idx::Const(half as u32)).swz(Swz::H00);
    let s = if first { [konst, x] } else { [x, konst] };
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)
}

/// `vec2` of two converted lanes (load, lane) (`MKVEC.v2i16`).
fn mkvec_lanes(p: &mut Prog<'_>, lv: &Leaves, x: [(u8, u8); 2], rows: &mut u128) -> Result<u16, MaliError> {
    let s = [conv_lane(lv, x[0].0, x[0].1)?, conv_lane(lv, x[1].0, x[1].1)?];
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)
}

/// Words `comp` and `comp + 1` of the fp32 uniform load at `base`,
/// narrowed together ([`Leaf::Wide`]): `V2F32_TO_V2F16` of the two FAU words.
fn wide(p: &mut Prog<'_>, base: u16, comp: u8, rows: &mut u128) -> Result<u16, MaliError> {
    let s = [Src::of(Idx::Ubo { base, comp }), Src::of(Idx::Ubo { base, comp: comp + 1 })];
    row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 2 }, &s)
}

/// Pair node `x` as a source: a conversion, a constant, a uniform word, a
/// word of the look-up, or an operation already selected.
fn operand(pr: &Pairs, x: u8, lv: &Leaves, val: &[Option<Src>; MAX_PAIRS]) -> Result<Src, MaliError> {
    Ok(match pr.nodes[x as usize] {
        P::Leaf(Leaf::Half { k, h, swz }) => Src::ssa(conv_of(lv, k, 2 * h)?).swz(swz),
        P::Leaf(Leaf::Splat { k, c }) => conv_lane(lv, k, c)?,
        P::Leaf(Leaf::Const(w)) => Src::of(Idx::Const(w)),
        P::Leaf(Leaf::Uniform { base, comp, swz }) => Src::of(Idx::Ubo { base, comp }).swz(swz),
        P::Leaf(Leaf::Tex(j, h)) => {
            Src::ssa(lv.tex.get(j as usize).and_then(|w| w[h as usize & 3]).ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::TexSwapped(j, h)) => {
            let w = lv.tex.get(j as usize).and_then(|w| w[h as usize & 3]);
            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?).swz(Swz::H10)
        }
        P::Leaf(Leaf::Frag) => Src::ssa(lv.frag.ok_or(MaliError::Defect(Defect::IrIndex))?),
        P::Leaf(Leaf::FragLane(c)) => {
            Src::ssa(lv.frag_lanes.get(c as usize).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::Ubo(h)) => {
            Src::ssa(lv.ubo_words.get(h as usize).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::UboLane(c)) => {
            let w = lv.ubo_words.get(c as usize / 2).copied().flatten();
            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?).swz(if c % 2 == 0 { Swz::H00 } else { Swz::H11 })
        }
        P::Leaf(Leaf::UboHalf(k)) => {
            Src::ssa(lv.ubo_half.get(k as usize).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::UboWord(k, h)) => {
            let w = lv.ubo_lanes.get(k as usize).and_then(|w| w.get(h as usize)).copied().flatten();
            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::TexLane(j, c)) => {
            let w = lv.tex.get(j as usize).and_then(|w| w[(c as usize / 2) & 3]);
            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?).swz(if c % 2 == 0 { Swz::H00 } else { Swz::H11 })
        }
        P::Leaf(
            Leaf::Mk { .. }
            | Leaf::Lanes { .. }
            | Leaf::Zext(_)
            | Leaf::Lane { .. }
            | Leaf::Pack { .. }
            | Leaf::Wide { .. },
        )
        | P::Op(..) => val[x as usize].ok_or(MaliError::Defect(Defect::IrIndex))?,
        P::Leaf(Leaf::WideLane { .. }) => val[x as usize].ok_or(MaliError::Defect(Defect::IrIndex))?.swz(Swz::H00),
        /* read by its `exp2` alone ([`op`]) */
        P::Leaf(Leaf::Raw { .. }) => return Err(MaliError::Defect(Defect::IrIndex)),
        P::Leaf(Leaf::Part { x, h }) => {
            val[x as usize].ok_or(MaliError::Defect(Defect::IrIndex))?.swz(if h == 0 { Swz::H00 } else { Swz::H11 })
        }
    })
}

/// Operation `x` as its row, its `vec2` operands just before it.
fn op(
    p: &mut Prog<'_>,
    pr: &Pairs,
    x: u8,
    lv: &Leaves,
    val: &mut [Option<Src>; MAX_PAIRS],
    rows: &mut u128,
) -> Result<(), MaliError> {
    let P::Op(a, srcs) = pr.nodes[x as usize] else { return Err(MaliError::Defect(Defect::IrIndex)) };
    /* `exp2` of a load's two fp32 words ([`Leaf::Raw`]): each lane's
     * expansion, then one narrowing of the two (gm2_fsmisc_5) */
    if let P::Leaf(Leaf::Raw { k, h }) = pr.nodes[srcs[0] as usize] {
        let word = |c: u8| lv.raw.get(k as usize).and_then(|r| r[c as usize & 3]);
        let mut e = [Src::NULL; 2];
        for (d, c) in e.iter_mut().zip([2 * h, 2 * h + 1]) {
            let w = word(c).ok_or(MaliError::Defect(Defect::IrIndex))?;
            *d = Src::ssa(exp2_rows(p, Src::ssa(w), Src::of(Idx::Const(F32_ONE)))?);
        }
        val[x as usize] = Some(Src::ssa(row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 2 }, &e)?));
        return Ok(());
    }
    let mut s = [Src::NULL; 3];
    for (k, y) in srcs.iter().enumerate().take(a.arity()) {
        match pr.nodes[*y as usize] {
            P::Leaf(Leaf::Mk { k: v, c, half, first }) => {
                val[*y as usize] = Some(Src::ssa(mkvec(p, lv, v, c, half, first, rows)?))
            }
            P::Leaf(Leaf::Lanes { k: v, c, e, d }) => {
                val[*y as usize] = Some(Src::ssa(mkvec_lanes(p, lv, [(v, c), (e, d)], rows)?))
            }
            P::Leaf(Leaf::Wide { base, comp }) if val[*y as usize].is_none() => {
                val[*y as usize] = Some(Src::ssa(wide(p, base, comp, rows)?))
            }
            P::Leaf(Leaf::WideLane { base, comp }) if val[*y as usize].is_none() => {
                let s = [Src::of(Idx::Ubo { base, comp })];
                val[*y as usize] = Some(Src::ssa(row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 1 }, &s)?))
            }
            /* a lane alone and a constant half as an operand, made where
             * it is read as an output word is ([`vectorise`]'s computed
             * lane beside a constant; deqp_constop_op_abs: `U16_TO_U32 r0,
             * r0^.h0`, then the `FMA.v2f16` reading it) */
            P::Leaf(Leaf::Zext(z)) if val[*y as usize].is_none() => {
                let s = [operand(pr, z, lv, val)?];
                val[*y as usize] = Some(Src::ssa(row(p, rows, Row { op: Alu::U2u32, bits: 32, width: 1 }, &s)?));
            }
            P::Leaf(Leaf::Lane { x: z, half }) if val[*y as usize].is_none() => {
                let s = [operand(pr, z, lv, val)?.swz(Swz::H00), Src::of(Idx::Const(half as u32)).swz(Swz::H00)];
                val[*y as usize] = Some(Src::ssa(row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?));
            }
            _ => {}
        }
        s[k] = operand(pr, *y, lv, val)?;
        /* a one-lane operation reads a constant's low half ([`coord_lane`])
         * and a one-lane operation's (texmore_2's `%9^.h00`) */
        let lone = match pr.nodes[*y as usize] {
            P::Leaf(Leaf::Const(_)) => true,
            /* the narrowed position's lane in its own register
             * (fsarith_138_deriv's `FMA.v2f16 r1, r1^.h00, r2.h00, 0x0.neg`) */
            P::Leaf(Leaf::FragLane(_)) => true,
            P::Op(..) => pr.scalar[*y as usize],
            _ => false,
        };
        /* a one-lane operation read by a two-lane one is its low half in
         * both (fsvec_normalize_mediump's `FMA.v2f16 r0, r0^, r2.h00,
         * 0x0.neg` of the `frsq`, [`vectorise`]) */
        let splat = !pr.scalar[x as usize] && matches!(pr.nodes[*y as usize], P::Op(..)) && pr.scalar[*y as usize];
        /* a select's mask is its compare's low half in both lanes (`r0.h00`)
         * when the compare is of one lane; a compare of two is read whole
         * (gm2_fsmisc_8) */
        let one_mask = a == Alu::Bcsel && k == 0 && (pr.scalar[x as usize] || pr.scalar[*y as usize]);
        if (pr.scalar[x as usize] && lone) || splat || one_mask {
            s[k] = s[k].swz(Swz::H00);
        }
        /* a two-lane select's constant side of equal halves is the 16-bit
         * immediate in both (`bi_imm_u16`), so `va_lower_isel` leaves the
         * `MUX` (gm2_fsmisc_8's `MUX.v2i16 r0, 0x0, r2^, r0^`) */
        if let (Alu::Bcsel, 1 | 2, P::Leaf(Leaf::Const(c))) = (a, k, pr.nodes[*y as usize]) {
            if !pr.scalar[x as usize] && c >> 16 == c & 0xffff {
                s[k] = s[k].swz(Swz::H00);
            }
        }
    }
    if is_transc(a) {
        /* `pow`'s exponent: the fp32 constant itself, no half */
        let k = match (a, pr.nodes[srcs[1] as usize]) {
            (Alu::Fpow, P::Leaf(Leaf::Const(b))) => Src::of(Idx::Const(b)),
            (Alu::Fpow, _) => return Err(MaliError::Defect(Defect::IrIndex)),
            _ => Src::NULL,
        };
        val[x as usize] = Some(Src::ssa(transc_rows(p, rows, a, s[0], k)?));
        return Ok(());
    }
    /* a derivative reads its operand as one register word (`CLPER.i32`
     * takes no half swizzle): a half of a converted pair is widened first,
     * `MKVEC.v2i16 a.h0, #0` as `U16_TO_U32` (fsarith_138_deriv's
     * `U16_TO_U32 r0, r0^.h0` for `v.x`); a lane in its own register (the
     * scaled product, the narrowed position's lane) is read whole */
    if is_deriv(a) {
        if let P::Leaf(Leaf::Half { .. }) = pr.nodes[srcs[0] as usize] {
            s[0] = Src::ssa(row(p, rows, Row { op: Alu::U2u32, bits: 32, width: 1 }, &s[..1])?);
        }
        s[0] = s[0].swz(Swz::H01);
    }
    let width = if pr.scalar[x as usize] { 1 } else { 2 };
    val[x as usize] = Some(Src::ssa(row(p, rows, Row { op: a, bits: 16, width }, &s)?));
    Ok(())
}

/// [`super::emit::fexp2`] of the fp32 value `x`, scaled by `log2_base`.
fn exp2_rows(p: &mut Prog<'_>, x: Src, log2_base: Src) -> Result<u16, MaliError> {
    super::emit::fexp2(p, x, log2_base)
}

/// `bi_lower_fsincos_32`'s constants (bifrost_compile.c:2547-2549): the f32
/// roundings of `2.0f / 3.14159f`, `-3.14159f / 2.0` and the bias that puts
/// the argument's sixty-fourths of a quarter turn in the low bits.
const TWO_OVER_PI: u32 = 0x3f22_f98c;
const MPI_OVER_TWO: u32 = 0xbfc9_0fd0;
const SINCOS_BIAS: u32 = 0x4940_0000;

/// `sin`, `cos` or `log2` of the 16-bit lane `x`, as `bi_lower_bit_size`
/// and `bi_emit_alu` make it: `F16_TO_F32`, the 32-bit expansion, then the
/// narrowing to the low half (`FADD.f32 r.h0, r, -0.0`). The expansions are
/// `bi_lower_fsincos_32` (bifrost_compile.c:2552) and `bi_flog2_32` (:2499),
/// in their call order (fsarith_77_transc.expect.json words 0-28;
/// docs/notes/mali-transcendentals.md).
fn transc_rows(p: &mut Prog<'_>, rows: &mut u128, a: Alu, x: Src, k: Src) -> Result<u16, MaliError> {
    let c = |bits: u32| Src::of(Idx::Const(bits));
    let neg = |s: Src| Src { neg: !s.neg, ..s };
    let neg_zero = neg(c(0));
    let x = Src::ssa(row(p, rows, Row { op: Alu::F2f32, bits: 32, width: 1 }, &[x])?);
    let mut ins = |op: Op, s: &[Src]| -> Result<Src, MaliError> { Ok(Src::ssa(alu(p, op, s)?)) };
    let r = match a {
        Alu::Fsin | Alu::Fcos => {
            let cos = a == Alu::Fcos;
            let x_u6 = ins(Op::FmaF32, &[x, c(TWO_OVER_PI), c(SINCOS_BIAS)])?;
            /* `FADD_IMM` once fused (opt.rs `fuse_add_imm`) */
            let t = ins(Op::FaddF32, &[x_u6, neg(c(SINCOS_BIAS))])?;
            let e = ins(Op::FmaF32, &[t, c(MPI_OVER_TWO), x])?;
            let sinx = ins(Op::FsinTableU6, &[x_u6])?;
            let cosx = ins(Op::FcosTableU6, &[x_u6])?;
            /* e * e * 2^-1, the scale an integer source */
            let e2h = ins(Op::FmaRscaleF32, &[e, e, neg_zero, c(0xffff_ffff)])?;
            let quad = ins(Op::FmaF32, &[neg(e2h), if cos { cosx } else { sinx }, neg_zero])?;
            let t = p.temp()?;
            let mut i = Ins::new(Op::FmaF32, &[Dest::ssa(t)], &[e, if cos { neg(sinx) } else { cosx }, quad]);
            i.clamp = clamp::M1_1;
            p.push(i)?;
            Src::ssa(alu(p, Op::FaddF32, &[Src::ssa(t), if cos { cosx } else { sinx }])?)
        }
        Alu::Fexp2 => Src::ssa(exp2_rows(p, x, c(F32_ONE))?),
        _ => {
            let l = Src::ssa(super::emit::flog2(p, x)?);
            /* `bi_fpow_32` (bifrost_compile.c:2523-2535): the exponent
             * scaled by the log */
            if a == Alu::Fpow {
                Src::ssa(exp2_rows(p, k, l)?)
            } else {
                l
            }
        }
    };
    row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 1 }, &[r])
}
