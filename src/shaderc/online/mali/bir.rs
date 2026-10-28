// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2020-2025 Collabora Ltd.
// SPDX-FileCopyrightText: 2025-2026 Arm Ltd.
// Derived from Mesa 26.2.1 src/panfrost/compiler/bifrost/ (compiler.h,
// bir.c, the generated bi_opcodes.c and valhall/valhall.c) (MIT). The
// upstream part stays under its licence (LICENSES/MIT.txt); our changes are
// Apache-2.0. See NOTICE.
//! The Valhall back end's program between selection and packing: Mesa's
//! BIR (`bi_instr`, bifrost/compiler.h) for the ops this back end selects,
//! in basic blocks (`bi_block`: successors and predecessors, the blocks in
//! their emission order, which is their layout order).
//!
//! The instructions are one list in program order, each stamped with its
//! block ([`Ins::block`]); a block's instructions are contiguous, in block
//! order, so a block is the run of instructions that carry its index
//! ([`Prog::block_start`], [`Prog::block_end`]), possibly empty (Mesa makes
//! an empty `bi_block` for an empty NIR block, `emit_block`). A pass that
//! inserts an instruction says which block it joins ([`Prog::insert`]
//! takes the block of the instruction it stands before, [`Prog::insert_in`]
//! names one, [`Prog::push`] the block being emitted).
//!
//! Only what the passes after selection read is modelled. The per-op facts
//! are transcribed from Mesa 26.2.1's generated tables (the values are
//! printed next to each entry in [`props`] and [`src_info`]):
//! `bi_opcode_props[]` (build/src/panfrost/compiler/bifrost/bi_opcodes.c,
//! from bifrost/ISA.xml and IR_pseudo.xml) and `valhall_opcodes[]`
//! (build/.../valhall/valhall.c, from valhall/ISA.xml by valhall.c.py).
//! The counts are bir.c's: [`count_read`] is `bi_count_read_registers`
//! (bir.c:67-84), [`count_write`] `bi_count_write_registers` (bir.c:87-140)
//! and the staging count `bi_count_staging_registers` (bir.c:46-65).

/// The instructions one program may hold: selection, the constant moves and
/// the flow NOPs all count. A program that needs more is refused
/// ([`super::MaliError::InstrTableFull`]), never truncated. The largest
/// measured, case 194's offarm256, holds 329 (Mesa's 312 and the moves).
pub const MAX_INSTRS: usize = 384;
/// The most SSA values a program may define (the RA's node count);
/// offarm256 defines 330.
pub const MAX_NODES: usize = 384;
/// Sources of one instruction (BLEND has five: colour, coverage, the blend
/// descriptor's two words, the dual-source colour).
pub const MAX_SRCS: usize = 5;
/// Destinations of one instruction (a SPLIT of a vec4).
pub const MAX_DESTS: usize = 4;
/// The blocks one program may hold: the entry block and three per `if`
/// (then, else, after; `emit_if`), nested or not.
pub const MAX_BLOCKS: usize = 32;
/// The predecessors one block may have (an `if`'s join has two; no loop).
pub const MAX_PREDS: usize = 4;

/// A special FAU value (valhall/ISA.xml "FAU special page 0"), each 64 bits:
/// the source names one 32-bit half.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Special {
    /// `atest_datum` (page 0, entry 5).
    AtestDatum,
    /// `blend_descriptor_0` (page 0, entry 8).
    BlendDescriptor0,
}

/// A word of the sysval UBO a vertex shader reads (Mesa's
/// `PAN_SYSVAL_VIEWPORT_SCALE` / `_OFFSET`, one vec4 slot each,
/// gallium/drivers/panfrost/pan_cmdstream.c:989-1010).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sysval {
    ViewportScale,
    ViewportOffset,
}

impl Sysval {
    /// The sysval's first word in the sysval UBO: one vec4 slot each, in
    /// the order the vertex shader's `nir_lower_viewport_transform` asks
    /// for them (scale, then offset; pan_shc measured the layout: scale at
    /// byte 0, offset at byte 16, tools/mali/testdata/pan_shc-out/*.json on
    /// PR #1072's branch).
    pub const fn base_word(self) -> u16 {
        match self {
            Sysval::ViewportScale => 0,
            Sysval::ViewportOffset => 4,
        }
    }
}

/// What a source or a destination names (`bi_index`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Idx {
    Null,
    /// An SSA value and a 32-bit word offset into it (`BI_INDEX_NORMAL`).
    Ssa(u16, u8),
    /// A hardware register (`BI_INDEX_REGISTER`): before RA only the
    /// preloaded registers read by their moves, after RA every operand.
    Reg(u8),
    /// A 32-bit constant before `va_lower_constants` (`BI_INDEX_CONSTANT`).
    Const(u32),
    /// A component of a sysval before `bi_opt_push_ubo` (a `load_ubo` of
    /// the sysval UBO, word `base_word + component`).
    Sysval(Sysval, u8),
    /// Word `base + comp` of the uniform block (Mesa's UBO 0, in its packed
    /// layout) before `bi_opt_push_ubo`: a lane of the `load_ubo` whose
    /// first word is `base` (the push takes, per load, the words from its
    /// base up to its highest lane read).
    Ubo {
        base: u16,
        comp: u8,
    },
    /// A pushed 32-bit FAU word: slot `w / 2`, half `w % 2`
    /// (`BIR_FAU_UNIFORM | slot`, offset = half).
    Uniform(u16),
    /// An entry of the table of immediates (`va_lut(i)`, valhall/ISA.xml
    /// `<lut name="Immediates">`).
    Lut(u8),
    /// A special FAU value, the high half when the flag is set.
    Special(Special, bool),
}

/// A 32-bit operand's half selection (`enum bi_swizzle`: `H01` is the
/// identity, `H00` / `H11` replicate one half, `H10` swaps the halves of a
/// 16-bit pair; as a widen of a 32-bit source `H00` reads half 0 and `H11`
/// half 1), or one of its bytes (`B0`..`B3`, `bi_byte`: as a widen of a
/// 32-bit integer source the byte, zero- or sign-extended; the form
/// `va_lower_constants` gives a small integer constant the table holds a
/// byte of, `0x1000000.b3` for 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Swz {
    H01,
    H00,
    H11,
    H10,
    B0,
    B1,
    B2,
    B3,
}

/// A source operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Src {
    pub idx: Idx,
    pub swz: Swz,
    pub neg: bool,
    pub abs: bool,
    /// `va_mark_last`'s last-use flag (a register source only).
    pub discard: bool,
}

impl Src {
    pub const NULL: Src = Src { idx: Idx::Null, swz: Swz::H01, neg: false, abs: false, discard: false };
    pub const fn of(idx: Idx) -> Src {
        Src { idx, swz: Swz::H01, neg: false, abs: false, discard: false }
    }
    pub const fn ssa(v: u16) -> Src {
        Src::of(Idx::Ssa(v, 0))
    }
    pub const fn swz(self, swz: Swz) -> Src {
        Src { swz, ..self }
    }
    /// An SSA value (`bi_is_ssa`).
    pub const fn ssa_value(&self) -> Option<u16> {
        match self.idx {
            Idx::Ssa(v, _) => Some(v),
            _ => None,
        }
    }
}

/// A destination: an SSA value (with a word offset after `bi_lower_vector`)
/// or, after RA, a register; `H00` writes only the low half (`va_pack_wrmask`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dest {
    pub idx: Idx,
    pub swz: Swz,
}

