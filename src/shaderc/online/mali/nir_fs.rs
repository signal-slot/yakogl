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
//! * the varyings: the components read, compacted from VAR0 in layout
//!   order (`compact_varyings`), loaded where their first read stands
//!   ([`layout`]);
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
use super::nir::{self, Alu, Kind, Nir, NirNode, Val, MAX_LOCALS, MAX_NODES, MAX_VALUES};
use super::select::{alu, f16_exact, f16_rtne, generic_layout, preloads, split, UboLayout, VaryingAt, MAX_VARYINGS};
use super::{Defect, FsPrecision, MaliError};
use crate::gl::consts::{GL_BOOL, GL_FLOAT, GL_INT};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::{self, AluOp, Es100Tex, IoKind, IoSlot, Module, Node};

/// The vectorised operations and operands of the colour: a pair node is
/// named by a `u8` ([`P::Op`]), so up to 255 (case 166's sixteen look-ups
/// and their sums take 126, unitype_166c).
pub const MAX_PAIR_NODES: usize = 255;

/// The `demote_if`s one shader may hold ([`Fs::discard`]).
pub const MAX_DISCARDS: usize = 8;

/// The run-time indexed uniform reads one shader may make ([`Fs::ubo_idx`]).
pub const MAX_UBO_READS: usize = 8;

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
    /// The texels the colour reads (bit `c`: texel `c`), set by [`layout`].
    pub lanes: u8,
}

impl TexLookup {
    /// A look-up at IR instruction `at`, its node `node`.
    fn new(sampler: u8, coord: TexCoord, cube: Option<Cube>, fetch: bool, at: usize, node: u16) -> TexLookup {
        TexLookup { sampler, coord, cube, fetch, array: false, at: at as u16, node, index: 0, load: None, lanes: 0 }
    }

    const NONE: TexLookup = TexLookup {
        sampler: 0,
        coord: TexCoord::Computed,
        cube: None,
        fetch: false,
        array: false,
        at: 0,
        node: 0,
        index: 0,
        load: None,
        lanes: 0,
    };

    /// The colour reads it ([`layout`]; Mesa's DCE takes one it does not).
    pub fn live(&self) -> bool {
        self.lanes != 0
    }
}

/// The look-ups one fragment shader may make (femtovg's filter_image makes
/// 49: one, then two in each of 24 unrolled passes,
/// fvg_filter_image.expect.json).
pub const MAX_LOOK_UPS: usize = 64;