impl Dest {
    pub const NULL: Dest = Dest { idx: Idx::Null, swz: Swz::H01 };
    pub const fn ssa(v: u16) -> Dest {
        Dest { idx: Idx::Ssa(v, 0), swz: Swz::H01 }
    }
}

/// `STORE`'s cache hint (`va_pack_store`: `.istream` for the position and
/// attribute sections, `.estream` for generic varyings,
/// pan_nir_lower_vs_outputs.c:76-82).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Istream,
    Estream,
}

/// A compare's condition (`enum bi_cmpf`, valhall/ISA.xml "Condition": the
/// encoding is the order here).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cmpf {
    Eq = 0,
    Gt = 1,
    Ge = 2,
    Ne = 3,
    Lt = 4,
    Le = 5,
}

/// How a compare encodes its boolean (`enum bi_result_type`, ISA.xml
/// "Result type"): `M1` is 0 / ~0 (the mask a `bcsel` tests), `F1` is 0.0
/// / 1.0 (a `b2f` folded into the compare, `bi_optimizer_result_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ResultType {
    I1 = 0,
    F1 = 1,
    M1 = 2,
}

/// A rounding mode (`enum bi_round`, ISA.xml "Round mode").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Round {
    Rte = 0,
    Rtp = 1,
    Rtn = 2,
    Rtz = 3,
}

/// `enum bi_clamp` (ISA.xml "Clamp"): the clamp a float instruction
/// applies to its result, [`Ins::clamp`]; `FCLAMP`'s own, before it is
/// folded or lowered (opt.rs).
pub mod clamp {
    pub const NONE: u8 = 0;
    pub const ZERO_INF: u8 = 1;
    pub const M1_1: u8 = 2;
    pub const ZERO_ONE: u8 = 3;
}

/// A look-up's level-of-detail mode (`enum va_lod_mode`, the flags
/// pan_nir_lower_tex.c:927-947 sets).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LodMode {
    /// From the derivatives (`BI_VA_LOD_MODE_COMPUTED_LOD`): a fragment
    /// look-up without a level.
    Computed,
    /// Level 0 (`BI_VA_LOD_MODE_ZERO_LOD`): a `txl` at a constant 0.
    Zero,
    /// The level in the staging word after the coordinate, signed 8.8
    /// fixed point (`BI_VA_LOD_MODE_EXPLICIT`): any other `txl`.
    Explicit,
    /// `texelFetch` at level 0: a `txf`, which `bi_emit_tex_valhall`
    /// selects as `TEX_FETCH` (bifrost_compile.c:3535-3540) on the integer
    /// coordinate's words, no level word, and which has no `skip` bit
    /// (`bi_has_skip_bit`, bi_helper_invocations.c:43-60). Measured:
    /// fbload_fbload.expect.json's `TEX_FETCH.slot0.reserved.32.2d`.
    Fetch,
}

/// A look-up's texture dimension (`enum bi_dimension` as
/// `valhall_tex_dimension` maps a `glsl_sampler_dim`, bifrost_compile.c:
/// 3461-3485; the ISA's `Dimension` enum is 1d, 2d, 3d, cube in order,
/// valhall/ISA.xml:477-482).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TexDim {
    /// `GLSL_SAMPLER_DIM_2D`: `tex2d` (1).
    D2,
    /// `GLSL_SAMPLER_DIM_CUBE`: `cube` (3); the coordinate's two staging
    /// words are the cube descriptor `build_cube_desc` packs (the face
    /// index in the top three bits of S).
    Cube,
}

/// A special varying's name (`enum bi_varying_name`, the ISA's
/// `varying_name`: point 0, frag_w 2, frag_z 3, valhall/ISA.xml).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum VaryingName {
    /// gl_PointCoord.
    Point = 0,
    /// gl_FragCoord.z.
    FragZ = 3,
}

/// The ops of this back end (`enum bi_opcode`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Nop,
    MovI32,
    /// `IADD_IMM.i32 dest, src0, #imm`.
    IaddImmI32(u32),
    /// `FADD_IMM.f32 dest, src0, #imm` (`va_fuse_add_imm`, va_optimize.c).
    FaddImmF32(u32),
    /// `FADD_IMM.v2f16 dest, src0, #imm`, the same for an fp16 pair.
    FaddImmV2f16(u32),
    FmaF32,
    /// Two fp16 lanes at once (a mediump fragment shader's arithmetic).
    FmaV2f16,
    FaddV2f16,
    FaddF32,
    FrcpF32,
    /// `.nan_suppress`, the semantics `nir_fmax` gets (bifrost_compile.c).
    FmaxF32,
    /// The fp16 `fmax` of gl_PointSize's clamp (`nir_lower_point_size`),
    /// `.nan_suppress` as `FMAX.f32`.
    FmaxV2f16,
    FminF32,
    /// `FMIN.v2f16` `.nan_suppress`: a blend shader's `fmin` of the source
    /// and the destination (GL_MIN, `nir_blend_func`).
    FminV2f16,
    /// `FABSNEG.f32` / `.v2f16`: the pseudo-op of an fneg or fabs (the
    /// modifiers on its source), folded into its users or lowered to an
    /// FADD before packing (opt.rs).
    FabsnegF32,
    FabsnegV2f16,
    /// `FCLAMP.f32`: the pseudo-op of an `fsat` (and Mesa's other clamp
    /// forms), its clamp in [`Ins::clamp`]; folded into its single-use
    /// producer or lowered to an FADD before packing (opt.rs).
    FclampF32,
    /// `FCLAMP.v2f16`, the same for an fp16 pair (a blend shader's `fsat`
    /// of the source colour and of the constants, blend.rs).
    FclampV2f16,
    /// `FCMP_OR.f32 dest, a, b, 0 .cmpf .result` (`bi_fcmp_to`, which
    /// `va_lower_isel` makes the `_OR` form with a zero third source).
    FcmpOrF32 {
        cmpf: Cmpf,
        result: ResultType,
    },
    /// `FCMP_OR.v2f16`, the same per 16-bit lane: a mediump compare
    /// (`bi_fcmp_to` of a 16-bit `flt_pan`), each lane's mask 0 / ~0 in
    /// its half.
    FcmpOrV2f16 {
        cmpf: Cmpf,
        result: ResultType,
    },
    /// `ICMP_OR.s32`, the same for signed integers (`bi_icmp_to` with
    /// `nir_type_int` and an ordering: `ICMP.s32`, then `va_lower_isel`).
    IcmpOrS32 {
        cmpf: Cmpf,
        result: ResultType,
    },
    /// `ICMP_OR.u32`: an equality (`ieq_pan`, `ine_pan`) is sign-blind, so
    /// `bi_icmp_to` picks `ICMP.i32` (bi_builder.h.py: a type `i` takes
    /// `nir_type_int` too, and only the `i32` form lists `eq`/`ne`), which
    /// `va_lower_isel` makes the `_OR.u32` form (va_lower_isel.c:16-18).
    IcmpOrU32 {
        cmpf: Cmpf,
        result: ResultType,
    },
    /// `MUX.i32.int_zero dest, a, b, mask`: `a` where the mask is zero,
    /// else `b` (`bi_mux_to` with `BI_MUX_INT_ZERO`: a `bcsel`'s else, then,
    /// condition). `va_lower_isel` makes it a CSEL when no source is
    /// swizzled.
    MuxI32,
    /// `MUX.i32.bit dest, a, b, mask`: bit by bit, `a`'s where the mask is
    /// set and `b`'s where it is clear (ISA.xml MUX.i32 `(A & mask) | (B &
    /// ~mask)`; `bi_mux_to` with `BI_MUX_BIT`, a NIR `bitfield_select(mask,
    /// a, b)`, bifrost_compile.c:3419-3426: the cube descriptor's S word,
    /// `s` below bit 29 and the face above). Never a CSEL
    /// (`bi_can_replace_with_csel` excludes `BI_MUX_BIT`, bir.c:252-258).
    MuxBitI32,
    /// `CUBEFACE1 dest, x, y, z`: the largest absolute coordinate
    /// (`nir_cubeface_pan`'s first lane, `bi_cubeface1_to`,
    /// bifrost_compile.c:2118-2124).
    Cubeface1,
    /// `CUBEFACE2_V9 dest, x, y, z`: the face index, preshifted into the
    /// top three bits (`nir_cubeface_pan`'s second lane,
    /// `bi_cubeface2_v9_to`).
    Cubeface2,
    /// `CUBE_SSEL dest, z, x, face`: the S coordinate of the face
    /// (`nir_cube_ssel_pan`, bifrost_compile.c:2128-2131).
    CubeSsel,
    /// `CUBE_TSEL dest, y, z, face`: the T coordinate of the face
    /// (`nir_cube_tsel_pan`, :2134-2137).
    CubeTsel,
    /// `MUX.v2i16.int_zero`, the same per 16-bit lane (a `b2f16`).
    MuxV2i16,
    /// `CSEL.u32.eq dest, a, b, t, f`: `t` where `a == b`, else `f`
    /// (`bi_csel_from_mux`).
    CselU32,
    /// `CSEL.s32.cmpf dest, a, b, t, f`: `t` where `a cmpf b` (signed),
    /// else `f`; `imax` is `.gt a, b, a, b` and `imin` `.lt`
    /// (`bi_csel_to(.., nir_type_int, ..)`, bifrost_compile.c:3061-3071).
    CselS32 {
        cmpf: Cmpf,
    },
    /// `IABS.s32` (`nir_op_iabs`, bifrost_compile.c:3371-3373).
    IabsS32,
    /// `IMUL.i32` (`nir_op_imul`, bifrost_compile.c:3367-3369: the low word
    /// of the product).
    ImulI32,
    /// `ISUB.s32 dest, a, b` (`nir_op_isub`; `nir_op_ineg` as `0 - a`,
    /// bifrost_compile.c:3351-3357).
    IsubS32,
    /// `U32_TO_F32` (`nir_op_u2f32` of a 32-bit source,
    /// bifrost_compile.c:3171-3172).
    U32ToF32,
    /// `F32_TO_U32.rtz` (`nir_op_f2u32`, bifrost_compile.c:3137-3150).
    F32ToU32,
    /// `LSHIFT_XOR.i32 dest, a, b, shift` (`nir_op_ixor` with the shift the
    /// byte 0, bifrost_compile.c:3383-3385).
    LshiftXorI32,
    /// `FROUND.f32` with the rounding mode (`ffloor`: `.rtn`, `fceil`:
    /// `.rtp`).
    FroundF32 {
        round: Round,
    },
    /// `FROUND.v2f16`, the same per 16-bit lane (an fp16 `ffloor` of a
    /// vector of two halves).
    FroundV2f16 {
        round: Round,
    },
    FrsqF32,
    /// `F32_TO_S32.rtz` (`nir_op_f2i32`).
    F32ToS32,
    /// `S32_TO_F32` (`nir_op_i2f32`).
    S32ToF32,
    IaddS32,
    /// `S16_TO_S32 dest, x.hN`: a 16-bit half sign-extended into a word
    /// (`nir_op_i2i32` of a 16-bit source, bifrost_compile.c:3205-3211: a
    /// 16-bit compare's mask widened for a 32-bit select).
    S16ToS32,
    /// `U16_TO_U32 dest, x.hN`: a 16-bit half into a word (what
    /// `MKVEC.v2i16 x, #0` lowers to, bi_lower_mkvec_swz.c:256-257).
    U16ToU32,
    /// `U16_TO_F32 dest, x.hN`: a 16-bit half converted to fp32 (`nir_op_u2f32`
    /// of a 16-bit source, bifrost_compile.c:3167-3176; the pixel position's
    /// lanes, gl_FragCoord).
    U16ToF32,
    /// `F16_TO_F32 dest, x.hN`: an fp16 half converted to fp32
    /// (`nir_op_f2f32` of a 16-bit source; case121's coordinate).
    F16ToF32,
    /// `F16_TO_S32.rtz dest, x.hN`: an fp16 half converted to a 32-bit
    /// integer (`nir_op_f2i32` of a 16-bit source, bifrost_compile.c:3100;
    /// case 72's `int(u_in.x)`).
    F16ToS32,
    /// `U8_TO_F32 dest, x.bN`: a byte of a word converted to fp32
    /// (`nir_op_u2f32` of `extract_u8(x, N)`, bifrost_compile.c:3167-3176;
    /// intuni_ishr's `float(s & 255)`, `U8_TO_F32 r1, r0.b0`).
    U8ToF32,
    V2f32ToV2f16,
    MkvecV2i16,
    /// `IADD.v2u16 dest, x.hNN, 0`: a word of the halves of one 16-bit
    /// pair, swizzled (`MKVEC.v2i16 x.h0, x.h0` on Valhall, which has no
    /// `SWZ.v2i16`: `build_swz_v2i16_to`, bi_lower_mkvec_swz.c:92-102).
    IaddV2u16,
    /// `MKVEC.v2i8 dest, a.bN, b.bN, cd`: `a | b << 8 | cd << 16`, two
    /// byte lanes and a 16-bit half (`bi_lower_mkvec_swz`'s v4i8 on
    /// Valhall, bi_lower_mkvec_swz.c:189-203: a blend shader's tile word).
    MkvecV2i8,
    ShaddxS64,
    /// Attribute record `index` of the attribute table (table 1), `vecsize`
    /// 32-bit components.
    LdAttrImm {
        index: u8,
        vecsize: u8,
    },
    /// The varying buffer address (`lea_buf_pan`): resource table `table`,
    /// buffer `index`.
    LeaBufImm {
        table: u8,
        index: u8,
    },
    StoreI32 {
        access: Access,
    },
    /// gl_PointSize's fp16 store (`store_global_psiz_pan`, `is_psiz_write`:
    /// the store the no_psiz copy leaves out, bifrost_compile.c:4724-4750).
    StoreI16 {
        access: Access,
    },
    /// Varying `index` of the varying buffer, `vecsize` f32 components,
    /// sampled at the centre, `.store` update; `flat` read as stored
    /// (`.src_flat32`), else interpolated (`.src_f32`).
    LdVarBufImmF32 {
        index: u8,
        vecsize: u8,
        flat: bool,
    },
    /// A special varying (`load_var_special_pan`, bifrost_compile.c:
    /// 1419-1440): `name` the ISA's `varying_name` (gl_PointCoord's
    /// `point`, gl_FragCoord.z's `frag_z`, pan_nir_lower_var_special.c:
    /// 17-25), `vecsize` f32 components at the centre, `.clobber` update.
    LdVarSpecialF32 {
        name: VaryingName,
        vecsize: u8,
    },
    /// Input `index` of attribute table `table` interpolated at the
    /// centre, `.store` update, `vecsize` f32 components (`LD_VAR_IMM`: a
    /// frame shader's coordinate, which no vertex shader writes).
    LdVarImmF32 {
        table: u8,
        index: u8,
        vecsize: u8,
    },
    Atest,
    /// `ZS_EMIT`: write the depth (`z`, source 0) and the stencil (`s`,
    /// source 1) of the fragment, source 2 the coverage, the updated
    /// coverage the staging destination (`bi_zs_emit_to`,
    /// bifrost_compile.c:1647-1655).
    ZsEmit {
        z: bool,
        s: bool,
    },
    /// `f32`: the colour in fp32 registers (register format F32), else fp16;
    /// `u32`: in unsigned integer registers (U32, a `uvec4` output's
    /// `nir_type_uint32`, `bi_reg_fmt_for_nir`), `f32` then false.
    Blend {
        sr_count: u8,
        f32: bool,
        u32: bool,
    },
    /// `BRANCHZI.absolute`: `.eq` the fragment shader's blend-shader call
    /// (va_lower_blend), `.ne` a blend shader's return through the link
    /// register (`blend_return_pan`, bifrost_compile.c:1711-1721).
    BranchzI {
        eq: bool,
    },
    /// `LSHIFT_AND.i32 dest, a, mask, shift`: `(a << shift) & mask`
    /// (`bi_lshift_and_i32`, a NIR `iand`; the shift a byte lane, the mask
    /// Valhall's third source, `va_swap_12`).
    LshiftAndI32,
    /// `LSHIFT_OR.i32 dest, a, b, shift`: `(a << shift) | b`
    /// (`bi_lshift_or_i32`; a NIR `ishl` is `(a << shift) | 0`, the shift
    /// byte 0 of its source, bifrost_compile.c `nir_op_ishl`); the sources
    /// as [`Op::LshiftAndI32`]'s.
    LshiftOrI32,
    /// `RSHIFT_AND.i32 dest, a, mask, shift`: `(a >> shift) & mask`
    /// (`bi_rshift_and_i32`, the sample id of r61, `bi_load_sample_id_to`,
    /// bifrost_compile.c:887-896); the sources as [`Op::LshiftAndI32`]'s.
    RshiftAndI32,
    /// `RSHIFT_OR.i32 dest, a, b, shift`: `(a >> shift) | b` (a NIR `ushr`
    /// is `(a >> shift) | 0`, bifrost_compile.c:3050-3052); the sources
    /// as [`Op::LshiftAndI32`]'s.
    RshiftOrI32,
    /// `ARSHIFT_OR.i32 dest, a, b, shift`: `(a >> shift) | b`, the shift
    /// arithmetic (a NIR `ishr` is `(a >> shift) | 0`, bifrost_compile.c:
    /// 3053-3055; intuni_ishr's `ARSHIFT_OR.i32 r0, u2.w0, r0^.b00, 0x0`);
    /// the sources as [`Op::LshiftAndI32`]'s.
    ArshiftOrI32,
    /// `LD_PKA.i<32 * words> @dest, address, descriptor` (`bi_ld_pka_to`
    /// with `seg = BI_SEG_UBO`, bifrost_compile.c:870-883): `words` 32-bit
    /// words of the buffer the mode descriptor (source 1, UBO 0 is the
    /// constant 0) names, at the byte offset of source 0, into `words`
    /// staging registers. A UBO is read-only, so no memory ordering holds
    /// it (va_insert_flow.c `bi_is_memory_access`, bi_pressure_schedule.c).
    /// `half`: `LD_PKA.i16`, one 16-bit value into the low half of one
    /// register (`words` 1; a mediump float array's element,
    /// case108_uniidx).
    LdPka {
        words: u8,
        half: bool,
    },
    /// `LD_TILE`: the tile buffer's colour at the pixel and sample the first
    /// source names (`load_tile_pan`, a blend shader's destination,
    /// bifrost_compile.c:1445-1466): `vecsize` channels in fp16 (`f16`) or
    /// fp32, sources the pixel indices, the coverage and the conversion.
    LdTile {
        vecsize: u8,
        f16: bool,
    },
    /// `BRANCHZ.i16.eq src0, #0` to the start of block `target`
    /// (`bi_branchz_i16(.., BI_CMPF_EQ)` with a `branch_target`): taken when
    /// the 16-bit source reads zero. An `if`'s branch to its else block
    /// reads the condition mask's low half (`bi_half(cond, false)`,
    /// `emit_if`); the then block's exit to the join is the same op on the
    /// zero constant (`bi_jump`, which `va_lower_isel` makes
    /// `BRANCHZ.i16.eq #0`). The offset is computed when the words are
    /// packed (`va_lower_branch_target`).
    Branchz {
        target: u8,
        /// `.eq` (taken when the source is zero); false is `.ne`, which
        /// only `bi_opt_control_flow`'s inversion makes.
        eq: bool,
    },
    /// `PHI dest, src per predecessor` at the start of a join block
    /// (`bi_phi_to`): source `i` is the value from predecessor `i`
    /// ([`Block::pred`]'s order, `bi_predecessor_index`). Lowered to moves
    /// by the register allocator's out-of-SSA pass (ra.rs).
    Phi,
    /// `DISCARD.f32 a, b .cmpf`: flags the thread as discarded when the
    /// comparison holds (valhall/ISA.xml:829-840; the thread keeps running,
    /// its coverage dropped at ATEST, bifrost_nir.c:406-410). The fused form
    /// `DISCARD.b32(FCMP.f32 a, b)` makes (`bi_fuse_discard_fcmp`,
    /// bi_opt_mod_props.c:111-134), or `demote`'s `DISCARD.f32 0, 0 .eq`
    /// (bifrost_compile.c:1989-1991). No destination; a side effect
    /// (`bi_side_effects`, bir.c:181-192), never CSE'd (bi_opt_cse.c:117-124).
    DiscardF32 {
        cmpf: Cmpf,
    },
    /// `DISCARD.b32 mask`: what `demote_if` selects (bifrost_compile.c:
    /// 1979-1987), a pseudo-op here: fused with the compare that defines its
    /// mask (opt.rs `mod_prop_forward`) or lowered to `DISCARD.f32 mask, 0
    /// .ne` (`bi_lower_opt_instruction_helper`, bi_opt_mod_props.c:501-518).
    DiscardB32,
    /// `TEX_SINGLE.2d.computed_lod` (`bi_emit_tex_valhall`,
    /// bifrost_compile.c:3489-3591), the narrow handle form: the staging
    /// sources are the coordinate's `sr_count` words, source 1 and 2 the
    /// 64-bit handle (the packed texture and sampler indices, then the narrow
    /// word, pan_nir_lower_tex.c:723-745, 940-944). The texels come back in
    /// `fp16` (`register_width` 16-bit) when `f16`, the channels of `mask`
    /// written. `skip`: helper invocations need not run it
    /// (`bi_analyze_helper_requirements`, bi_helper_invocations.c).
    /// `lod`: how the level of detail is chosen ([`LodMode`]); `dim` the
    /// texture's dimension ([`TexDim`]).
    TexSingle {
        sr_count: u8,
        /// The channels written (`write_mask`, bit 0 red): one register
        /// each at 32 bits.
        mask: u8,
        f16: bool,
        skip: bool,
        lod: LodMode,
        dim: TexDim,
    },
    Collect,
    Split,
}