/// A cube look-up's staging ([`TexLookup::cube`]). In a highp shader the
/// look-up node's sources are `build_cube_desc`'s S and T and, at an
/// explicit level, its 8.8 word ([`nir::Nir::cube_desc`],
/// [`nir::Nir::sfixed_8_8`]), fp32 nodes of the graph; in a mediump one
/// they are the direction's three fp16 lanes, converted at the look-up
/// (`F16_TO_F32` each, the cube ops being fp32 only) and the descriptor
/// built there ([`select`]; case41.expect.json).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cube {
    /// The level is not the constant 0 (`nir_scalar_is_zero`): the LOD
    /// mode `EXPLICIT` and a third staging word; else `ZERO_LOD`
    /// (pan_nir_lower_tex.c:927-937).
    pub explicit: bool,
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
    for (first, root) in ternaries {
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
        /* each value is pushed once (`under`), so the stack holds them all */
        let mut under = [false; MAX_VALUES];
        let mut stack = [0u32; MAX_VALUES];
        let mut sp = 1;
        stack[0] = root.0;
        while sp > 0 {
            sp -= 1;
            let v = stack[sp] as usize;
            if v >= n || under[v] {
                continue;
            }
            under[v] = true;
            keys[v] = keys[v].min((i as u32) << 16 | v as u32);
            let mut child = |x: u32| {
                if (x as usize) < n && !under[x as usize] && sp < stack.len() {
                    stack[sp] = x;
                    sp += 1;
                }
            };
            match m.instrs[v] {
                ir::Op::Alu(a, srcs) => srcs.iter().take(a.arity()).for_each(|s| child(s.val.0)),
                /* a look-up's coordinate is visited as its child */
                ir::Op::Tex { coord, .. } => child(coord.val.0),
                /* and an indexed uniform read's index (builtin_45's `idx +
                 * i`, an int sum the walk keeps as a node) */
                ir::Op::LoadUniformIdx { index, .. } => child(index.val.0),
                /* a grafted local's one read is its stored tree */
                ir::Op::LoadLocal { local, index: None, .. } => {
                    if let Some(src) = grafted.get(local.0 as usize).copied().flatten() {
                        child(src.0)
                    }
                }
                _ => {}
            }
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
        /* a whole assignment, read with the identity swizzle */
        let comps = m.comps_of(src.val) as usize;
        if write_mask != (1u8 << comps) - 1 || (0..comps).any(|c| src.swizzle[c] as usize != c) {
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
                /* a 2D array's look-up (an `ir_texture`, grafted as any
                 * rvalue: case 42's `t` and `f`, whose `tex` and `txf` stand
                 * at the colour's store in intbig_case42.stderr, after the
                 * fetch's coordinates) */
                Some(ir::Op::Tex { dim: ir::TexDim::D2Array, coord, .. }) => {
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
        if !clobbered {
            *o = Some(src.val);
        }
    }
    out
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
        /* #1385 (b): case 122's `0.5 * (fcols[fidx] + t)` (uniidx_i), case
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
    /// from uniforms alone ([`super::link::VaryingConsts::flat`]).
    pub flat: bool,
    /// Holds a varying the link keeps whole, of which the colour may read
    /// some lanes only ([`layout`]'s packed matrix).
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
/// `nir_lower_idiv`'s scale of the divisor's reciprocal, 4294966784.0 (the
/// largest float below 2^32: the estimate never overshoots;
/// intbig_case79.stderr's `fmul %17, %18 (4.294967e+09)`).
const IDIV_RCP_SCALE: u32 = 0x4f7f_fffe;
/// 2.0 in both halves of an fp16 pair.
const F16X2_TWO: u32 = 0x4000_4000;

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
    prec: FsPrecision,
    /// A value of the other width met ([`MaliError::MixedPrecision`],
    /// reported when no other gap is).
    pub mixed: bool,
    /// Each node's IR instruction (its place in NIR's order, [`keys`]).
    origin: &'s mut [u16],
    /// Each IR instruction's place in NIR's order.
    keys: &'s mut [u32],
    /// The colour's lanes, once the shader stores it.
    pub(super) colour: Option<[Val; 4]>,
    /// The expressions Mesa's link moves into the program's vertex shader
    /// (motion.rs), in the order they were found.
    pub moved: [Moved; MAX_MOVED],
    pub nmoved: usize,
    /// Each varying location's first read (`u32::MAX`: not read).
    var_key: [u32; MAX_VARYINGS],
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
    /// The `break`s of the kept loops walked so far ([`BreakEdge`]).
    breaks: [BreakEdge; MAX_BREAKS],
    nbreaks: usize,
}

/// The `break`s a kept loop may take ([`BreakEdge`]).
pub const MAX_BREAKS: usize = 8;

/// A `break` of a kept loop: the block whose jump enters the loop's exit,
/// and the locals and the colour as they stand there (the exit's phis
/// take one source from each, in the order the jumps were emitted).
#[derive(Clone, Copy)]
struct BreakEdge {
    lp: u8,
    block: u8,
    locals: [[Val; 4]; MAX_LOCALS],
    colour: Option<[Val; 4]>,
}

impl BreakEdge {
    const NONE: BreakEdge = BreakEdge { lp: 0, block: 0, locals: [[Val::Undef; 4]; MAX_LOCALS], colour: None };
}

impl<'s> Fs<'s> {
    fn key(&self, v: Val) -> u32 {
        match v {
            Val::Lane(n, _) | Val::Pair(n, _) => self.keys[self.origin[n as usize] as usize],
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

    /// One lane of `fmul` or `fadd` under the pair's `rule`.
    fn op(&mut self, a: AluOp, x: Val, y: Val, at: usize, rule: Rule) -> Result<Val, MaliError> {
        if x == Val::Undef || y == Val::Undef {
            return Err(MaliError::ColourForm);
        }
        let (cx, cy) = (matches!(x, Val::Const(_)), matches!(y, Val::Const(_)));
        let neg = |s: &Self, v: Val| matches!(s.alu_of(v), Some((Alu::Fneg, _)));
        match (a, rule) {
            (AluOp::Fmul, Rule::IdentitySecond) => Ok(x),
            (AluOp::Fmul, Rule::IdentityFirst) => Ok(y),
            /* `fmul(a, 0.0)` is 0.0 (nir_opt_algebraic.py's inexact rule, in
             * force without `precise`): case 122's `t.w * 0.0` (uniidx_i,
             * no FMA in Mesa's program). Like the others it holds for a
             * lane pair whose constant is 0.0 in both lanes: case 166's
             * `u_p[58].y * vec4(1.0, 0.0, ..)` keeps its x and y lanes as
             * `ffma %6.yy, (1.0, 0.0), %4` (unitype_166a.stderr) */
            (AluOp::Fmul, Rule::Zero) => Ok(Val::Const(0)),
            /* two constants: the front end folds them */
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
            (AluOp::Fmul, _) => self.node(Alu::Fmul, [x, y, Val::Undef], at),
            (AluOp::Fadd, Rule::IdentitySecond) => Ok(x),
            (AluOp::Fadd, Rule::IdentityFirst) => Ok(y),
            (AluOp::Fadd, _) if cx && cy => Err(MaliError::ConstantFold),
            (AluOp::Fadd, _) => self.node(Alu::Fadd, [x, y, Val::Undef], at),
            _ => Err(MaliError::Arithmetic),
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
    /// `fabs(fabs(a))` are `fabs(a)` (nir_opt_algebraic.py).
    fn abs(&mut self, x: Val, at: usize) -> Result<Val, MaliError> {
        match (x, self.alu_of(x)) {
            (Val::Undef, _) => Err(MaliError::ColourForm),
            (Val::Const(b), _) => Ok(Val::Const(b & !F32_NEG_ZERO)),
            (_, Some((Alu::Fneg, [y, ..]))) => self.abs(y, at),
            (_, Some((Alu::Fabs, _))) => Ok(x),
            _ => self.node(Alu::Fabs, [x, Val::Undef, Val::Undef], at),
        }
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
                    let n = self.look_ups[j].node;
                    self.nir.nodes[n as usize].bits = 32;
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
        let bool_uniform = |k: usize| {
            matches!(m.def(srcs[k].val), Some(ir::Op::LoadUniform { dword_off, .. })
                if m.uniforms.iter().any(|u| u.dword_off == *dword_off && u.gl_type as u32 == GL_BOOL))
        };
        match a {
            AluOp::F2i if self.is_f16(p) || self.uniform_dword(p).is_some() => self.int32(Alu::F2i32, [p, u, u], at),
            /* of an fp32 value (case 42's `int(uv.x * float(sz.x))`,
             * `F32_TO_S32.rtz`) */
            AluOp::F2i if self.fp32_lane(p) => self.int32(Alu::F2i, [p, u, u], at),
            AluOp::I2b if bool_uniform(0) => Ok(p),
            AluOp::Bnot | AluOp::Inot if self.mask_lane(p) => match not(self, p) {
                Some(inner) => Ok(inner),
                None => self.int32(Alu::Inot, [p, u, u], at),
            },
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
            AluOp::Iadd if !self.mask_lane(p) && !self.mask_lane(q) => self.int32(Alu::Iadd, [p, q, u], at),
            AluOp::I2f if !self.mask_lane(p) => self.op32(Alu::I2f, [p, u, u], at),
            AluOp::Idiv if !self.mask_lane(p) && !self.mask_lane(q) => self.idiv(p, q, at),
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

    /// `v` is an fp32 value of a mediump shader: an integer's conversion
    /// or arithmetic on one ([`Fs::int_lane`], the walk's fp32 arm).
    fn fp32_lane(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        matches!(
            self.nir.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::I2f | Alu::U2f | Alu::Fmul | Alu::Fadd | Alu::Ffma), bits: 32, .. }
        ) && !self.prec.highp
    }

    fn is_f16(&self, v: Val) -> bool {
        let Val::Lane(n, _) = v else { return false };
        matches!(self.nir.nodes[n as usize], NirNode { kind: Kind::Alu(op), bits: 16, .. } if !op.is_int())
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

    /// The two halves `x` and `y` as one fp16 vector: the vector they are
    /// lanes 0 and 1 of, or their `vec2` (`MKVEC.v2i16 x.h0, y.h0`,
    /// prec_82.expect.json).
    fn vec16(&mut self, x: Val, y: Val, at: usize) -> Result<Val, MaliError> {
        if let (Val::Lane(n, 0), Val::Lane(m, 1)) = (x, y) {
            if n == m && self.nir.nodes[n as usize].width == 2 {
                return Ok(Val::Lane(n, 0));
            }
        }
        if !self.is_f16(x) || !self.is_f16(y) {
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
        /* the front end records no width here (`comps_of` answers 4) and
         * pads a swizzle with its last lane: a lane whose operands are the
         * lane before's is the same scalar operation, one node (`nir_opt_cse`
         * after `nir_lower_alu_width`) */
        let same = |i: usize| (0..ir_arity(a)).all(|k| x[k][i] == x[k][i - 1]);
        let d = (1..w).find(|&i| same(i)).unwrap_or(w);
        if (d..w).any(|i| !same(i)) {
            return Err(MaliError::FragCoordMediump);
        }
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
            AluOp::Ffloor | AluOp::Ffract | AluOp::Fneg | AluOp::Fadd if w == 2 => {
                let v = self.vec16(x[0][0], x[0][1], at)?;
                let r = match a {
                    AluOp::Ffloor => self.node16(Alu::Ffloor, [v, u, u], 2, at)?,
                    AluOp::Fneg => self.node16(Alu::Fneg, [v, u, u], 2, at)?,
                    AluOp::Fadd => {
                        let q = self.vec16(x[1][0], x[1][1], at)?;
                        self.node16(Alu::Fadd, [v, q, u], 2, at)?
                    }
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
                    None => self.op32(Alu::Fadd, [x, y, u], at),
                }
            }
            /* `fmul(fneg(b2f(a)), b)` is `bcsel(a, fneg(b), 0.0)`
             * (nir_opt_algebraic.py:1696), a form no reference draws: refused
             * before the graph's `fmul(fneg(a), b)` rule rewrites it */
            AluOp::Fmul if [x, y].iter().any(|v| self.is_neg_b2f(*v)) => Err(MaliError::AlgebraicForm),
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
                match inner {
                    Some((c2, c1, b, n)) => {
                        let saved = self.nir.nonuniform_const;
                        self.nir.nonuniform_const |= self.nir.nonuniform[n as usize];
                        let folded = self.op32(Alu::Fmul, [Val::Const(c2), Val::Const(c1), u], at);
                        let r = folded.and_then(|c| self.op32(Alu::Fmul, [c, b, u], at));
                        self.nir.nonuniform_const = saved;
                        r
                    }
                    None => self.op32(Alu::Fmul, [x, y, u], at),
                }
            }
            AluOp::Fmul => self.op32(Alu::Fmul, [x, y, u], at),
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
            AluOp::Flt => self.op32(Alu::Fcmp(Cmpf::Lt), [x, y, u], at),
            /* GLSL's `<=` is `fge` with its sources swapped (glsl_to_nir.cpp:2488) */
            AluOp::Fle => self.op32(Alu::Fcmp(Cmpf::Ge), [y, x, u], at),
            AluOp::Feq => self.op32(Alu::Fcmp(Cmpf::Eq), [x, y, u], at),
            AluOp::Ilt => self.op32(Alu::Icmp(Cmpf::Lt), [x, y, u], at),
            AluOp::Ile => self.op32(Alu::Icmp(Cmpf::Ge), [y, x, u], at),
            /* `ieq_pan` (femtovg's `texType == 1`); measured:
             * fvg_texture_copy_unclipped.expect.json's `ICMP_OR.u32.eq.m1` */
            AluOp::Ieq => self.op32(Alu::Icmp(Cmpf::Eq), [x, y, u], at),
            AluOp::B2f => self.op32(Alu::B2f, [x, u, u], at),
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
            AluOp::Iadd => self.op32(Alu::Iadd, [x, y, u], at),
            AluOp::Iand => self.op32(Alu::Iand, [x, y, u], at),
            AluOp::Ishr => self.op32(Alu::Ishr, [x, y, u], at),
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
            /* `!=`: no reference that compiles draws it (intuni_sharedint's
             * `mode != 0` waits for its `sin` and its vertex `if`) */
            AluOp::Fne | AluOp::Ine => Err(MaliError::CompareForm),
            _ => Err(MaliError::Arithmetic),
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
        let w = m.comps_of(ir::ValueId(id as u32)) as usize;
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
            let wn = m.comps_of(neg) as usize;
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
        /* the front end records no width for a conversion or a compare
         * (`comps_of` answers 4): the value has the lanes its sources
         * have, so a lane past the sources' ends is not one of its */
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
        origin: scratch.alloc_filled(MAX_NODES, 0u16).map_err(oom)?,
        keys: scratch.alloc_filled(MAX_VALUES, u32::MAX).map_err(oom)?,
        colour: None,
        moved: [Moved::NONE; MAX_MOVED],
        nmoved: 0,
        var_key: [u32::MAX; MAX_VARYINGS],
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
        prods: 0,
        look_ups: [TexLookup::NONE; MAX_LOOK_UPS],
        nlook: 0,
        rows: 0,
        npairs: 0,
        frag_xy: None,
        frag_zw: None,
        frag16: None,
        rank: [u32::MAX; MAX_LOCALS],
        colour_rank: u32::MAX,
        next_rank: 0,
        discards: 0,
        loops: 0,
        guard: u32::MAX,
        kept: None,
        kept_body: (0, 0),
        breaks: [BreakEdge::NONE; MAX_BREAKS],
        nbreaks: 0,
    };
    let vals = scratch.alloc_filled(MAX_VALUES, [Val::Undef; 4]).map_err(oom)?;
    let mut locals = [[Val::Undef; 4]; MAX_LOCALS];
    /* an `if` that ends `main` joins at the end of the program */
    let end = m.instrs.len().min(MAX_VALUES - 1);
    if walk_nodes(&mut f, m, m.nodes_of(m.body), vals, &mut locals, prec, true, end, None)? != Flow::Next {
        return Err(MaliError::ControlFlow);
    }
    order_keys(m, f.keys);
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
/// in ([`IfExit`]); the `if`'s own end is returned.
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
    if !prec.highp {
        return walk_if16(f, m, cond, then_, else_, exit, vals, locals, prec, join);
    }
    let Val::Lane(cn, _) = cond else { return Err(MaliError::IfForm) };
    if !matches!(f.nir.nodes[cn as usize].kind, Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_))) {
        return Err(MaliError::IfForm);
    }
    if f.nir.nifs >= nir::MAX_IFS {
        return Err(MaliError::BlockTableFull);
    }
    let before = f.nir.cur;
    let (saved_locals, saved_colour) = (*locals, f.colour);
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
    let (then_locals, then_colour) = (*locals, f.colour);
    *locals = saved_locals;
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
        f.breaks[f.nbreaks] = BreakEdge { lp, block: then_last, locals: then_locals, colour: then_colour };
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
    let mut vars = [(u32::MAX, usize::MAX); MAX_LOCALS + 1];
    let mut nvars = 0;
    for (l, lanes) in locals.iter().enumerate() {
        if brk.is_none() && (0..4).any(|c| then_locals[l][c] != lanes[c]) {
            vars[nvars] = (f.rank[l], l);
            nvars += 1;
        }
    }
    let colour = match (then_colour, f.colour) {
        _ if brk.is_some() => None,
        (Some(a), Some(b)) if a != b => {
            vars[nvars] = (f.colour_rank, MAX_LOCALS);
            nvars += 1;
            Some((a, b))
        }
        _ => None,
    };
    vars[..nvars].sort_unstable_by(|x, y| y.cmp(x));
    for &(_, l) in &vars[..nvars] {
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
            locals[l][c] = f.phi(then_locals[l][c], locals[l][c], at)?;
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
    if !f.mask_lane(cond) {
        return Err(MaliError::ControlFlow);
    }
    let (saved_locals, saved_colour) = (*locals, f.colour);
    let then_flow = walk_nodes(f, m, then_, vals, locals, prec, false, join, None)?;
    let then_locals = *locals;
    *locals = saved_locals;
    let then_colour = core::mem::replace(&mut f.colour, saved_colour);
    let else_flow = walk_nodes(f, m, else_, vals, locals, prec, false, join, None)?;
    if then_flow != Flow::Next || else_flow != Flow::Next || then_colour != f.colour {
        return Err(MaliError::ControlFlow);
    }
    let not = |f: &mut Fs<'_>, v: Val| f.int_lane(m, AluOp::Bnot, &[ir::Src::id(ir::ValueId(0)); 3], [v; 3], join);
    for (l, lanes) in locals.iter_mut().enumerate() {
        for (c, e) in lanes.iter_mut().enumerate() {
            let t = then_locals[l][c];
            *e = match (t, *e) {
                _ if t == *e => t,
                /* a side that leaves it undefined (`nir_opt_undef`) */
                (t, Val::Undef) => t,
                (Val::Undef, e) => e,
                (Val::Const(u32::MAX), Val::Const(0)) => cond,
                (Val::Const(0), Val::Const(u32::MAX)) => not(f, cond)?,
                (Val::Const(u32::MAX), e) if e == cond => cond,
                (t, Val::Const(0)) if t == cond => cond,
                _ => return Err(MaliError::ControlFlow),
            };
        }
    }
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
        if walk_nodes(f, m, lp.nodes, vals, locals, prec, false, lp.last, Some(next))? == Flow::Break {
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
    walk_if(f, m, c, &[Node::Break], rest, IfExit::Guard(lp), vals, locals, prec, lp.last)
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
    let r = walk_passes(f, m, Unroll { nodes, last, pass: 0 }, vals, locals, prec);
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
fn ir_folds(m: &Module<'_>, v: ir::ValueId, locals: &[[Val; 4]; MAX_LOCALS], depth: u32) -> bool {
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
fn declared_in(m: &Module<'_>, nodes: &[Node], l: usize, c: usize) -> bool {
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
    /* intloop_loop_nested: fp16 vectors as the header's phis, which the
     * mediump selection ([`select`], no blocks) does not make */
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
    let lp = f.nir.nloops as u8;
    f.nir.nloops += 1;
    let pre = f.nir.cur;
    let header = f.nir.new_block(pre)?;
    f.nir.cur = header;
    /* the header's phis, by the locals' first stores */
    let first_store = |l: usize| {
        m.instrs.iter().position(|op| matches!(*op, ir::Op::StoreLocal { local, .. } if local.0 as usize == l))
    };
    let mut order = [(usize::MAX, 0usize); MAX_LOCALS];
    let mut nl = 0;
    for (l, &mask) in masks.iter().enumerate() {
        if mask != 0 {
            order[nl] = (first_store(l).unwrap_or(usize::MAX), l);
            nl += 1;
        }
    }
    order[..nl].sort_unstable();
    let mut phis = [(0u16, 0usize, 0usize); MAX_LOCALS * 4];
    let mut nphi = 0;
    for &(_, l) in &order[..nl] {
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
                Val::Lane(n, _) => f.nir.nodes[n as usize].bits,
                _ => 32,
            };
            let id = f.nir.push_node(NirNode { kind: Kind::Phi, src: [v, Val::Undef, Val::Undef], bits, width: 1 })?;
            f.origin[id as usize] = last as u16;
            locals[l][c] = Val::Lane(id, 0);
            phis[nphi] = (id, l, c);
            nphi += 1;
        }
    }
    let (outer_kept, outer_body, outer_loops, outer_guard) = (f.kept, f.kept_body, f.loops, f.guard);
    let first_break = f.nbreaks;
    (f.kept, f.kept_body, f.loops, f.guard) = (Some(lp), (nodes.as_ptr() as usize, nodes.len()), 0, u32::MAX);
    let flow = walk_nodes(f, m, nodes, vals, locals, prec, false, last, None);
    (f.kept, f.kept_body, f.loops, f.guard) = (outer_kept, outer_body, outer_loops, outer_guard);
    /* a body that always breaks runs once: no reference */
    if flow? == Flow::Break {
        return Err(MaliError::LoopForm);
    }
    let latch = f.nir.cur;
    for &(id, l, c) in &phis[..nphi] {
        let v = locals[l][c];
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
        for b in f.breaks[first_break..f.nbreaks].iter_mut() {
            for v in b.locals.iter_mut().flatten().filter(|v| **v == from) {
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
    *locals = edges[0].locals;
    f.colour = edges[0].colour;
    if nedges == 2 {
        /* the exit's phis, a lane the edges leave different, in the order
         * an `if`'s join has them ([`walk_if`]), each taking the edges'
         * values in the order their jumps were added (divnest's outer exit:
         * `n` and `outer`, whose copies stand before each edge's `JUMP`) */
        if edges[1].colour != f.colour {
            return Err(MaliError::LoopForm);
        }
        let mut vars = [(u32::MAX, usize::MAX); MAX_LOCALS];
        let mut nvars = 0;
        for l in 0..MAX_LOCALS {
            if edges[0].locals[l] != edges[1].locals[l] {
                vars[nvars] = (f.rank[l], l);
                nvars += 1;
            }
        }
        vars[..nvars].sort_unstable();
        for &(_, l) in &vars[..nvars] {
            for c in 0..4 {
                locals[l][c] = f.phi(edges[0].locals[l][c], edges[1].locals[l][c], last)?;
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
                    let c = f.inverse_compare(c)?;
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
                 * constant ([`super::link::VaryingConsts`]): the load is of
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
             * mediump shader's read is narrowed to fp16 ([`Fs::frag_coord16`]);
             * a mediump ES 3.00 shader's highp read has no reference */
            /* a highp shader's z is the special varying's fp32 lane
             * ([`FRAG_Z`], loaded only when read, [`layout`]): Mesa reads it
             * at fp32 where a highp operation does, as the position's x and
             * y (`load_var_special_pan` is 32-bit, bifrost_compile.c:1428) */
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } if prec.highp => {
                out = f.frag_coord(at)?;
                out[2] = Val::Lane(f.input(FRAG_Z, at)?, 0);
            }
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } if !prec.frag_coord_highp => out = f.frag_coord16(at)?,
            ir::Op::LoadInput { slot: IoSlot::FragCoord, .. } => return Err(MaliError::FragCoordMediump),
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
                /* a texel stored to a (highp) local: the look-up returns
                 * fp32 ([`Fs::look_up`]) */
                if prec.highp {
                    for v in lanes {
                        if let Some((j, _)) = f.texel(v) {
                            let n = f.look_ups[j].node;
                            f.nir.nodes[n as usize].bits = 32;
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
                match slot {
                    IoSlot::FragColor(0) => {
                        if f.colour_rank == u32::MAX {
                            f.colour_rank = f.next_rank;
                            f.next_rank += 1;
                        }
                        f.colour = Some(lanes)
                    }
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
                let w = (m.comps_of(ir::ValueId(id.0)) as usize).min(4);
                let mut src = [[Val::Undef; 4]; 3];
                for (k, l) in src.iter_mut().enumerate().take(arity) {
                    for (i, v) in l.iter_mut().enumerate().take(w) {
                        *v = lane(vals, s[k], i);
                    }
                }
                /* the lanes an operand has (a compare's and a conversion's
                 * width is its sources', [`Fs::alu32_lanes`]) */
                let n = (0..w).take_while(|&i| src[0][i] != Val::Undef).count();
                let low = |f: &Fs<'_>, k: usize| fc[k] || (0..n).any(|i| f.is_f16(src[k][i]));
                let konst = |k: usize| (0..n).all(|i| matches!(src[k][i], Val::Const(_)));
                if (0..arity).any(|k| low(f, k)) && (0..arity).all(|k| low(f, k) || konst(k)) {
                    out = f.alu16(a, src, fc, n, at)?;
                } else {
                    out = f.alu32(m, a, &s, vals, at)?
                }
            }
            /* a product or sum on an fp32 value of a mediump shader (an
             * integer's conversion, case 79's `float(q.x) / 4.0`): lower_
             * precision leaves it at 32 bits, an operation the vertex stage
             * builds (intbig_case79.stderr: `fmul %124, 0.25` at 32 bits,
             * narrowed at the store) */
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s)
                if (0..m.comps_of(ir::ValueId(id.0)) as usize)
                    .any(|i| f.fp32_lane(lane(vals, s[0], i)) || f.fp32_lane(lane(vals, s[1], i))) =>
            {
                let op = if a == AluOp::Fmul { Alu::Fmul } else { Alu::Fadd };
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                /* an fp16 operand is widened (lower_precision's `f162f` at
                 * the edge of the lowered tree: case 42's `uv.x *
                 * float(sz.x)`, `F16_TO_F32` then `FMA.f32`) */
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (lane(vals, s[0], i), lane(vals, s[1], i));
                    let ok = |v: Val| f.fp32_lane(v) || f.is_f16(v) || matches!(v, Val::Const(_));
                    if !ok(x) || !ok(y) {
                        return Err(MaliError::MixedPrecision);
                    }
                    *o = f.op32(op, [x, y, Val::Undef], at)?;
                }
            }
            /* a sum or product of constants (an unrolled loop's `r +=
             * 0.25`, case 72): `nir_opt_constant_folding` at fp16, which
             * equals the fp32 result when that is exact in fp16; another
             * has no reference */
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s)
                if (0..m.comps_of(ir::ValueId(id.0)) as usize)
                    .all(|i| matches!((lane(vals, s[0], i), lane(vals, s[1], i)), (Val::Const(_), Val::Const(_)))) =>
            {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (Val::Const(p), Val::Const(q)) = (lane(vals, s[0], i), lane(vals, s[1], i)) else {
                        return Err(MaliError::Defect(Defect::IrIndex));
                    };
                    let (p, q) = (f32::from_bits(p), f32::from_bits(q));
                    let r = if a == AluOp::Fadd { p + q } else { p * q }.to_bits();
                    if [p.to_bits(), q.to_bits(), r].iter().any(|b| f16_exact(*b).is_none()) {
                        return Err(MaliError::ConstantFold);
                    }
                    *o = Val::Const(r);
                }
            }
            ir::Op::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                /* the rules run on 16-bit lane pairs: a rule holds for a
                 * pair when both its lanes' constants make it */
                let ident: &[u32] = if a == AluOp::Fmul { &[F32_ONE] } else { &[0, F32_NEG_ZERO] };
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
                    for i in lanes.clone() {
                        let (x, y) = (f.fp16_operand(lane(vals, s[0], i)), f.fp16_operand(lane(vals, s[1], i)));
                        out[i] = f.op(a, x, y, at, rule)?;
                    }
                }
            }
            /* a mediump compare: one 16-bit lane each (`flt`, `fge` with
             * GLSL's `<=` swapped, `feq`; the mask a discard tests,
             * [`Fs::discard`]) */
            ir::Op::Alu(a @ (AluOp::Flt | AluOp::Fle | AluOp::Feq), s) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let (x, y) = (f.fp16_operand(lane(vals, s[0], i)), f.fp16_operand(lane(vals, s[1], i)));
                    if x == Val::Undef || y == Val::Undef {
                        if i > 0 {
                            break;
                        }
                        return Err(MaliError::ColourForm);
                    }
                    if matches!((x, y), (Val::Const(_), Val::Const(_))) {
                        return Err(MaliError::ConstantFold);
                    }
                    *o = match a {
                        AluOp::Flt => f.node(Alu::Fcmp(Cmpf::Lt), [x, y, Val::Undef], at)?,
                        AluOp::Fle => f.node(Alu::Fcmp(Cmpf::Ge), [y, x, Val::Undef], at)?,
                        _ => f.node(Alu::Fcmp(Cmpf::Eq), [x, y, Val::Undef], at)?,
                    };
                }
            }
            ir::Op::Alu(a @ (AluOp::Fneg | AluOp::Fabs), s) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = f.fp16_operand(lane(vals, s[0], i));
                    *o = if a == AluOp::Fneg { f.neg(x, at)? } else { f.abs(x, at)? };
                }
            }
            /* the front end's `mix(a, b, t)` is `ffma(t, fadd(b, fneg(a)),
             * a)`, the `ffma` nir_lower_flrp makes too: case 197's vtl_fs,
             * `FADD.v2f16 r2, r2^, r0.neg` then `FMA.v2f16 r0, u2.w0.h00,
             * r2^, r0^` (tools/mali/testdata/vtl_fs.expect.json) */
            ir::Op::Alu(AluOp::Ffma, s) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
                for (i, o) in out.iter_mut().enumerate().take(w) {
                    let x = [0, 1, 2].map(|k| f.fp16_operand(lane(vals, s[k], i)));
                    if x.contains(&Val::Undef) {
                        return Err(MaliError::ColourForm);
                    }
                    *o = f.node(Alu::Ffma, x, at)?;
                }
            }
            /* the integer and boolean operations of a mediump shader, as
             * 32-bit values ([`Fs::int_lane`]): folded where the constants
             * decide them (the index of a uniform array element: `int(t.w
             * * 0.0)` is the constant 0 and `fidx + 0` is `fidx`, case 122's
             * program has neither an F32_TO_S32 nor an IADD, uniidx_i), else
             * the compares and conversions the references draw */
            ir::Op::Alu(a, s) if int_op(a) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
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
            ir::Op::Alu(AluOp::Bcsel, s) if f.mask_lane(lane(vals, s[0], 0)) => {
                let w = m.comps_of(ir::ValueId(id.0)) as usize;
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
                 * it, read as it is, intbig_case112.expect.json) */
                let int = decl.is_some_and(|u| matches!(u.gl_type as u32, GL_INT | GL_BOOL) && u.array_len <= 1);
                /* a float array's element at a constant index: no reference
                 * draws it (case108_uniidx reads at run-time indices only) */
                if decl.is_some_and(|u| u.gl_type as u32 == GL_FLOAT && u.array_len > 1) {
                    return Err(MaliError::UniformArray);
                }
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = f.uniform_lane(dword_off + c as u16, if int { 32 } else { 16 }, at)?;
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
                let shape = match u.gl_type as u32 {
                    GL_FLOAT => comps == 1 && !vec4,
                    _ => comps == 4 && f.nubo == 0,
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
             * level (`textureCube`'s `COMPUTED_LOD`) has no reference */
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
                let cube = Some(Cube { explicit });
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
            /* `textureSize` of a 2D array at level 0 in a mediump shader
             * (case 42): `pan_nir_load_va_tex_size` (pan_nir.h:106-140),
             * the descriptor's first eight bytes ([`Kind::TexDesc`]), its
             * width and height halves `u2u32` plus one, each selected
             * against 0 by the null test of its first half (`ieq_pan` at 16
             * bits, the `if` NIR flattens to `bcsel_pan`), in
             * intbig_case42.stderr's order (`%7`..`%18`). The layer count
             * (byte 24) is read by no reference */
            ir::Op::TexSize { sampler, dim: ir::TexDim::D2Array, lod } if !prec.highp => {
                if lane(vals, lod, 0) != Val::Const(0) {
                    return Err(MaliError::TexKind);
                }
                let u = Val::Undef;
                let d = NirNode { kind: Kind::TexDesc { sampler }, src: [u; 3], bits: 32, width: 2 };
                let d = f.nir.push_node(d)?;
                f.origin[d as usize] = at as u16;
                let w = f.int32(Alu::Extract16(0), [Val::Lane(d, 1), u, u], at)?;
                let h = f.int32(Alu::Extract16(1), [Val::Lane(d, 1), u, u], at)?;
                let w = f.int32(Alu::Iadd, [w, Val::Const(1), u], at)?;
                let h = f.int32(Alu::Iadd, [h, Val::Const(1), u], at)?;
                let null = NirNode {
                    kind: Kind::Alu(Alu::Icmp(Cmpf::Eq)),
                    src: [Val::Lane(d, 0), Val::Const(0), u],
                    bits: 16,
                    width: 1,
                };
                let null = Val::Lane(f.nir.push_node(null)?, 0);
                if let Val::Lane(n, _) = null {
                    f.origin[n as usize] = at as u16;
                }
                out[0] = f.int32(Alu::Bcsel, [null, Val::Const(0), w], at)?;
                out[1] = f.int32(Alu::Bcsel, [null, Val::Const(0), h], at)?;
            }
            ir::Op::Tex { sampler, coord, .. } => {
                if !matches!(op.es100_tex(), Some((Es100Tex::Tex2D, None))) {
                    return Err(MaliError::TexKind);
                }
                let (s, t) = (lane(vals, coord, 0), lane(vals, coord, 1));
                let coord = match (f.input_lane(s), f.input_lane(t), s, t) {
                    (Some((v, 0)), Some((w, 1)), ..) if v == w && v != POINT_COORD => {
                        /* a vec2 varying read whole (the references' coordinate) */
                        if m.inputs.iter().find(|d| d.slot() == IoSlot::Varying(v)).map(|d| d.components) != Some(2) {
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
            ir::Op::TexSize { .. } => return Err(MaliError::TexKind),
            /* a `discard` outside `if (c) discard;` (unconditional, or in a
             * block with other statements): `demote`'s `DISCARD.f32 0, 0
             * .eq`, which no reference draws */
            ir::Op::Discard => return Err(MaliError::DiscardForm),
        }
        vals[id.0 as usize] = out;
    }
    Ok(Flow::Next)
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
fn ir_uses(m: &Module<'_>, v: ir::ValueId) -> usize {
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
/// vertex stage reads it (`nir::uniform_of`), an array of vec4 (femtovg's
/// `frag[14]`: `array_len` elements of 4 dwords each), or an array of float
/// (case108_uniidx's `u_arr[4]`: one dword an element, Mesa's packed
/// stride). Any other array has no reference yet
/// ([`MaliError::UniformArray`]).
pub fn uniform_extent<'m>(m: &Module<'m>, dword: u16) -> Result<(&'m ir::UniformDecl<'m>, u16), MaliError> {
    let u = m
        .uniforms
        .iter()
        .find(|u| {
            u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK
                && dword >= u.dword_off
                && dword < u.dword_off + u.dwords.max(1)
        })
        .ok_or(MaliError::Defect(Defect::IrIndex))?;
    if u.array_len > 1 {
        let n = u16::from(u.array_len);
        let fits = match u.gl_type as u32 {
            crate::gl::consts::GL_FLOAT_VEC4 => u.dwords == 4 * n,
            GL_FLOAT => u.dwords == n,
            _ => false,
        };
        if !fits {
            return Err(MaliError::UniformArray);
        }
        return Ok((u, u.dwords));
    }
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
    /* a highp float array: no reference (its element is one dword) */
    if u.array_len > 1 && u.gl_type as u32 == GL_FLOAT {
        return Err(MaliError::UniformArray);
    }
    Ok(if total == 16 || u.array_len > 1 {
        (u.dword_off + (dword - u.dword_off) / 4 * 4, 4)
    } else {
        (u.dword_off, total)
    })
}

/// `nir_opt_shrink_vectors` for the uniform loads of a highp shader: each
/// keeps the lanes up to the highest one read (the vertex stage's `shrink`,
/// nir.rs; a load the colour alone reads counts its stores, pushed before
/// this runs). A vec4 array's element also loses the lanes below the
/// lowest one read, its offset moved up (`shrink_dest_to_read_mask` with
/// `shrink_start`, nir_opt_shrink_vectors.c:82-140): measured on femtovg's
/// `frag[12].w` and `frag[10].x`, one FAU word each
/// (fvg_fill_color_nodiscard.expect.json), where a plain vec4's `u.zw` in
/// a vertex shader keeps x and y (sel_u_zw); a plain fragment uniform read
/// from its second lane on has no reference and keeps the vertex stage's
/// form.
fn shrink_uniforms(f: &mut Fs<'_>, m: &Module<'_>) {
    for k in 0..f.nir.len {
        let id = f.nir.order[k];
        let Kind::Uniform { dword, .. } = f.nir.nodes[id as usize].kind else { continue };
        let (mut low, mut top) = (4u8, 0u8);
        for (u, s) in f.nir.uses(id) {
            if let Val::Lane(_, c) = f.nir.nodes[u as usize].src[s] {
                top = top.max(c + 1);
                low = low.min(c);
            }
        }
        let element = dword != FB_WPOS_Y_TRANSFORM && uniform_extent(m, dword).is_ok_and(|(u, _)| u.array_len > 1);
        if !element || low == 0 || low >= top {
            f.nir.nodes[id as usize].kind = Kind::Uniform { dword, comps: top };
            continue;
        }
        f.nir.nodes[id as usize].kind = Kind::Uniform { dword: dword + u16::from(low), comps: top - low };
        for j in 0..f.nir.len {
            let u = f.nir.order[j] as usize;
            for s in f.nir.nodes[u].src.iter_mut() {
                if let Val::Lane(n, c) = *s {
                    if n == id {
                        *s = Val::Lane(n, c - low);
                    }
                }
            }
        }
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

/// The varyings the colour reads, laid out and compacted as Mesa's link
/// does, in the order of their first reads. The layout takes them in
/// declaration order, or in the program's vertex shader's order when the
/// link compiled this shader for it (`order`, link.rs `VaryingOrder`:
/// Mesa's fragment shader has the vertex shader's layout).
pub fn layout(f: &mut Fs<'_>, m: &Module<'_>, order: Option<super::link::VaryingOrder>) -> Result<(), MaliError> {
    if m.outputs.iter().any(|o| o.kind != IoKind::Colour || o.location != 0) {
        return Err(MaliError::FragOutput);
    }
    let c = f.colour.ok_or(MaliError::NoColour)?;
    if c.contains(&Val::Undef) {
        return Err(MaliError::ColourForm);
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
            /* so is one a mediump shader makes an integer of, through its
             * fp16 lanes (measured: intloop_int_compare.expect.json and
             * intloop_tri_int.expect.json load `.v4` and their vertex
             * programs store four lanes of `v_color`, of which the colour
             * reads two and three; the cmp_* shaders, which read lanes as
             * floats, are compacted) */
            whole[loc as usize] |= !f.prec.highp
                && order.iter().any(|&id| {
                    let nd = f.nir.nodes[id as usize];
                    let from_lane = |v: Val| match f.alu_of(v) {
                        Some((op, s)) if !op.is_int() => {
                            s[..op.arity()].iter().any(|x| matches!(*x, Val::Lane(n, _) if n == node))
                        }
                        _ => false,
                    };
                    nd.kind == Kind::Alu(Alu::F2i32)
                        && from_lane(nd.src[0])
                        && c.iter().any(|&root| reaches(f, root, Val::Lane(id, 0)))
                });
        }
    }
    /* a matrix varying of columns narrower than a vec4 is read whole, every
     * column, when any lane of it is read: the GLSL linker packs its
     * columns into one slot (`packed:v_m[0],v_m[1]`), which the link then
     * sees as read whole (measured: varyconst_78_matvary.expect.json, a
     * mat2 of which the colour reads v_m[0].x and v_m[1].y, stores and
     * loads all four lanes) */
    fn root<'a>(d: &ir::IoDecl<'a>) -> &'a str {
        d.name.split('[').next().unwrap_or(d.name)
    }
    let packed = |d: &&ir::IoDecl<'_>| d.kind == IoKind::Varying && is_matrix(d.gl_type) && d.components < 4;
    for d in m.inputs.iter().filter(packed) {
        let any =
            m.inputs.iter().filter(packed).any(|e| root(e) == root(d) && read.get(e.location as usize) > Some(&0));
        if any {
            let loc = d.location as usize;
            *read.get_mut(loc).ok_or(MaliError::VaryingLocation)? |= (1 << d.components) - 1;
            whole[loc] = true;
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
    /* the descriptor index of each: its sampler's rank among the samplers
     * the program reads, by binding (the declaration order; [`TEX_INDEX`]) */
    for j in 0..f.nlook {
        let b = f.look_ups[j].sampler;
        let below = (0..b).filter(|&x| f.look_ups().iter().any(|t| t.live() && t.sampler == x)).count();
        f.look_ups[j].index = below as u8;
    }
    let decls = m.inputs.iter().filter(|d| d.kind == IoKind::Varying && read.get(d.location as usize) > Some(&0));
    for d in decls.clone() {
        if d.interp != ir::INTERP_SMOOTH {
            return Err(MaliError::Interpolation);
        }
    }
    /* (rank, location, components): a stable insertion by the vertex rank */
    let mut by = [(0u8, 0u8, 0u8); MAX_VARYINGS];
    let mut nd = 0;
    for d in decls {
        let rank = order.map_or(0, |o| o.rank(d.location));
        if nd >= MAX_VARYINGS {
            return Err(MaliError::VaryingLocation);
        }
        let at = by[..nd].iter().position(|x| x.0 > rank).unwrap_or(nd);
        by.copy_within(at..nd, at + 1);
        by[at] = (rank, d.location, d.components);
        nd += 1;
    }
    let mut lay = [VaryingAt { location: 0, comps: 0, offset: 0 }; MAX_VARYINGS];
    let n = generic_layout(by[..nd].iter().map(|&(_, l, c)| (l, c)), &mut lay)?;
    let mut slots = [FsVarying::NONE; MAX_VARYINGS];
    /* `compact_varyings`: every component read, in layout order, takes the
     * next scalar slot from VAR0 */
    let mut s = 0u16;
    /* (a flat lane read, an interpolated lane read): the link packs flat
     * lanes in slots of their own, a layout no reference draws beside
     * interpolated ones (nir.rs refuses the pair's vertex shader too) */
    let (mut flat, mut smooth) = (false, false);
    let mut kept = [false; MAX_VARYINGS];
    for at in &lay[..n] {
        let loc = at.location as usize & 15;
        let first = s;
        if whole[loc] {
            read[loc] = (1 << at.comps) - 1;
        }
        for comp in 0..at.comps {
            if read[loc] & (1 << comp) == 0 {
                continue;
            }
            if f.prec.consts.flat(at.location, comp) {
                flat = true;
            } else {
                smooth = true;
            }
            let (slot, lane) = ((s / 4) as usize, (s % 4) as u8);
            let x = slots.get_mut(slot).ok_or(MaliError::VaryingLocation)?;
            let key = if lane == 0 { f.var_key[loc] } else { x.key().min(f.var_key[loc]) };
            *x = FsVarying::new(slot as u8, lane + 1, 16 * slot as u16, key);
            f.lane_of[loc][comp as usize] = Some((slot as u8, lane));
            kept[slot] |= whole[loc];
            s += 1;
        }
        f.compacted |= read[loc] != (1 << at.comps) - 1;
        f.reads[f.nreads] = FsRead { location: at.location, mask: read[loc], offset: 4 * first };
        f.nreads += 1;
    }
    if flat && smooth {
        return Err(MaliError::VaryingNotFromAttribute);
    }
    let nslots = s.div_ceil(4) as usize;
    for x in &mut slots[..nslots] {
        x.flat = flat;
    }
    for (x, k) in slots[..nslots].iter_mut().zip(kept) {
        x.whole = k;
    }
    let pc = FsVarying::new(POINT_COORD, 2, 0, f.pcoord_key);
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
            if k != k1 || (x, y) != (0, 1) {
                return Err(MaliError::TexCoord);
            }
            f.look_ups[j].load = Some(k);
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
    /// Word `h` of the uniform array element read at a run-time index
    /// (`LD_PKA.i64`'s two words, uniidx_i).
    Ubo(u8),
    /// Component `c` of that element alone, its half of word `c / 2` in
    /// both lanes (a one-lane operation's source, [`coord_lane`]).
    UboLane(u8),
    /// Read `k` of a float array at a run-time index ([`Fs::ubo_idx`]):
    /// `LD_PKA.i16`'s half, one lane ([`Pairs::scalar`]).
    UboHalf(u8),
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
            P::Leaf(Leaf::Tex(..) | Leaf::TexLane(..)) => Form::Tex,
            P::Leaf(Leaf::Ubo(_) | Leaf::UboLane(_) | Leaf::UboHalf(_)) => Form::Ubo,
            /* `MKVEC.v2i16 x, #0`: a lane and a constant half */
            P::Leaf(Leaf::Zext(_) | Leaf::Lane { .. }) => Form::Mk,
            /* `MKVEC.v2i16 x, y` of the position's two halves */
            P::Leaf(Leaf::Frag) => Form::Mk,
            P::Leaf(Leaf::Const(_)) => Form::Const,
            P::Op(Alu::Fneg, _) => Form::Neg,
            P::Op(Alu::Fabs, _) => Form::Abs,
            P::Op(..) => Form::Op,
        }
    }

    /// The productions of the tree under `x` (`at`: where it stands).
    fn note(&self, x: u8, at: Place, prods: &mut u128) {
        *prods |= prod(self.form(x), at);
        if let P::Op(op, s) = self.nodes[x as usize] {
            let places: &[Place] = match op {
                Alu::Fadd => &[Place::AddA, Place::AddB],
                Alu::Fmul => &[Place::MulA, Place::MulB],
                Alu::Ffma => &[Place::FmaA, Place::FmaB, Place::FmaC],
                Alu::Fneg => &[Place::NegSrc],
                _ => &[Place::AbsSrc],
            };
            for (k, p) in places.iter().enumerate() {
                self.note(s[k], *p, prods);
            }
        }
    }
}

/// `nir_opt_vectorize` of lanes `a` and `b` (one pair's low and high
/// lane): the two trees match operation for operation, their leaves pair
/// up as [`Leaf`] names them; the late fusion where an fadd stands.
fn vectorise(f: &mut Fs<'_>, pr: &mut Pairs, a: Val, b: Val, pair: u8, ubo: &UboLayout) -> Result<u8, MaliError> {
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
            match (pr.nodes[x as usize], pr.nodes[y as usize]) {
                (P::Op(Alu::Fmul, [m0, m1, _]), _) => P::Op(Alu::Ffma, [m0, m1, y]),
                (_, P::Op(Alu::Fmul, [m0, m1, _])) => P::Op(Alu::Ffma, [m0, m1, x]),
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
        /* a look-up's result in its own pairs, in order */
        _ if f.texel(a).is_some() || f.texel(b).is_some() => match (f.texel(a), f.texel(b)) {
            (Some((t, i)), Some((u, j))) if t == u && i % 2 == 0 && j == i + 1 => P::Leaf(Leaf::Tex(t as u8, i / 2)),
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
                } else if i == j && comps == 1 {
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
                /* `MKVEC.v2i16 x, #0` is `U16_TO_U32 x` (bi_lower_mkvec_swz.c:256-257) */
                if half == 0 {
                    return Err(MaliError::ColourZeroHalf);
                }
                let (k, c) = index(f, v, i)?;
                P::Leaf(Leaf::Mk { k, c, half })
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
                (None, None, Val::Const(_), _) if f.input_lane(b).is_some() => return Err(MaliError::ColourPair),
                _ => return Err(MaliError::ColourForm),
            },
        },
    };
    pr.push(p, place)
}

/// Lane `v` is fadds whose leaves are a float array's indexed reads
/// ([`Fs::ubo_read`]), the form [`scalar`] takes.
fn sum_of_reads(f: &Fs<'_>, v: Val) -> bool {
    match f.alu_of(v) {
        Some((Alu::Fadd, s)) => s[..2].iter().all(|&x| sum_of_reads(f, x)),
        Some(_) => false,
        None => f.ubo_read(v).is_some(),
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
        _ => return Err(MaliError::ColourForm),
    };
    let x = pr.push(p, place)?;
    pr.scalar[x as usize] = true;
    Ok(x)
}

/// An operand of a mediump discard's compare as a leaf of one lane and the
/// half it is read through (`bi_alu_src_index` of a one-lane 16-bit op
/// replicates the lane): a component of an fp16 uniform (`u2.w0.h11`,
/// alu_branchlane) or a constant's half. A varying's lane is refused
/// ([`MaliError::DiscardForm`]): Mesa converts it alone (`FADD.f32 r0.h0,
/// r1^, 0x0.neg`), but alu_branchquad also keeps the mediump varying whole
/// (`.v4`, four vertex stores) where the read lanes are y and w, a link
/// rule (no compaction of a mediump varying the GL path reads in part) no
/// other reference pins; any other operand has no reference either.
fn cmp_leaf(f: &Fs<'_>, pr: &mut Pairs, v: Val, ubo: &UboLayout) -> Result<(u8, Swz), MaliError> {
    let (leaf, swz) = match (f.alu_of(v), f.input_lane(v), f.uniform_dword(v), v) {
        (_, _, _, Val::Const(c)) => (Leaf::Const(u32::from(f16_rtne(c).ok_or(MaliError::Fp16Inexact)?)), Swz::H00),
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
    if half == 0 {
        return Err(MaliError::ColourZeroHalf);
    }
    let x = lane_tree(f, pr, a, pair, ubo)?;
    pr.push(P::Leaf(Leaf::Lane { x, half }), (f.key(a), pair)).map(Some)
}

/// Lane `v` is sums and products of fp16 texels and constants, with a
/// texel among them, or sums of a uniform's component, negated or not,
/// and constants, with a uniform among them ([`lane_tree`]'s form;
/// varyconst_75_immneg's `1.0 - u_k.z`).
fn lone_tree(f: &Fs<'_>, v: Val) -> bool {
    /* (a texel, a uniform) among the leaves */
    fn walk(f: &Fs<'_>, v: Val) -> Option<(bool, bool)> {
        let or = |x: (bool, bool), y: (bool, bool)| (x.0 | y.0, x.1 | y.1);
        match (f.alu_of(v), f.texel(v), v) {
            (Some((Alu::Fadd | Alu::Fmul, s)), ..) => Some(or(walk(f, s[0])?, walk(f, s[1])?)),
            (Some((Alu::Fneg, s)), ..) => walk(f, s[0]),
            (None, Some((j, _)), _) if f.tex_f16(j) => Some((true, false)),
            (None, None, Val::Const(_)) => Some((false, false)),
            (None, None, _) if f.uniform_dword(v).is_some() => Some((false, true)),
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
    match walk(f, v) {
        Some((true, false)) => true,
        Some((false, true)) => uniform_form(f, v),
        _ => false,
    }
}

/// Lane `v` of the colour alone ([`lane_word`]) as nodes of one lane: the
/// fmuls and fadds of texels and constants, an fadd of a product fused as
/// [`vectorise`] fuses it (texmore_2.stderr: `%9 = fmul %6.z, 0.25`,
/// `%12 = ffma %4.z, 0.5, %9`; `FMA.v2f16 %35^.h00, u268[1].h11, u256.neg`
/// then `FMA.v2f16 %27^.h00, u269.h11, %9^.h00`).
fn lane_tree(f: &Fs<'_>, pr: &mut Pairs, v: Val, pair: u8, ubo: &UboLayout) -> Result<u8, MaliError> {
    let place = (f.key(v), pair);
    let p = match (f.alu_of(v), f.texel(v), v) {
        (Some((Alu::Fmul, s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            P::Op(Alu::Fmul, [x, y, 0])
        }
        /* an fneg of a uniform's component, a source modifier of its
         * reader (75: `FADD.v2f16 r1, 0x3C000000.h11, u2.w1.neg.h00`) */
        (Some((Alu::Fneg, s)), ..) => P::Op(Alu::Fneg, [lane_tree(f, pr, s[0], pair, ubo)?, 0, 0]),
        (Some((Alu::Fadd, s)), ..) => {
            let x = lane_tree(f, pr, s[0], pair, ubo)?;
            let y = lane_tree(f, pr, s[1], pair, ubo)?;
            match (pr.nodes[x as usize], pr.nodes[y as usize]) {
                (P::Op(Alu::Fmul, [m0, m1, _]), _) => P::Op(Alu::Ffma, [m0, m1, y]),
                (_, P::Op(Alu::Fmul, [m0, m1, _])) => P::Op(Alu::Ffma, [m0, m1, x]),
                _ => P::Op(Alu::Fadd, [x, y, 0]),
            }
        }
        (None, Some((j, c)), _) => P::Leaf(Leaf::TexLane(j as u8, c)),
        /* a product with 2.0 is the late rules' `fadd(a, a)`: no reference */
        (None, None, Val::Const(F32_TWO)) => return Err(MaliError::AlgebraicForm),
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
            let two = |x: u8| pr.nodes[x as usize] == P::Leaf(Leaf::Const(F16X2_TWO));
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
        P::Leaf(Leaf::Zext(y) | Leaf::Lane { x: y, .. }) => mark(pr, y, live),
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
    };
    let mut roots = [0u8; 2];
    let c = f.colour.ok_or(MaliError::NoColour)?;
    for (k, r) in roots.iter_mut().enumerate() {
        *r = match lane_word(f, &mut pr, c[2 * k], c[2 * k + 1], k as u8, ubo)? {
            Some(x) => x,
            None => vectorise(f, &mut pr, c[2 * k], c[2 * k + 1], k as u8, ubo)?,
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
     * u3.w0, r1^`), z a lane alone (`u2.w1.h0`), converted at the look-up */
    let cube = match lone {
        Some(TexLookup { cube: Some(_), node, .. }) => {
            let nd = f.nir.nodes[node as usize];
            let xy = vectorise(f, &mut pr, nd.src[0], nd.src[1], 0, ubo)?;
            let z = cube_lane(f, &mut pr, nd.src[2], ubo)?;
            Some((xy, z))
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
    /* a load holds the lanes read, and those alone but where the link
     * keeps a varying whole ([`layout`]'s packed matrix: 78's v4 load, of
     * which lanes 0 and 3 are read) */
    for (k, v) in f.varyings[..f.nvar].iter().enumerate() {
        let full = (1u8 << v.comps) - 1;
        if used[k] == 0 || used[k] & !full != 0 || (used[k] != full && !v.whole) {
            return Err(MaliError::Defect(Defect::IrIndex));
        }
    }
    let mut live = [false; MAX_PAIRS];
    let cube_nodes = cube.map(|(xy, z)| [Some(xy), z.node()]).into_iter().flatten().flatten();
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
    let mut events =
        [(0u32, 0u8, LOAD, 0u8); MAX_VARYINGS + MAX_PAIRS + MAX_LOOK_UPS + 2 + 2 * MAX_UBO_READS + MAX_DISCARDS];
    let mut ne = 0;
    for (k, d) in discards[..nd].iter().enumerate() {
        events[ne] = (d.0, 0, DISCARD, k as u8);
        ne += 1;
    }
    for (k, v) in f.varyings[..f.nvar].iter().enumerate() {
        events[ne] = (v.key(), 0, LOAD, k as u8);
        ne += 1;
    }
    /* the narrowed position, where gl_FragCoord is read (its fp32 build,
     * the narrowings and the word: [`frag16`]) */
    if let (Some([x, _]), true) = (f.frag16, (0..pr.n).any(|i| live[i] && pr.nodes[i] == P::Leaf(Leaf::Frag))) {
        events[ne] = (f.key(x), 0, FRAG, 0);
        ne += 1;
    }
    let key_of = |at: u16| f.keys.get(at as usize).copied().ok_or(MaliError::Defect(Defect::IrIndex));
    for (j, t) in f.live_look_ups() {
        events[ne] = (key_of(t.at)?, 0, TEX, j as u8);
        ne += 1;
    }
    /* the indexed vec4 element the colour reads (its lanes in a live pair),
     * where its IR instruction stands */
    let vec4 = f.nubo == 1 && f.ubo_comps(f.ubo_idx[0]) == 4;
    if vec4 && pr.nodes[..pr.n].iter().any(|x| matches!(x, P::Leaf(Leaf::Ubo(_) | Leaf::UboLane(_)))) {
        events[ne] = (key_of(f.origin[f.ubo_idx[0] as usize])?, 0, UBO, 0);
        ne += 1;
    }
    /* each float array read the colour reads: its offset where its IR
     * instruction stands, its load moved down to just before its first
     * reader (`nir_opt_move` of `load_ubo`, bifrost_nir.c; case108_uniidx:
     * the offsets of the first two reads, then both loads, then their sum) */
    for k in 0..f.nubo.min(MAX_UBO_READS) {
        let read = |i: usize| pr.nodes[i] == P::Leaf(Leaf::UboHalf(k as u8));
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
    events.sort_unstable_by_key(|e| (e.0, e.1, e.2, e.3));
    let mut lv = Leaves {
        conv: [[None; 4]; MAX_VARYINGS],
        raw: [[None; 4]; MAX_VARYINGS],
        tex: [[None; 4]; MAX_LOOK_UPS],
        ubo: *ubo,
        ubo_words: [None; 2],
        ubo_off: [None; MAX_UBO_READS],
        ubo_half: [None; MAX_UBO_READS],
        ubo_lanes: [[None; 4]; MAX_UBO_READS],
        desc: None,
        frag: None,
    };
    let mut val = [None::<Src>; MAX_PAIRS];
    let mut rows = 0u128;
    let mut texel = None;
    for &(_, _, what, x) in events.iter() {
        match what {
            LOAD => convert(p, f, x as usize, &conv_form, &mut lv, &mut rows)?,
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
                     * `u2.w1.h0`), then the descriptor */
                    (None, Some((xy, z))) => {
                        let w = operand(&pr, xy, &lv, &val)?;
                        let x = f2f32(p, &mut rows, w.swz(Swz::H00))?;
                        let y = f2f32(p, &mut rows, w.swz(Swz::H11))?;
                        let z = match z {
                            CubeLane::F32(c) => Src::of(Idx::Const(c)),
                            CubeLane::Node(z, swz) => f2f32(p, &mut rows, operand(&pr, z, &lv, &val)?.swz(swz))?,
                        };
                        let st = cube_rows(p, &mut rows, [x, y, z])?;
                        staging[..2].copy_from_slice(&st);
                        2
                    }
                    (None, None) => 0,
                };
                tex(p, f, x as usize, &mut lv, (n > 0).then_some(&staging[..n]))?;
                /* a colour that is the texel whole: `bi_emit_tex_valhall`
                 * collects the split words into the look-up's value right
                 * after it (bifrost_compile.c), so its moves stand before
                 * ATEST (case41.expect.json: `MOV.i32 r0, r2^`, `MOV.i32
                 * r1, r3`, then `ATEST ... r3^.h1`) */
                if roots.map(|r| pr.nodes[r as usize]) == [P::Leaf(Leaf::Tex(x, 0)), P::Leaf(Leaf::Tex(x, 1))] {
                    let mut w = [Src::NULL; 2];
                    for (k, x) in w.iter_mut().enumerate() {
                        *x = Src::ssa(alu(p, Op::MovI32, &[operand(&pr, roots[k], &lv, &val)?])?);
                    }
                    texel = Some(alu(p, Op::Collect, &w)?);
                }
            }
            FRAG => lv.frag = Some(frag16(p, &lv, &mut rows)?),
            UBO if vec4 => ubo_load(p, f, &mut lv, &mut rows)?,
            OFFSET => ubo_offset(p, f, x as usize, &mut lv, &mut rows)?,
            UBO => ubo_fetch(p, x as usize, &mut lv)?,
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
    /* ATEST on the alpha: the constant's half, or the high half of the
     * second pair's word (a uniform word's own half when it is a splat) */
    let at = match (alpha, pr.nodes[roots[1] as usize]) {
        (Some(h), _) => Src::of(Idx::Const(h as u32)).swz(Swz::H00),
        (None, P::Leaf(Leaf::Uniform { base, comp, swz })) => {
            Src::of(Idx::Ubo { base, comp }).swz(if swz == Swz::H00 { Swz::H00 } else { Swz::H11 })
        }
        (None, _) => operand(&pr, roots[1], &lv, &val)?.swz(Swz::H11),
    };
    let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), at, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    let mut words = [Src::NULL; 2];
    for (k, w) in words.iter_mut().enumerate().filter(|_| texel.is_none()) {
        *w = match pr.nodes[roots[k] as usize] {
            P::Leaf(Leaf::Mk { k: v, c, half }) => Src::ssa(mkvec(p, &lv, v, c, half, &mut rows)?),
            P::Leaf(Leaf::Lanes { k: v, c, e, d }) => Src::ssa(mkvec_lanes(p, &lv, [(v, c), (e, d)], &mut rows)?),
            P::Leaf(Leaf::Zext(x)) => {
                /* a read the word alone reads is loaded here, after ATEST
                 * (`nir_opt_move` sinks it to the colour's store; uniarray_b:
                 * `ATEST`, then `LD_PKA.i16`, then `U16_TO_U32`) */
                if let P::Leaf(Leaf::UboHalf(r)) = pr.nodes[x as usize] {
                    if lv.ubo_half.get(r as usize).copied().flatten().is_none() {
                        ubo_fetch(p, r as usize, &mut lv)?;
                    }
                }
                let s = [operand(&pr, x, &lv, &val)?];
                Src::ssa(row(p, &mut rows, Row { op: Alu::U2u32, bits: 32, width: 1 }, &s)?)
            }
            P::Leaf(Leaf::Lane { x, half }) => {
                let s = [operand(&pr, x, &lv, &val)?.swz(Swz::H00), Src::of(Idx::Const(half as u32)).swz(Swz::H00)];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
            P::Leaf(Leaf::Const(c)) => Src::of(Idx::Const(c)),
            _ => Src::ssa(alu(p, Op::MovI32, &[operand(&pr, roots[k], &lv, &val)?])?),
        };
    }
    let col = match texel {
        Some(c) => c,
        None => alu(p, Op::Collect, &words)?,
    };
    alu(
        p,
        Op::Blend { sr_count: 2, f32: false, u32: false },
        &[
            Src::ssa(col),
            Src::ssa(t),
            Src::of(Idx::Special(Special::BlendDescriptor0, false)),
            Src::of(Idx::Special(Special::BlendDescriptor0, true)),
            Src::NULL,
        ],
    )?;
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
                let nd = f.nir.nodes[f.nir.order[e.3 as usize] as usize];
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
                } else if x / 2 == y / 2 {
                    /* one aligned pair the other way round: vectorised too,
                     * not drawn by a reference */
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
            /* an fp16 texel of a 2D array look-up: its half of the register
             * the write mask packs it in ([`tex_mask`]) */
            Kind::Tex if f.look_up_of(n).is_some_and(|j| f.look_ups[j].array) => {
                let j = f.look_up_of(n).ok_or(defect)?;
                let mask = tex_mask(&f.look_ups[j], true);
                let at = (mask & ((1u8 << c) - 1)).count_ones() as usize;
                let w = lv.tex.get(j).and_then(|w| w[(at / 2) & 3]).ok_or(defect)?;
                Ok(Src::ssa(w).swz(if at.is_multiple_of(2) { Swz::H00 } else { Swz::H11 }))
            }
            /* a descriptor word ([`tex_desc`]) */
            Kind::TexDesc { .. } => lv.desc.map(|d| Src::ssa(d[c as usize & 1])).ok_or(defect),
            _ => Err(MaliError::MixedPrecision),
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
    let int = |op: Alu| matches!(op, Alu::Icmp(_) | Alu::B2f | Alu::Bcsel | Alu::F2i32 | Alu::Inot);
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
    f.colour.is_some_and(|c| c.iter().all(|v| lane(v) || fp32(v)) && !c.iter().all(|v| matches!(v, Val::Const(_))))
        && pairs.is_some()
        && nodes.iter().enumerate().all(|(id, nd)| match nd.kind {
            Kind::Preload { .. } | Kind::Uniform { .. } => true,
            Kind::Input { loc, .. } => (loc as usize) < MAX_VARYINGS && nd.bits == 32,
            Kind::Alu(op) => (nd.bits == 32 && (int(op) || (divides && arith(op)))) || paired(id),
            _ => false,
        })
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
pub fn select_highp(
    p: &mut Prog<'_>,
    f: &mut Fs<'_>,
    m: &Module<'_>,
    ubo: &UboLayout,
    scratch: &Arena<'_>,
) -> Result<(), MaliError> {
    let oom = |_| MaliError::OutOfScratch;
    let defect = MaliError::Defect(Defect::IrIndex);
    f.ubo = *ubo;
    let c = f.colour.ok_or(MaliError::NoColour)?;
    /* the colour's stores: the fusion counts them as uses */
    for (k, v) in c.iter().enumerate() {
        let st = NirNode {
            kind: Kind::Store { out: 0, word: k as u8 },
            src: [*v, Val::Undef, Val::Undef],
            bits: 32,
            width: 1,
        };
        f.nir.push_node(st)?;
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
    let inots = f.nir.inverted_guards();
    f.nir.peephole_select(nir::PEEPHOLE_LIMIT_GL, true, &inots);
    f.nir.invert_guards(&inots)?;
    f.nir.peephole_select(nir::PEEPHOLE_LIMIT, false, &[false; nir::MAX_IFS]);
    /* a guarded pass too large for the peephole stays a branch (femtovg's
     * filter_image) */
    f.nir.lay_out_inverted_guards();
    f.nir.fold_const_selects();
    f.nir.cse_compares();
    nir::dce(&mut f.nir, None);
    nir::fuse(&mut f.nir);
    nir::late_rules(&mut f.nir);
    nir::bcsel_fadd(&mut f.nir)?;
    nir::dce(&mut f.nir, None);
    clamp_forms(&f.nir)?;
    shrink_uniforms(f, m);
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
    let key_of = |id: usize| -> u32 {
        let own = f.keys[f.origin[id] as usize];
        match f.nir.nodes[id].kind {
            Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_)) => {
                f.nir.uses(id as u16).map(|(u, _)| f.keys[f.origin[u as usize] as usize]).min().unwrap_or(own)
            }
            _ => own,
        }
    };
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
    let inline = cube || f.live_look_ups().any(|(_, t)| t.array);
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
            Kind::Tex | Kind::TexDesc { .. } if inline || unrolled(id as u16) => {
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
        raw: [[None; 4]; MAX_VARYINGS],
        tex: [[None; 4]; MAX_LOOK_UPS],
        ubo: *ubo,
        ubo_words: [None; 2],
        ubo_off: [None; MAX_UBO_READS],
        ubo_half: [None; MAX_UBO_READS],
        ubo_lanes: [[None; 4]; MAX_UBO_READS],
        desc: None,
        frag: None,
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
            LOAD | TAIL_LOAD => convert(p, f, x as usize, &none, &mut lv, &mut rows)?,
            TEX => tex_highp(p, f, x as usize, &mut lv, val)?,
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
                let s0 = src32(f, &lv, val, nd.src[0])?;
                let s1 = if header { Src::NULL } else { src32(f, &lv, val, nd.src[1])? };
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
                    tex_highp(p, f, j, &mut lv, val)?;
                    continue;
                }
                /* `pan_nir_load_va_desc`'s `load_ssbo` (`LD_PKA.i64` of the
                 * table handle at the descriptor's offset), split into its
                 * two words */
                if let Kind::TexDesc { sampler } = nd.kind {
                    lv.desc = Some(tex_desc(p, f, sampler)?);
                    continue;
                }
                if at_store(id) {
                    continue;
                }
                let Kind::Alu(op) = nd.kind else { return Err(defect) };
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
                            *y = y.swz(Swz::H00);
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
                    convert(p, f, k as usize, &none, lv, rows)?;
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
        alu(
            p,
            Op::Blend { sr_count: 4, f32: !u32, u32 },
            &[
                Src::ssa(col),
                Src::ssa(t),
                Src::of(Idx::Special(Special::BlendDescriptor0, false)),
                Src::of(Idx::Special(Special::BlendDescriptor0, true)),
                Src::NULL,
            ],
        )?;
        f.rows = rows;
        return preloads(p);
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
                convert(p, f, k as usize, &none, lv, rows)?;
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
            Word::Const(_) | Word::Tex(..) => {}
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
        Word::Const(_) => return Err(defect),
    };
    let t = alu(p, Op::Atest, &[Src::of(Idx::Reg(60)), at, Src::of(Idx::Special(Special::AtestDatum, false))])?;
    let mut out = [Src::NULL; 2];
    for (k, o) in out.iter_mut().enumerate() {
        *o = match words[k] {
            Word::Const(w) => Src::of(Idx::Const(w)),
            Word::Pair(..) => Src::ssa(alu(p, Op::MovI32, &[Src::ssa(pair[k])])?),
            Word::Tex(j, h) => Src::ssa(alu(p, Op::MovI32, &[Src::ssa(tex_word(j, h)?)])?),
            Word::Lanes(a, b) => {
                let s = [half(a)?, half(b)?];
                Src::ssa(row(p, &mut rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)?)
            }
        };
    }
    let col = alu(p, Op::Collect, &out)?;
    alu(
        p,
        Op::Blend { sr_count: 2, f32: false, u32: false },
        &[
            Src::ssa(col),
            Src::ssa(t),
            Src::of(Idx::Special(Special::BlendDescriptor0, false)),
            Src::of(Idx::Special(Special::BlendDescriptor0, true)),
            Src::NULL,
        ],
    )?;
    f.rows = rows;
    preloads(p)
}

/// Look-up `j` of a highp shader ([`tex`]): a computed coordinate's two
/// values, or a cube's S, T and explicit level's word, selected before it.
fn tex_highp(p: &mut Prog<'_>, f: &Fs<'_>, j: usize, lv: &mut Leaves, val: &[Option<Src>]) -> Result<(), MaliError> {
    let t = f.look_ups().get(j).copied().ok_or(MaliError::Defect(Defect::IrIndex))?;
    let nd = f.nir.nodes[t.node as usize];
    let mut staging = [Src::NULL; 3];
    let n = match t {
        TexLookup { cube: Some(Cube { explicit: true }), .. } | TexLookup { array: true, .. } => 3,
        TexLookup { coord: TexCoord::Computed, .. } => 2,
        _ => 0,
    };
    for (k, x) in staging.iter_mut().enumerate().take(n) {
        *x = src32(f, lv, val, nd.src[k])?;
    }
    tex(p, f, j, lv, (n > 0).then_some(&staging[..n]))
}

/// The table handle `pan_nir_load_va_desc` loads a texture descriptor
/// through: the texture table (`PAN_TABLE_TEXTURE`, 4) in the low byte of
/// `pan_res_handle(62, 0)` (pan_nir.h:67-71; intbig_case42.expect.json's
/// FAU word `3e000004`).
const TEX_TABLE_HANDLE: u32 = (62 << 24) | 4;

/// The first two words of `sampler`'s texture descriptor ([`Kind::TexDesc`]):
/// `LD_PKA.i64` at the descriptor's offset in the table (32 bytes each),
/// split (intbig_case42.expect.json: `LD_PKA.i64.unsigned.slot1 @r2:r3,
/// 0x0, u2.w1`). The sampler's look-up names the index ([`layout`]); a
/// size query of a sampler no look-up reads has no reference.
fn tex_desc(p: &mut Prog<'_>, f: &Fs<'_>, sampler: u8) -> Result<[u16; 2], MaliError> {
    let (_, t) = f.live_look_ups().find(|(_, t)| t.sampler == sampler).ok_or(MaliError::TexKind)?;
    let off = Src::of(Idx::Const(32 * u32::from(t.index)));
    let r = alu(p, Op::LdPka { words: 2, half: false }, &[off, Src::of(Idx::Const(TEX_TABLE_HANDLE))])?;
    let w = split(p, r, 2)?;
    Ok([w[0], w[1]])
}

/// The texels look-up `t` writes: a 2D array's those the colour reads
/// (`nir_def_components_read`, bifrost_compile.c:3522; case 42's `.a` of
/// the fetch), another's up to the last one read (`nir_opt_shrink_vectors`
/// trims the unread tail: texmore_2's `.rgb`), an fp32 look-up's from the
/// lowest one read (unitype_100.expect.json's `.ba`).
fn tex_mask(t: &TexLookup, f16: bool) -> u8 {
    let tail = (1u8 << (8 - t.lanes.leading_zeros())) - 1;
    if t.array {
        t.lanes
    } else if f16 || t.lanes == 0 {
        tail
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
            let cond = src32(f, lv, val, r.cond)?.swz(Swz::H00);
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
    )
}

/// The sources a front-end operation reads (`AluOp::arity` is private to
/// the front end's own walk).
fn ir_arity(a: AluOp) -> usize {
    match a {
        AluOp::Fadd
        | AluOp::Fmul
        | AluOp::Fmin
        | AluOp::Fmax
        | AluOp::Flt
        | AluOp::Fle
        | AluOp::Feq
        | AluOp::Fne
        | AluOp::Iadd
        | AluOp::Idiv
        | AluOp::Ilt
        | AluOp::Ile
        | AluOp::Ieq
        | AluOp::Ine
        | AluOp::Fdot2
        | AluOp::Fdot3
        | AluOp::Fdot4 => 2,
        AluOp::Ffma | AluOp::Bcsel => 3,
        _ => 1,
    }
}

/// [`emit::emit`], noting the row in `rows`.
fn row(p: &mut Prog<'_>, rows: &mut u128, r: Row, s: &[Src]) -> Result<u16, MaliError> {
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
    conv: [[Option<u16>; 4]; MAX_VARYINGS],
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
    /// The texture descriptor's two words `textureSize` reads
    /// ([`Kind::TexDesc`], [`tex_desc`]).
    desc: Option<[u16; 2]>,
    /// The narrowed position's word ([`Leaf::Frag`], [`frag16`]).
    frag: Option<u16>,
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
    let Kind::UniformIdx { dword, comps: 1 } = nd.kind else { return Err(defect) };
    let base = lv.ubo.word(dword).ok_or(defect)?;
    let idx = match nd.src[0] {
        Val::Lane(i, 0) => match f.nir.nodes[i as usize].kind {
            Kind::Uniform { dword: d, .. } => Src::of(Idx::Ubo { base: lv.ubo.word(d).ok_or(defect)?, comp: 0 }),
            _ => return Err(MaliError::UniformIndexed),
        },
        _ => return Err(MaliError::UniformIndexed),
    };
    let mut off = row(p, rows, Row { op: Alu::Ishl, bits: 32, width: 1 }, &[idx, Src::of(Idx::Const(2))])?;
    if base > 0 {
        let s = [Src::of(Idx::Const(4 * u32::from(base))), Src::ssa(off)];
        off = row(p, rows, Row { op: Alu::Iadd, bits: 32, width: 1 }, &s)?;
    }
    lv.ubo_off[k] = Some(off);
    Ok(())
}

/// Float array read `k`'s load at its offset ([`ubo_offset`]): `LD_PKA.i16`
/// of UBO 0, the element's fp16 half into the low half of a register.
fn ubo_fetch(p: &mut Prog<'_>, k: usize, lv: &mut Leaves) -> Result<(), MaliError> {
    let off = lv.ubo_off.get(k).copied().flatten().ok_or(MaliError::Defect(Defect::IrIndex))?;
    lv.ubo_half[k] = Some(alu(p, Op::LdPka { words: 1, half: true }, &[Src::ssa(off), Src::of(Idx::Const(0))])?);
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
    lv: &mut Leaves,
    rows: &mut u128,
) -> Result<(), MaliError> {
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
    let mut c = 0;
    while c < v.comps as usize {
        match form[k][c] {
            Some(true) => {
                let s = [Src::ssa(lanes[c]), Src::ssa(lanes[c + 1])];
                lv.conv[k][c] = Some(row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 2 }, &s)?);
                c += 2;
            }
            Some(false) => {
                lv.conv[k][c] = Some(row(p, rows, Row { op: Alu::F2f16, bits: 16, width: 1 }, &[Src::ssa(lanes[c])])?);
                c += 1;
            }
            None => c += 1,
        }
    }
    Ok(())
}

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
            let lane = |c: usize| lv.raw[k as usize][c].ok_or(MaliError::Defect(Defect::IrIndex));
            staging[..2].copy_from_slice(&[Src::ssa(lane(0)?), Src::ssa(lane(1)?)]);
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
     * pan_nir_lower_tex.c:927-937), its dimension `cube`; a 2D look-up
     * here is `texture2D`'s computed level, or `texelFetch`'s */
    let plain = if t.array { super::bir::TexDim::D2Array } else { super::bir::TexDim::D2 };
    let (lod, dim) = match t.cube {
        None if t.fetch => (super::bir::LodMode::Fetch, plain),
        Some(Cube { explicit: true }) => (super::bir::LodMode::Explicit, super::bir::TexDim::Cube),
        Some(Cube { explicit: false }) => (super::bir::LodMode::Zero, super::bir::TexDim::Cube),
        None => (super::bir::LodMode::Computed, plain),
    };
    let tail = (1u8 << (8 - t.lanes.leading_zeros())) - 1;
    let mask = tex_mask(&t, f16);
    let op = Op::TexSingle { sr_count: n as u8, mask, f16, skip: false, lod, dim };
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
    if mask == tail {
        let n = if f16 { 2 } else { 4 };
        let w = split(p, r, n)?;
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

/// The narrowed position's word in a mediump shader ([`Fs::frag_coord16`]):
/// x and y at fp32 in NIR's order (`u2f32` of both halves of the preloaded
/// pixel, their `+ 0.5`, y's `ffma` with `gl_FbWposYTransform`), each
/// narrowed (`f2f16`), then `vec2` of the two halves (prec_84.expect.json:
/// `U16_TO_F32` x2, `FADD_IMM` x2, `FMA.f32`, `FADD.f32 .h0` x2,
/// `MKVEC.v2i16 r0, r0^.h0, r1^.h0`).
fn frag16(p: &mut Prog<'_>, lv: &Leaves, rows: &mut u128) -> Result<u16, MaliError> {
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
    let s = [Src::ssa(x).swz(Swz::H00), Src::ssa(y).swz(Swz::H00)];
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)
}

fn conv_of(lv: &Leaves, k: u8, c: u8) -> Result<u16, MaliError> {
    lv.conv.get(k as usize).and_then(|x| x[c as usize & 3]).ok_or(MaliError::Defect(Defect::IrIndex))
}

/// `vec2` of a converted lane and a constant half (`MKVEC.v2i16`).
fn mkvec(p: &mut Prog<'_>, lv: &Leaves, k: u8, c: u8, half: u16, rows: &mut u128) -> Result<u16, MaliError> {
    let x = Src::ssa(conv_of(lv, k, c)?).swz(Swz::H00);
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &[x, Src::of(Idx::Const(half as u32)).swz(Swz::H00)])
}

/// `vec2` of two lanes (load, lane), each converted alone (`MKVEC.v2i16`).
fn mkvec_lanes(p: &mut Prog<'_>, lv: &Leaves, x: [(u8, u8); 2], rows: &mut u128) -> Result<u16, MaliError> {
    let s =
        [Src::ssa(conv_of(lv, x[0].0, x[0].1)?).swz(Swz::H00), Src::ssa(conv_of(lv, x[1].0, x[1].1)?).swz(Swz::H00)];
    row(p, rows, Row { op: Alu::Vec2, bits: 16, width: 2 }, &s)
}

/// Pair node `x` as a source: a conversion, a constant, a uniform word, a
/// word of the look-up, or an operation already selected.
fn operand(pr: &Pairs, x: u8, lv: &Leaves, val: &[Option<Src>; MAX_PAIRS]) -> Result<Src, MaliError> {
    Ok(match pr.nodes[x as usize] {
        P::Leaf(Leaf::Half { k, h, swz }) => Src::ssa(conv_of(lv, k, 2 * h)?).swz(swz),
        P::Leaf(Leaf::Splat { k, c }) => Src::ssa(conv_of(lv, k, c)?).swz(Swz::H00),
        P::Leaf(Leaf::Const(w)) => Src::of(Idx::Const(w)),
        P::Leaf(Leaf::Uniform { base, comp, swz }) => Src::of(Idx::Ubo { base, comp }).swz(swz),
        P::Leaf(Leaf::Tex(j, h)) => {
            Src::ssa(lv.tex.get(j as usize).and_then(|w| w[h as usize & 3]).ok_or(MaliError::Defect(Defect::IrIndex))?)
        }
        P::Leaf(Leaf::Frag) => Src::ssa(lv.frag.ok_or(MaliError::Defect(Defect::IrIndex))?),
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
        P::Leaf(Leaf::TexLane(j, c)) => {
            let w = lv.tex.get(j as usize).and_then(|w| w[(c as usize / 2) & 3]);
            Src::ssa(w.ok_or(MaliError::Defect(Defect::IrIndex))?).swz(if c % 2 == 0 { Swz::H00 } else { Swz::H11 })
        }
        P::Leaf(Leaf::Mk { .. } | Leaf::Lanes { .. } | Leaf::Zext(_) | Leaf::Lane { .. }) | P::Op(..) => {
            val[x as usize].ok_or(MaliError::Defect(Defect::IrIndex))?
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
    let mut s = [Src::NULL; 3];
    for (k, y) in srcs.iter().enumerate().take(a.arity()) {
        match pr.nodes[*y as usize] {
            P::Leaf(Leaf::Mk { k: v, c, half }) => val[*y as usize] = Some(Src::ssa(mkvec(p, lv, v, c, half, rows)?)),
            P::Leaf(Leaf::Lanes { k: v, c, e, d }) => {
                val[*y as usize] = Some(Src::ssa(mkvec_lanes(p, lv, [(v, c), (e, d)], rows)?))
            }
            _ => {}
        }
        s[k] = operand(pr, *y, lv, val)?;
        /* a one-lane operation reads a constant's low half ([`coord_lane`])
         * and a one-lane operation's (texmore_2's `%9^.h00`) */
        let lone = match pr.nodes[*y as usize] {
            P::Leaf(Leaf::Const(_)) => true,
            P::Op(..) => pr.scalar[*y as usize],
            _ => false,
        };
        if pr.scalar[x as usize] && lone {
            s[k] = s[k].swz(Swz::H00);
        }
    }
    let width = if pr.scalar[x as usize] { 1 } else { 2 };
    val[x as usize] = Some(Src::ssa(row(p, rows, Row { op: a, bits: 16, width }, &s)?));
    Ok(())
}