/// The message class of an op (`enum bifrost_message_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    None,
    Attribute,
    Varying,
    Store,
    Atest,
    Blend,
    Tex,
    /// `BIFROST_MESSAGE_Z_STENCIL`.
    ZStencil,
    /// `BIFROST_MESSAGE_TILE` (LD_TILE).
    Tile,
    /// `BIFROST_MESSAGE_LOAD` (LD_PKA).
    Load,
}

/// How many staging registers an op moves (`enum bi_sr_count`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SrCount {
    Fixed(u8),
    /// `BI_SR_COUNT_FORMAT`: the vector size (the register format is f32
    /// for every such op here).
    Format,
    /// `BI_SR_COUNT_SR_COUNT`: the instruction's own `sr_count`.
    Own,
}

/// The `bi_opcode_props[]` facts the passes read.
#[derive(Clone, Copy, Debug)]
pub struct Props {
    pub msg: Msg,
    pub sr_count: SrCount,
    pub sr_read: bool,
    pub sr_write: bool,
    /// `last`: the instruction ends the clause (side effect for DCE).
    pub last: bool,
    pub branch: bool,
}

/// `bi_opcode_props[op]` (Mesa 26.2.1 build/src/panfrost/compiler/bifrost/
/// bi_opcodes.c; the entries' `message, sr_count, sr_read, sr_write, last,
/// branch` fields, in that order).
pub const fn props(op: Op) -> Props {
    const ALU: Props = Props {
        msg: Msg::None,
        sr_count: SrCount::Fixed(0),
        sr_read: false,
        sr_write: false,
        last: false,
        branch: false,
    };
    match op {
        /* ATTRIBUTE, FORMAT, 0, 1, 0, 0 */
        Op::LdAttrImm { .. } => Props { msg: Msg::Attribute, sr_count: SrCount::Format, sr_write: true, ..ALU },
        /* ATTRIBUTE, 2, 0, 1, 0, 0 */
        Op::LeaBufImm { .. } => Props { msg: Msg::Attribute, sr_count: SrCount::Fixed(2), sr_write: true, ..ALU },
        /* STORE, 1, 1, 0, 0, 0 (STORE.i16 and STORE.i32 alike) */
        Op::StoreI32 { .. } | Op::StoreI16 { .. } => {
            Props { msg: Msg::Store, sr_count: SrCount::Fixed(1), sr_read: true, ..ALU }
        }
        /* VARYING, FORMAT, 0, 1, 0, 0 */
        Op::LdVarBufImmF32 { .. } | Op::LdVarSpecialF32 { .. } | Op::LdVarImmF32 { .. } => {
            Props { msg: Msg::Varying, sr_count: SrCount::Format, sr_write: true, ..ALU }
        }
        /* ATEST, 1, 0, 1, 0, 0 */
        Op::Atest => Props { msg: Msg::Atest, sr_count: SrCount::Fixed(1), sr_write: true, ..ALU },
        /* Z_STENCIL, 1, 0, 1, 0, 0 (ISA.xml: staging="w=1") */
        Op::ZsEmit { .. } => Props { msg: Msg::ZStencil, sr_count: SrCount::Fixed(1), sr_write: true, ..ALU },
        /* BLEND, SR_COUNT, 1, 0, 1, 0 */
        Op::Blend { .. } => Props { msg: Msg::Blend, sr_count: SrCount::Own, sr_read: true, last: true, ..ALU },
        /* NONE, 0, 0, 0, 1, 1 */
        Op::BranchzI { .. } => Props { last: true, branch: true, ..ALU },
        /* TILE, FORMAT, 0, 1, 0, 0 */
        Op::LdTile { .. } => Props { msg: Msg::Tile, sr_count: SrCount::Format, sr_write: true, ..ALU },
        /* BRANCHZ.i16: NONE, 0, 0, 0, 1, 1 (the same entry) */
        Op::Branchz { .. } => Props { last: true, branch: true, ..ALU },
        /* TEX, SR_COUNT, 1, 1, 0, 0 */
        Op::TexSingle { .. } => Props { msg: Msg::Tex, sr_count: SrCount::Own, sr_read: true, sr_write: true, ..ALU },
        /* LOAD, <words> (BI_SR_COUNT_1 .. _4 by the width), 0, 1, 0, 0 */
        Op::LdPka { .. } => Props { msg: Msg::Load, sr_count: SrCount::Format, sr_write: true, ..ALU },
        _ => ALU,
    }
}

/// `va_src_info(op, s)`'s fields this back end reads (valhall.h:56-66),
/// per the op's Valhall sources: the staging sources that are read, then
/// the sources (valhall.c.py:117-130).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SrcInfo {
    pub absneg: bool,
    pub swizzle: bool,
    pub widen: bool,
    pub lanes: bool,
    pub halfswizzle: bool,
    pub lane: bool,
    /// The source's size in bits: 8, 16, 32 or 64 (`VA_SIZE_*`); 8 for a
    /// pseudo-op, whose zeroed entry reads as `VA_SIZE_8`.
    pub bits: u8,
}

const fn info(absneg: bool, swizzle: bool, widen: bool, lane: bool, bits: u8) -> SrcInfo {
    SrcInfo { absneg, swizzle, widen, lanes: false, halfswizzle: false, lane, bits }
}

/// A pseudo-op's (`COLLECT`, `SPLIT`) or a missing source's entry: all
/// zero, `VA_SIZE_8`.
const NO_INFO: SrcInfo = info(false, false, false, false, 8);

/// `va_src_info(op, s)` (valhall.h:132-137, including its `va_swap_12` for
/// BLEND), transcribed from Mesa 26.2.1's generated valhall.c: each line
/// is that entry's `absneg swizzle widen lane size`.
pub const fn src_info(op: Op, s: usize) -> SrcInfo {
    let s = match op {
        /* va_swap_12 */
        Op::Blend { .. } if s == 1 || s == 2 => 3 - s,
        _ => s,
    };
    match (op, s) {
        (Op::FmaF32, 0..=2) => info(true, true, false, false, 32),
        (Op::FmaV2f16, 0..=2) => info(true, true, false, false, 16),
        (Op::FaddV2f16, 0..=1) => info(true, true, false, false, 16),
        (Op::FaddImmF32(_), 0) => info(false, false, false, false, 32),
        (Op::FaddImmV2f16(_), 0) => info(false, false, false, false, 16),
        /* a pseudo-op: no Valhall entry; its source is read as its FADD's */
        (Op::FabsnegF32, 0) => info(true, true, false, false, 32),
        (Op::FabsnegV2f16, 0) => info(true, true, false, false, 16),
        (Op::FaddF32 | Op::FmaxF32 | Op::FminF32, 0..=1) => info(true, true, false, false, 32),
        (Op::FmaxV2f16 | Op::FminV2f16, 0..=1) => info(true, true, false, false, 16),
        (Op::FrcpF32 | Op::FrsqF32 | Op::FroundF32 { .. }, 0) => info(true, true, false, false, 32),
        (Op::FroundV2f16 { .. }, 0) => info(true, true, false, false, 16),
        /* a pseudo-op (IR_pseudo.xml:139-147: a plain source) */
        (Op::FclampF32, 0) => info(false, false, false, false, 32),
        (Op::FclampV2f16, 0) => info(false, false, false, false, 16),
        (Op::FcmpOrF32 { .. }, 0..=1) => info(true, true, false, false, 32),
        (Op::FcmpOrV2f16 { .. }, 0..=1) => info(true, true, false, false, 16),
        (Op::FcmpOrV2f16 { .. }, 2) => info(false, false, false, false, 32),
        /* DISCARD.f32: both sources float (absneg, widen), valhall.c */
        (Op::DiscardF32 { .. }, 0..=1) => info(true, true, false, false, 32),
        /* a pseudo-op: its one source is the mask word */
        (Op::DiscardB32, 0) => info(false, false, false, false, 32),
        (Op::FcmpOrF32 { .. } | Op::IcmpOrS32 { .. } | Op::IcmpOrU32 { .. }, 2) => info(false, false, false, false, 32),
        /* an integer source widens (a half or a byte, zero- or sign-extended:
         * `widen = 1`, valhall.c ICMP_OR.u32/.s32 and IADD.s32); it does
         * not swizzle */
        (Op::IcmpOrS32 { .. } | Op::IcmpOrU32 { .. } | Op::IaddS32, 0..=1) => info(false, false, true, false, 32),
        (Op::MuxI32 | Op::MuxBitI32 | Op::CselU32 | Op::CselS32 { .. }, 0..=3) => info(false, false, false, false, 32),
        /* IABS.s32 and IMUL.i32: integer sources that widen (valhall.c) */
        (Op::IabsS32, 0) | (Op::ImulI32, 0..=1) => info(false, false, true, false, 32),
        /* ISUB.s32 widens its sources as IADD.s32; U32_TO_F32 and
         * F32_TO_U32 read a plain word, F32_TO_U32's float with abs and
         * neg as F32_TO_S32's (valhall.c) */
        (Op::IsubS32, 0..=1) => info(false, false, true, false, 32),
        (Op::U32ToF32, 0) => info(false, false, false, false, 32),
        (Op::F32ToU32, 0) => info(true, false, false, false, 32),
        /* the coordinates: absneg and widen, as FMA.f32's (valhall.c
         * CUBEFACE1, CUBEFACE2_V9, CUBE_SSEL, CUBE_TSEL); the face index a
         * plain word */
        (Op::Cubeface1 | Op::Cubeface2, 0..=2) => info(true, true, false, false, 32),
        (Op::CubeSsel | Op::CubeTsel, 0..=1) => info(true, true, false, false, 32),
        (Op::CubeSsel | Op::CubeTsel, 2) => info(false, false, false, false, 32),
        (Op::MuxV2i16, 0..=2) => info(false, true, false, false, 16),
        (Op::F32ToS32, 0) => info(true, false, false, false, 32),
        (Op::S32ToF32, 0) => info(false, false, false, false, 32),
        (Op::U16ToU32 | Op::S16ToS32, 0) => info(false, false, false, true, 16),
        (Op::U16ToF32, 0) => info(false, false, false, true, 16),
        (Op::F16ToF32 | Op::F16ToS32, 0) => info(true, false, false, true, 16),
        (Op::U8ToF32, 0) => info(false, false, false, true, 8),
        (Op::MovI32 | Op::IaddImmI32(_), 0) => info(false, false, false, false, 32),
        (Op::V2f32ToV2f16, 0..=1) => info(true, false, false, false, 16),
        (Op::MkvecV2i16, 0..=1) => info(false, false, false, true, 16),
        (Op::IaddV2u16, 0..=1) => info(false, true, false, false, 16),
        /* the two byte lanes, then the 16-bit half read whole */
        (Op::MkvecV2i8, 0..=1) => info(false, false, false, true, 8),
        (Op::MkvecV2i8, 2) => info(false, false, false, false, 16),
        (Op::ShaddxS64, 0) => info(false, false, false, false, 64),
        (Op::ShaddxS64, 1) => info(false, false, true, false, 64),
        (Op::LdAttrImm { .. }, 0..=1) => info(false, false, false, false, 32),
        (Op::LeaBufImm { .. }, 0) => info(false, false, false, false, 32),
        (Op::StoreI32 { .. } | Op::StoreI16 { .. }, 0) => info(false, false, false, false, 32),
        (Op::StoreI32 { .. } | Op::StoreI16 { .. }, 1) => info(false, false, false, false, 64),
        (Op::LdVarBufImmF32 { .. } | Op::LdVarSpecialF32 { .. } | Op::LdVarImmF32 { .. }, 0) => {
            info(false, false, false, false, 32)
        }
        (Op::Atest, 0) | (Op::Atest, 2) => info(false, false, false, false, 32),
        (Op::Atest, 1) => info(false, true, false, false, 32),
        (Op::ZsEmit { .. }, 0..=2) => info(false, false, false, false, 32),
        (Op::Blend { .. }, 0) | (Op::Blend { .. }, 2) => info(false, false, false, false, 32),
        (Op::Blend { .. }, 1) => info(false, false, false, false, 64),
        (Op::BranchzI { .. }, 0..=1) => info(false, false, false, false, 32),
        /* LSHIFT_AND.i32: the value (widen), then, swapped by va_swap_12,
         * BIR's mask is Valhall's third source (32 bits, notted) and BIR's
         * shift its second (an 8-bit `lanes` source) */
        (
            Op::LshiftAndI32
            | Op::LshiftOrI32
            | Op::LshiftXorI32
            | Op::RshiftAndI32
            | Op::RshiftOrI32
            | Op::ArshiftOrI32,
            0,
        ) => info(false, false, true, false, 32),
        (
            Op::LshiftAndI32
            | Op::LshiftOrI32
            | Op::LshiftXorI32
            | Op::RshiftAndI32
            | Op::RshiftOrI32
            | Op::ArshiftOrI32,
            1,
        ) => info(false, false, false, false, 32),
        (
            Op::LshiftAndI32
            | Op::LshiftOrI32
            | Op::LshiftXorI32
            | Op::RshiftAndI32
            | Op::RshiftOrI32
            | Op::ArshiftOrI32,
            2,
        ) => SrcInfo { lanes: true, ..info(false, false, false, false, 8) },
        /* the byte offset, then the mode descriptor (va_pack.c:746-769) */
        (Op::LdPka { .. }, 0..=1) => info(false, false, false, false, 32),
        (Op::LdTile { .. }, 0..=2) => info(false, false, false, false, 32),
        /* `<src combine="true">`: a 32-bit word whose half the branch reads
         * is its swizzle (va_pack.c:680-684, packed as a widen); the
         * constant zero of a jump is read whole (`0x0`, the table's entry) */
        (Op::Branchz { .. }, 0) => info(false, false, false, false, 32),
        (Op::TexSingle { .. }, 0) => info(false, false, false, false, 32),
        (Op::TexSingle { .. }, 1) => info(false, false, false, false, 64),
        _ => NO_INFO,
    }
}

/// `valhall_opcodes[op].nr_staging_srcs`.
pub const fn nr_staging_srcs(op: Op) -> usize {
    match op {
        Op::StoreI32 { .. } | Op::StoreI16 { .. } | Op::Blend { .. } | Op::TexSingle { .. } => 1,
        _ => 0,
    }
}

/// `valhall_opcodes[op].nr_srcs` (the sources after the staging ones).
pub const fn nr_srcs(op: Op) -> usize {
    match op {
        Op::Nop | Op::Collect | Op::Split | Op::Phi => 0,
        /* the value compared (Mesa's second source, the zero, is not
         * packed: the offset takes its place) */
        Op::Branchz { .. } => 1,
        Op::DiscardB32 => 1,
        Op::DiscardF32 { .. } => 2,
        Op::MovI32
        | Op::IaddImmI32(_)
        | Op::FaddImmF32(_)
        | Op::FaddImmV2f16(_)
        | Op::FabsnegF32
        | Op::FabsnegV2f16
        | Op::FclampF32
        | Op::FclampV2f16
        | Op::FrcpF32
        | Op::FrsqF32
        | Op::FroundF32 { .. }
        | Op::FroundV2f16 { .. }
        | Op::F32ToS32
        | Op::S32ToF32
        | Op::S16ToS32
        | Op::U16ToU32
        | Op::U16ToF32
        | Op::F16ToF32
        | Op::F16ToS32
        | Op::U8ToF32
        | Op::IabsS32
        | Op::U32ToF32
        | Op::F32ToU32
        | Op::LeaBufImm { .. }
        | Op::LdVarBufImmF32 { .. }
        | Op::LdVarSpecialF32 { .. }
        | Op::LdVarImmF32 { .. } => 1,
        Op::StoreI32 { .. } | Op::StoreI16 { .. } | Op::TexSingle { .. } => 1,
        Op::FaddF32
        | Op::FaddV2f16
        | Op::FmaxF32
        | Op::FmaxV2f16
        | Op::FminF32
        | Op::FminV2f16
        | Op::IaddS32
        | Op::IsubS32
        | Op::ImulI32
        | Op::V2f32ToV2f16
        | Op::MkvecV2i16
        | Op::IaddV2u16
        | Op::ShaddxS64 => 2,
        Op::LdAttrImm { .. } | Op::Blend { .. } | Op::BranchzI { .. } | Op::LdPka { .. } => 2,
        Op::LshiftAndI32
        | Op::LshiftOrI32
        | Op::LshiftXorI32
        | Op::RshiftAndI32
        | Op::RshiftOrI32
        | Op::ArshiftOrI32
        | Op::MkvecV2i8
        | Op::LdTile { .. } => 3,
        Op::FmaF32 | Op::FmaV2f16 | Op::Atest | Op::ZsEmit { .. } => 3,
        Op::FcmpOrF32 { .. } | Op::IcmpOrS32 { .. } | Op::IcmpOrU32 { .. } | Op::MuxI32 | Op::MuxV2i16 => 3,
        Op::FcmpOrV2f16 { .. } => 3,
        Op::MuxBitI32 | Op::Cubeface1 | Op::Cubeface2 | Op::CubeSsel | Op::CubeTsel => 3,
        Op::CselU32 | Op::CselS32 { .. } => 4,
    }
}

/// `valhall_opcodes[op].is_signed` (valhall.py:198: the name's type, after
/// its dot, has an `s`; `ICMP_OR.u32` has none).
pub const fn is_signed(op: Op) -> bool {
    matches!(
        op,
        Op::ShaddxS64
            | Op::IaddS32
            | Op::IsubS32
            | Op::IcmpOrS32 { .. }
            | Op::CselS32 { .. }
            | Op::IabsS32
            | Op::S16ToS32
    )
}

/// One instruction (`bi_instr`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ins {
    pub op: Op,
    pub dest: [Dest; MAX_DESTS],
    pub nd: u8,
    pub src: [Src; MAX_SRCS],
    pub ns: u8,
    /// `enum va_flow` (valhall/ISA.xml "Flow"); before the flow pass a
    /// bitmask of slots to wait on.
    pub flow: u8,
    /// The message slot (`va_assign_slots`).
    pub slot: u8,
    /// `bi_instr::clamp` ([`clamp`]): a float op's result clamp.
    pub clamp: u8,
    /// The block the instruction belongs to ([`Prog::blocks`]).
    pub block: u8,
}

/// `enum va_flow` values (valhall/ISA.xml:69-84).
pub mod flow {
    pub const NONE: u8 = 0;
    pub const WAIT0: u8 = 1;
    pub const WAIT0126: u8 = 8;
    pub const WAIT: u8 = 9;
    pub const RECONVERGE: u8 = 10;
    pub const DISCARD: u8 = 13;
    pub const END: u8 = 15;
    /// `va_flow_is_wait_or_none` (valhall.h:139-143).
    pub const fn is_wait_or_none(f: u8) -> bool {
        f <= WAIT
    }
}

impl Ins {
    pub const NOP: Ins = Ins {
        op: Op::Nop,
        dest: [Dest::NULL; MAX_DESTS],
        nd: 0,
        src: [Src::NULL; MAX_SRCS],
        ns: 0,
        flow: 0,
        slot: 0,
        clamp: clamp::NONE,
        block: 0,
    };

    pub fn new(op: Op, dests: &[Dest], srcs: &[Src]) -> Ins {
        let mut i = Ins { op, ..Ins::NOP };
        i.dest[..dests.len()].copy_from_slice(dests);
        i.nd = dests.len() as u8;
        i.src[..srcs.len()].copy_from_slice(srcs);
        i.ns = srcs.len() as u8;
        i
    }

    pub fn nop(flow: u8) -> Ins {
        Ins { flow, ..Ins::NOP }
    }

    pub fn dests(&self) -> &[Dest] {
        &self.dest[..self.nd as usize]
    }

    pub fn srcs(&self) -> &[Src] {
        &self.src[..self.ns as usize]
    }

    /// `bi_count_staging_registers` (bir.c:46-65).
    pub fn staging_count(&self) -> u32 {
        match props(self.op).sr_count {
            SrCount::Fixed(n) => n as u32,
            SrCount::Format => match self.op {
                Op::LdAttrImm { vecsize, .. }
                | Op::LdVarBufImmF32 { vecsize, .. }
                | Op::LdVarSpecialF32 { vecsize, .. }
                | Op::LdVarImmF32 { vecsize, .. } => vecsize as u32,
                /* a 16-bit register format packs two channels a register */
                Op::LdTile { vecsize, f16 } => {
                    if f16 {
                        (vecsize as u32).div_ceil(2)
                    } else {
                        vecsize as u32
                    }
                }
                /* `BI_SR_COUNT_<n>` of the width's opcode (bi_opcodes.c) */
                Op::LdPka { words, .. } => words as u32,
                _ => 0,
            },
            SrCount::Own => match self.op {
                Op::Blend { sr_count, .. } | Op::TexSingle { sr_count, .. } => sr_count as u32,
                _ => 0,
            },
        }
    }

    /// `bi_count_read_registers` (bir.c:67-84) for the ops here.
    pub fn count_read(&self, s: usize) -> u32 {
        if s == 0 && props(self.op).sr_read {
            self.staging_count()
        } else if s == 0 && self.op == Op::Split {
            self.nd as u32
        } else if self.op == Op::ShaddxS64 {
            2
        } else {
            1
        }
    }

    /// `bi_count_write_registers` (bir.c:87-140) for the ops here.
    pub fn count_write(&self, d: usize) -> u32 {
        if let (0, Op::TexSingle { mask, f16, .. }) = (d, self.op) {
            /* a register a channel written, two to a register at 16 bits */
            let chans = mask.count_ones();
            return if f16 { chans.div_ceil(2) } else { chans };
        }
        if d == 0 && props(self.op).sr_write {
            self.staging_count()
        } else if self.op == Op::ShaddxS64 {
            2
        } else if self.op == Op::Collect && d == 0 {
            self.ns as u32
        } else {
            1
        }
    }

    /// `bi_writemask` (bir.c:142-147): the words written, at the
    /// destination's offset.
    pub fn writemask(&self, d: usize) -> u32 {
        let off = match self.dest[d].idx {
            Idx::Ssa(_, off) => off as u32,
            _ => 0,
        };
        ((1u32 << self.count_write(d)) - 1) << off
    }

    /// `bi_is_staging_src` (compiler.h): source 0 or 4 of an op that reads
    /// staging registers.
    pub fn is_staging_src(&self, s: usize) -> bool {
        (s == 0 || s == 4) && props(self.op).sr_read
    }

    /// `bi_side_effects` (bir.c:181-221) for the ops here: a `last`
    /// instruction, the message passes with an effect, and a `DISCARD`
    /// (either form, :189-192), which has no destination to be read.
    pub fn side_effects(&self) -> bool {
        let p = props(self.op);
        p.last
            || matches!(p.msg, Msg::Store | Msg::Atest | Msg::Blend | Msg::ZStencil)
            || matches!(self.op, Op::DiscardF32 { .. } | Op::DiscardB32)
    }
}

/// A basic block's edges (`bi_block`): its successors (at most two: an
/// `if`'s branch target then its fall-through, `emit_if`) and its
/// predecessors in the order they were added (`bi_block_add_successor`
/// appends to the successor's predecessor list), and the flags the passes
/// set on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Block {
    pub succ: [Option<u8>; 2],
    pub pred: [u8; MAX_PREDS],
    pub npred: u8,
    /// `bi_block::needs_nop`: a branch targets this terminal block, which
    /// then ends with an explicit `NOP.end` (bifrost_compile.c
    /// `bi_lower_branch`, va_insert_flow.c:485-489).
    pub needs_nop: bool,
    /// Removed by `bi_opt_control_flow` (opt.rs `control_flow`): no
    /// instruction, no edge, not laid out.
    pub dead: bool,
}

impl Block {
    pub const NONE: Block = Block { succ: [None; 2], pred: [0; MAX_PREDS], npred: 0, needs_nop: false, dead: false };

    pub fn preds(&self) -> &[u8] {
        &self.pred[..self.npred as usize]
    }

    pub fn succs(&self) -> impl Iterator<Item = u8> + '_ {
        self.succ.iter().flatten().copied()
    }

    pub fn nsucc(&self) -> usize {
        self.succ.iter().flatten().count()
    }

    /// `bi_predecessor_index`.
    pub fn pred_index(&self, b: u8) -> Option<usize> {
        self.preds().iter().position(|&p| p == b)
    }
}

/// A program: its instructions in the scratch arena, in block order.
pub struct Prog<'s> {
    pub ins: &'s mut [Ins],
    pub n: usize,
    /// The next SSA value (`ctx->ssa_alloc`).
    pub ssa_alloc: u16,
    /// The most instructions the program held at once (its high-water in
    /// the [`MAX_INSTRS`] table: the constant and FAU moves come and the
    /// CSE takes some away).
    pub peak: usize,
    /// The blocks, in layout order (`ctx->blocks`); block 0 is the entry.
    pub blocks: [Block; MAX_BLOCKS],
    pub nblocks: u8,
    /// The block [`Prog::push`] emits into (`ctx->current_block`).
    pub cur: u8,
}

/// The program would not fit one of its tables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Full {
    /// [`MAX_INSTRS`] instructions.
    Instrs,
    /// [`MAX_NODES`] SSA values.
    Values,
    /// [`MAX_BLOCKS`] blocks, or a block's edges.
    Blocks,
}

impl<'s> Prog<'s> {
    pub fn new(ins: &'s mut [Ins]) -> Prog<'s> {
        Prog { ins, n: 0, ssa_alloc: 0, peak: 0, blocks: [Block::NONE; MAX_BLOCKS], nblocks: 1, cur: 0 }
    }

    pub fn as_slice(&self) -> &[Ins] {
        &self.ins[..self.n]
    }

    /// A fresh SSA value (`bi_temp`).
    pub fn temp(&mut self) -> Result<u16, Full> {
        if self.ssa_alloc as usize >= MAX_NODES {
            return Err(Full::Values);
        }
        self.ssa_alloc += 1;
        Ok(self.ssa_alloc - 1)
    }

    /// A new block after every block so far (`create_empty_block` then
    /// `list_addtail`); its index, which is its place in the layout.
    pub fn new_block(&mut self) -> Result<u8, Full> {
        if self.nblocks as usize >= MAX_BLOCKS {
            return Err(Full::Blocks);
        }
        self.nblocks += 1;
        Ok(self.nblocks - 1)
    }

    /// `bi_block_add_successor`: `to` after `from`'s successors, `from`
    /// appended to `to`'s predecessors (once).
    pub fn add_successor(&mut self, from: u8, to: u8) -> Result<(), Full> {
        let f = &mut self.blocks[from as usize];
        if f.succ.contains(&Some(to)) {
            return Ok(());
        }
        let Some(slot) = f.succ.iter_mut().find(|s| s.is_none()) else { return Err(Full::Blocks) };
        *slot = Some(to);
        let t = &mut self.blocks[to as usize];
        if t.preds().contains(&from) {
            return Ok(());
        }
        if t.npred as usize >= MAX_PREDS {
            return Err(Full::Blocks);
        }
        t.pred[t.npred as usize] = from;
        t.npred += 1;
        Ok(())
    }

    /// The blocks in layout order that are not dead.
    pub fn live_blocks(&self) -> impl Iterator<Item = u8> + '_ {
        (0..self.nblocks).filter(|&b| !self.blocks[b as usize].dead)
    }

    /// The index of block `b`'s first instruction: the instructions of the
    /// blocks before it (an empty block starts where the next one does).
    pub fn block_start(&self, b: u8) -> usize {
        self.as_slice().iter().take_while(|i| i.block < b).count()
    }

    /// One past block `b`'s last instruction.
    pub fn block_end(&self, b: u8) -> usize {
        self.as_slice().iter().take_while(|i| i.block <= b).count()
    }

    /// Append to the block being emitted (`bi_after_block(current_block)`).
    pub fn push(&mut self, i: Ins) -> Result<usize, Full> {
        let at = self.block_end(self.cur);
        self.insert_in(at, i, self.cur)?;
        Ok(at)
    }

    /// Insert before the instruction at `at`, into its block (`at == n`:
    /// after the last instruction, into the last block).
    pub fn insert(&mut self, at: usize, i: Ins) -> Result<(), Full> {
        let block = if at < self.n {
            self.ins[at].block
        } else if self.n > 0 {
            self.ins[self.n - 1].block
        } else {
            self.cur
        };
        self.insert_in(at, i, block)
    }

    /// Insert after the instruction at `at`, into its block
    /// (`bi_after_instr`).
    pub fn insert_after(&mut self, at: usize, i: Ins) -> Result<(), Full> {
        let block = self.ins[at].block;
        self.insert_in(at + 1, i, block)
    }

    /// Append to block `b` (`bi_after_block`), wherever it stands.
    pub fn insert_end(&mut self, b: u8, i: Ins) -> Result<usize, Full> {
        let at = self.block_end(b);
        self.insert_in(at, i, b)?;
        Ok(at)
    }

    /// Insert at position `at` as an instruction of block `block`, which
    /// must be the block the position falls in.
    pub fn insert_in(&mut self, at: usize, mut i: Ins, block: u8) -> Result<(), Full> {
        if self.n >= self.ins.len() {
            return Err(Full::Instrs);
        }
        i.block = block;
        self.ins.copy_within(at..self.n, at + 1);
        self.ins[at] = i;
        self.n += 1;
        self.peak = self.peak.max(self.n);
        Ok(())
    }

    pub fn remove(&mut self, at: usize) {
        self.ins.copy_within(at + 1..self.n, at);
        self.n -= 1;
    }

    /// The last instruction of block `b`, if it has one.
    pub fn last_of(&self, b: u8) -> Option<usize> {
        let end = self.block_end(b);
        (end > self.block_start(b)).then(|| end - 1)
    }
}
