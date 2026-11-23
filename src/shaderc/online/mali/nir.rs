// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014-2025 Intel Corporation, Valve Corporation, Collabora Ltd.
// Derived from Mesa 26.2.1 src/compiler/nir/nir_opt_move.c,
// nir_lower_viewport_transform.c and the fusion rules of
// nir_opt_algebraic.py (MIT). The upstream part stays under its licence
// (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
//! A vertex shader as Mesa's NIR has it when instruction selection starts
//! (the program `BIFROST_MESA_DEBUG=shaders` prints first), for the shapes
//! this back end takes: scalar float arithmetic over attributes, uniforms
//! and constants.
//!
//! The front end's IR is walked in its order, which is the order of the
//! GLSL source as Mesa's GLSL front end emits it too, and each step Mesa
//! takes on the way is done here, by name:
//!
//! * `nir_lower_alu_width`: a vector operation becomes one scalar operation
//!   per lane, in lane order, where the vector one stood; a dot product
//!   becomes the product of its last lanes and a fused multiply-add per
//!   lane below it (measured: tools/mali/testdata/sel_u_vecmat);
//! * `nir_opt_constant_folding` and the algebraic rules these shapes meet
//!   (`x + 0.0`, `x * 1.0`, `x * 0.0` under GL's float controls,
//!   nir_opt_algebraic.py), at the instruction they fold;
//! * `nir_opt_cse`: an instruction equal to an earlier one (commutative
//!   operands in either order) is the earlier one;
//! * `nir_lower_viewport_transform` (nir_lower_viewport_transform.c:62-96)
//!   on the position, before its store;
//! * `pan_nir_lower_vs_outputs`: per output a `lea_buf_pan`, then one store
//!   per lane, at the end of the program;
//! * the late fusion (nir_opt_algebraic.py:3793-3816): `fadd(fmul(a, b),
//!   c)` becomes `ffma(a, b, c)` where the `fadd` stood, when every use of
//!   the product is an `fadd`; the commutative match takes the `fadd`'s
//!   first source first;
//! * `nir_opt_dce`, `nir_opt_shrink_vectors` (a load keeps the lanes up to
//!   the highest one read: measured on sel_u_zw and sel_u_yz);
//! * `nir_opt_move` (nir_opt_move.c), transcribed: each load moves to just
//!   before its first user, which is where `LD_ATTR` stands in the BIR.
//!
//! Constants are not instructions here (`load_const` moves too, but makes
//! no BIR). What the IR has that these steps do not cover is refused with
//! its own gap ([`MaliError`]).

use super::bir::{clamp, Cmpf, Swz, Sysval, MAX_BLOCKS};
use super::fau;
use super::select::{self, generic_layout, VaryingAt, MAX_VARYINGS};
use super::{Defect, MaliError};
use crate::gl::consts::{
    GL_BOOL, GL_BOOL_VEC2, GL_BOOL_VEC3, GL_BOOL_VEC4, GL_FLOAT, GL_FLOAT_MAT2, GL_FLOAT_MAT3, GL_FLOAT_MAT4,
    GL_FLOAT_VEC2, GL_FLOAT_VEC3, GL_FLOAT_VEC4, GL_INT, GL_INT_VEC2, GL_INT_VEC3, GL_INT_VEC4, GL_UNSIGNED_INT,
    GL_UNSIGNED_INT_VEC2, GL_UNSIGNED_INT_VEC3, GL_UNSIGNED_INT_VEC4,
};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::fold;
use crate::shaderc::online::ir::{
    self, AluOp, IoKind, IoSlot, Module, Node as IrNode, Op as IrOp, Prec, ScalarType, TexDim, TexLod, ValueId,
};

/// The instructions one vertex shader may hold here (the fragment walk's
/// too). The largest measured, femtovg's filter_image (24 unrolled
/// passes), takes 1153; fill_color with 256 offset arms (case 194's
/// offarm256) 611 (docs/notes/mali-int-and-size.md).
pub const MAX_NODES: usize = 1536;
/// The IR values the walk keeps lanes for (the fragment walk's too).
/// offarm256 takes 1658, about six per arm.
pub const MAX_VALUES: usize = 2048;
/// The local variables the walk keeps lanes for (the fragment walk's too).
/// Mesa has no such count (a local is SSA values by then); 256 holds
/// glmark2's terrain noise (fs 1637: 195 locals out of the front end, its
/// inlined calls' parameters among them) and dEQP tmp_array's 40-element
/// arrays (docs/notes/mali-local-arrays.md section 7). One table of every
/// local's lanes is on the walk's stack (8 KiB at 256); the copies the walk
/// nests are `nir_fs.rs` `Fs::pool`'s, in the scratch arena and as wide as
/// the module's own count.
pub const MAX_LOCALS: usize = 256;
/// The `if`s one shader may hold ([`IfRegion`]): gl_test's databreak (case
/// 87) has nine, the eight guarded passes of its unrolled loop and the
/// `continue` of one of them; femtovg's filter_image 27, the 24 guards of
/// its passes among them.
pub const MAX_IFS: usize = 32;
/// The loops Mesa keeps ([`LoopRegion`]) one shader may hold
/// (intloop_loop_nested holds nine).
pub const MAX_LOOPS: usize = 16;

/// `nir_opt_peephole_select`'s limit as bifrost calls it
/// (bifrost_nir.c:234-237: `.limit = 64, .expensive_alu_ok = true`).
pub const PEEPHOLE_LIMIT: u32 = 64;
/// The limit of the GL front end's own call (`gl_nir_opts`, gl_nir_linker.c;
/// the repository's oracle copy is pan_shc.c:156-161: `limit = 8,
/// indirect_load_ok, expensive_alu_ok`), which runs before the uniforms
/// become UBO loads.
pub const PEEPHOLE_LIMIT_GL: u32 = 8;

/// A source: a constant (32-bit, or a pair of fp16 halves for a 16-bit
/// two-lane node), lane `c` of node `n`, or the two 16-bit lanes of node
/// `n` through a swizzle (a two-lane node's source, nir_alu_src).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Val {
    Undef,
    Const(u32),
    Lane(u16, u8),
    Pair(u16, Swz),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alu {
    Fadd,
    Fmul,
    Ffma,
    Frcp,
    Fmax,
    Fmin,
    /// `f2f16`: the fp16 value in the low half.
    F2f16,
    /// `fneg` (`fmul(a, -1.0)`, nir_opt_algebraic.py:296): a source
    /// modifier of its ALU users, an `FADD.f32 a.neg, -0.0` for a store's.
    Fneg,
    Fabs,
    /// `vec2` of two 16-bit halves (`MKVEC.v2i16`).
    Vec2,
    /// `fsat` (`fmin(fmax(a, 0.0), 1.0)`, nir_opt_algebraic.py:1088),
    /// `fclamp_pos` (`fmax(a, 0.0)`) and `fsat_signed`
    /// (bifrost_nir_algebraic.py:34-36): `FCLAMP` with the clamp
    /// ([`super::bir::clamp`]).
    Fclamp(u8),
    /// `frsq`: half of `fsqrt` (`lower_fsqrt`: `frcp(frsq(a))`).
    Frsq,
    /// `ffloor` (`FROUND.rtn`); `ffract` is `fadd(a, fneg(ffloor(a)))`
    /// (`lower_ffract`, nir_opt_algebraic.py:546).
    Ffloor,
    /// `fceil` (`FROUND.rtp`).
    Fceil,
    /// `ftrunc` (`FROUND.rtz`): `i2f32(f2i32(a))` (nir_opt_algebraic.py:1250).
    Ftrunc,
    /// `flt_pan`, `fge_pan`, `feq_pan` (`pan_nir_lower_bool_to_bitsize`):
    /// a 32-bit 0 / ~0 mask (`FCMP.f32 .m1`). GLSL's `<=` is `fge` with
    /// its sources swapped (glsl_to_nir.cpp:2488).
    Fcmp(Cmpf),
    /// `ilt_pan` and the other integer compares, the same mask.
    Icmp(Cmpf),
    /// `b2f32`: `bcsel_pan(c, 1.0, 0.0)` (bifrost_nir_algebraic.py:127),
    /// `MUX.i32.int_zero 0.0, 1.0, c`, which folds into the compare's
    /// result type (`.f1`) when it is the compare's one use.
    B2f,
    /// `bcsel_pan(c, then, else)`: `MUX.i32.int_zero else, then, c`.
    Bcsel,
    /// `f2i32`: `F32_TO_S32.rtz`.
    F2i,
    /// `i2f32`: `S32_TO_F32`.
    I2f,
    /// `iadd` of two 32-bit integers.
    Iadd,
    /// `u2f32` of a 16-bit source (`U16_TO_F32`, bifrost_compile.c:3167-3176):
    /// a lane of the pixel position ([`Kind::PixelCoord`]).
    U2f32,
    /// `u2u32` of a 16-bit source (`U16_TO_U32`, bifrost_compile.c:3216-3226):
    /// a vertex look-up's 8.8 level ([`Kind::VsTex`]).
    U2u32,
    /// `f2f32` of a 16-bit source (`F16_TO_F32`, bifrost_compile.c's
    /// `nir_op_f2f32`): a mediump fragment shader's computed texture
    /// coordinate, which the look-up reads at fp32 (nir_fs.rs
    /// `TexCoord::Computed`, case121.expect.json).
    F2f32,
    /// `ishl` of a 32-bit integer by a constant (`LSHIFT_OR.i32 a, #0,
    /// shift`): the byte offset of a uniform array element read at a
    /// run-time index, `index << log2(element bytes)` (nir_lower_io's
    /// `index * stride`, `nir_lower_uniforms_to_ubo`'s `* 4`, folded by
    /// nir_opt_algebraic into one shift; [`Kind::UniformIdx`]).
    Ishl,
    /// `cubeface_pan(x, y, z)`'s first lane, the largest absolute
    /// coordinate (`CUBEFACE1`; a cube look-up's `build_cube_coord2_face`,
    /// pan_nir_lower_tex.c:68-100, the intrinsic nir_intrinsics.py:1767).
    Cubeface1,
    /// `cubeface_pan(x, y, z)`'s second lane, the face index preshifted
    /// into bits 29-31 (`CUBEFACE2_V9`). An integer.
    Cubeface2,
    /// `cube_ssel_pan(z, x, face)`: the face's S coordinate (`CUBE_SSEL`).
    CubeSsel,
    /// `cube_tsel_pan(y, z, face)`: the face's T coordinate (`CUBE_TSEL`).
    CubeTsel,
    /// `bitfield_select(mask, b, c)`: `b`'s bits under the mask, `c`'s
    /// above it (`MUX.i32.bit`; `build_cube_desc`'s S word,
    /// pan_nir_lower_tex.c:103-128: `s` below bit 29, the face above). An
    /// integer.
    BitfieldSelect,
    /// `iand` of two 32-bit integers (`LSHIFT_AND.i32 a, b, #0`;
    /// `nir_copysign`'s two masks, atan's sign).
    Iand,
    /// `ior` of two 32-bit integers (`LSHIFT_OR.i32 a, b, #0`).
    Ior,
    /// `imax` and `imin` of two 32-bit signed integers (`CSEL.s32.gt a, b,
    /// a, b` and `.lt`, bifrost_compile.c:3061-3071): `isign`'s lowering,
    /// `imin(imax(a, -1), 1)` (nir_opt_algebraic.py:2812, `lower_isign`).
    Imax,
    Imin,
    /// `iabs` of a 32-bit signed integer (`IABS.s32`).
    Iabs,
    /// `ushr` of a 32-bit integer by a constant (`RSHIFT_OR.i32 a, #0,
    /// shift`, the shift byte 0 of its source).
    Ushr,
    /// `imul` of two 32-bit integers (`IMUL.i32`).
    Imul,
    /// `i2i32` of a 16-bit boolean (`S16_TO_S32`, bifrost_compile.c:
    /// 3205-3211): the mask a 32-bit `bcsel_pan` reads when its condition
    /// is a 16-bit compare's (`pan_nir_lower_bool_to_bitsize`'s
    /// `resize_bool_alu_source`).
    I2i32,
    /// `f2i32` of a 16-bit source (`F16_TO_S32.rtz`, bifrost_compile.c:
    /// 3100): a mediump value's integer (case 72's `int(u_in.x)`, the
    /// half `u2.w0.h0`).
    F2i32,
    /// `inot` of a 32-bit boolean: GLSL's `!` on a run-time value. The
    /// walk folds it into its users (`inot(inot(a))`, `iand(a, a)`,
    /// `bcsel(inot(a), b, c)` as `bcsel(a, c, b)`, nir_opt_algebraic.py);
    /// one left for selection has no reference.
    Inot,
    /// `ishr` of a 32-bit integer (`ARSHIFT_OR.i32 a, #0, b.b0`,
    /// bifrost_compile.c:3053-3055): intuni_ishr's `v.x >> v.y`.
    Ishr,
    /// `u2f32(extract_u8(a, k))`, byte `k` of a word to fp32 (`U8_TO_F32
    /// a.bk`): what nir_opt_algebraic makes of `i2f32(iand(a, 0xff))` (`k`
    /// 0) and `i2f32(iand(ishr(a, 8k), 0xff))` (intuni_ishr's `float(s &
    /// 255)` and `float((s >> 8) & 255)`).
    U8f32(u8),
    /// `ixor` of two 32-bit integers (`LSHIFT_XOR.i32 a, b, #0`,
    /// bifrost_compile.c:3383-3385): `nir_lower_idiv`'s sign of a quotient.
    Ixor,
    /// `isub` (`ISUB.s32 a, b`, bifrost_compile.c:3355-3357).
    Isub,
    /// `ineg` (`ISUB.s32 0, a`, bifrost_compile.c:3351-3353).
    Ineg,
    /// `u2f32` of a 32-bit integer (`U32_TO_F32`, bifrost_compile.c:
    /// 3171-3172), where [`Alu::U2f32`] is of a 16-bit one.
    U2f,
    /// `f2u32` of an fp32 value (`F32_TO_U32.rtz`, bifrost_compile.c:
    /// 3137-3150).
    F2u,
    /// `extract_u16(a, k)`: half `k` of a word, zero-extended (`U16_TO_U32
    /// a.hk`, bifrost_compile.c:3013-3025): `nir_lower_idiv`'s 32 x 32
    /// high product from 16-bit halves.
    Extract16(u8),
    /// `b2i32` of a 32-bit boolean (`MUX.i32.int_zero 0, 1, c`,
    /// bifrost_compile.c:3246-3251), which `bi_optimizer_result_type`
    /// folds into a compare it alone reads (`.i1`).
    B2i,
    /// An unsigned ordering of two 32-bit integers (`uge_pan`, `ult_pan`:
    /// `ICMP.u32` with `BI_RESULT_TYPE_M1`, bifrost_compile.c:3261-3265, the
    /// `_OR` form after `va_lower_isel`).
    Ucmp(Cmpf),
    /// `mov` of one 16-bit lane of a vector (a texel): the copy
    /// `nir_lower_phis_to_scalar` puts at the end of a side for a phi whose
    /// source is a lane of a vector (a phi source takes no swizzle), which
    /// copy propagation cannot fold. `bi_make_vec_to` of one 16-bit
    /// component is `MKVEC.v2i16 a.hN, #0` (bifrost_compile.c:836-848),
    /// which `bi_lower_mkvec_swz` makes `U16_TO_U32 a.hN` (opt.rs;
    /// cflow_148_texbranch.expect.json's `U16_TO_U32 r1, r1^.h0` after each
    /// look-up under an `if`).
    Mov,
    /// `fsin`, `fcos`, `flog2` of a mediump fragment shader's 16-bit lane,
    /// as `bi_lower_bit_size` leaves them (bifrost_nir.c:42-49):
    /// `f2f16(op(f2f32(a)))`, one 16-bit node that `bi_emit_alu`'s 32-bit
    /// expansion stands for at selection (nir_fs.rs `transc_rows`;
    /// docs/notes/mali-transcendentals.md).
    Fsin,
    Fcos,
    Flog2,
    /// `fexp2`: of a 16-bit lane as the three above (gm2_fsmisc_6's
    /// `F16_TO_F32`, `bi_fexp_32`, `FADD.f32 .h0`); of a varying's lane at
    /// fp32, its `f2fmp` gone at the link (`f2f32(f2fmp(a))` is `a`),
    /// paired by its narrowing (gm2_fsmisc_5; nir_fs.rs [`exp2_rows`],
    /// docs/notes/mali-fs-arith.md).
    Fexp2,
    /// `fpow(a, #b)` of a mediump fragment lane and a constant exponent, as
    /// `bi_lower_bit_size` leaves it (`bi_fpow_32`: `bi_flog2_32` of `a`,
    /// then `bi_fexp_32` of `b` scaled by that log; gm2_fsarith_1).
    Fpow,
    /// `ddx` / `ddy` of one 16-bit lane of a mediump fragment shader (the
    /// fine form; `bi_emit_derivative`): `CLPER.i32.subgroup4.xor` of the
    /// lane's neighbour (lane ^ 1, lane ^ 2), `FADD.v2f16 n, a.neg`, then the
    /// sign fix `LSHIFT_XOR.v2i16` by the lane id's axis bit (X: `lane_id <<
    /// 15`; Y: `(lane_id & 2) << 14`). `ddy`'s operand is scaled by the
    /// y-flip `T.x` first (`nir_lower_wpos_ytransform`'s `lower_fddy`;
    /// nir_fs.rs `walk_nodes`). Case 138 deriv, fsarith_138_deriv.expect.json;
    /// docs/notes/mali-derivatives.md.
    Fddx,
    Fddy,
}

impl Alu {
    pub const fn arity(self) -> usize {
        match self {
            Alu::Frcp
            | Alu::F2f16
            | Alu::Fneg
            | Alu::Fabs
            | Alu::Fclamp(_)
            | Alu::Frsq
            | Alu::Ffloor
            | Alu::Fceil
            | Alu::Ftrunc
            | Alu::B2f
            | Alu::F2i
            | Alu::I2f
            | Alu::U2f32
            | Alu::U2u32
            | Alu::F2f32
            | Alu::Iabs
            | Alu::I2i32
            | Alu::F2i32
            | Alu::Inot
            | Alu::U8f32(_)
            | Alu::Ineg
            | Alu::U2f
            | Alu::F2u
            | Alu::Extract16(_)
            | Alu::Mov
            | Alu::B2i
            | Alu::Fsin
            | Alu::Fcos
            | Alu::Flog2
            | Alu::Fexp2
            | Alu::Fddx
            | Alu::Fddy => 1,
            Alu::Ffma
            | Alu::Bcsel
            | Alu::Cubeface1
            | Alu::Cubeface2
            | Alu::CubeSsel
            | Alu::CubeTsel
            | Alu::BitfieldSelect => 3,
            _ => 2,
        }
    }
    /// A texture intrinsic (`cubeface_pan`, `cube_ssel_pan`,
    /// `cube_tsel_pan`): not an ALU operation in NIR, so no constant fold
    /// and no algebraic rule touches it.
    const fn is_intrinsic(self) -> bool {
        matches!(self, Alu::Cubeface1 | Alu::Cubeface2 | Alu::CubeSsel | Alu::CubeTsel)
    }
    /// The first two sources commute (`nir_op_info::algebraic_properties`).
    const fn commutes(self) -> bool {
        match self {
            Alu::Fadd
            | Alu::Fmul
            | Alu::Ffma
            | Alu::Fmax
            | Alu::Fmin
            | Alu::Iadd
            | Alu::Iand
            | Alu::Ior
            | Alu::Imax
            | Alu::Imin
            | Alu::Imul
            | Alu::Ixor => true,
            Alu::Fcmp(c) | Alu::Icmp(c) => matches!(c, Cmpf::Eq | Cmpf::Ne),
            _ => false,
        }
    }
    /// The value is a 32-bit integer (a mask, a converted float, a sum):
    /// no float rule applies to it.
    pub const fn is_int(self) -> bool {
        matches!(
            self,
            Alu::Fcmp(_)
                | Alu::Icmp(_)
                | Alu::F2i
                | Alu::Iadd
                | Alu::U2u32
                | Alu::Ishl
                | Alu::Cubeface2
                | Alu::BitfieldSelect
                | Alu::Iand
                | Alu::Ior
                | Alu::Imax
                | Alu::Imin
                | Alu::Iabs
                | Alu::Ushr
                | Alu::Imul
                | Alu::I2i32
                | Alu::F2i32
                | Alu::Inot
                | Alu::Ishr
                | Alu::Ixor
                | Alu::Isub
                | Alu::Ineg
                | Alu::F2u
                | Alu::Extract16(_)
                | Alu::B2i
                | Alu::Ucmp(_)
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `load_input` of the attribute at this location, or (nir_fs.rs) the
    /// fragment input of the varying at this location or gl_PointCoord's
    /// (`nir_fs::POINT_COORD`); `comps` lanes.
    Input {
        loc: u8,
        comps: u8,
    },
    /// `load_ubo` of the uniform block: the front end's dword of the
    /// uniform's first component, or of a matrix column.
    Uniform {
        dword: u16,
        comps: u8,
    },
    /// The viewport scale or offset (a sysval UBO load, three lanes).
    Sysval(Sysval),
    Alu(Alu),
    /// `lea_buf_pan` of section `out` (0: gl_Position, 1: the generic
    /// varyings, 2: gl_PointSize at the output buffer index plus 4,
    /// pan_nir_lower_vs_outputs.c; NIR's CSE leaves one per section).
    Lea {
        out: u8,
    },
    /// The address of 32-bit word `word` of section `out`: the `iadd` of
    /// the section's address and `4 * word` that the store's split leaves
    /// (bifrost_nir.c:1009-1017). `swapped`: the constant is the second
    /// source, as in the `iadd` pan_nir_lower_vs_outputs.c:89 makes for an
    /// output at a non-zero offset (its first word, which no split adds to;
    /// measured: sel_v_color_uv).
    Addr {
        out: u8,
        word: u8,
        swapped: bool,
    },
    /// The store of `src[0]` to word `word` of section `out`, at
    /// `src[1]`'s address (an [`Kind::Addr`]) or, for word 0, at the
    /// section's. In a highp fragment shader (nir_fs.rs): colour lane
    /// `word`'s store, which the late fusion counts as a use.
    Store {
        out: u8,
        word: u8,
    },
    /// A fragment shader's texture look-up (nir_fs.rs): its four 16-bit
    /// lanes are the texels.
    Tex,
    /// A read of `sampler`'s texture descriptor for `textureSize`
    /// (nir_fs.rs `Fs::tex_size`; `pan_nir_load_va_desc` of
    /// `pan_nir_load_va_tex_size`, pan_nir.h:106-140: a `load_ssbo` of the
    /// descriptor table). `byte` 0: its first eight bytes (`(b, 4, 16,
    /// handle, 0)`, Mesa's `LD_PKA.i64`), two 32-bit lanes, the words, whose
    /// halves hold the null test and the width and height minus one; byte
    /// 28 (`7 * 4`): the depth minus one of a 3D texture, and byte 24 (`6 *
    /// 4`) the layer count minus one of an array, each one 16-bit lane
    /// (`LD_PKA.i16`).
    TexDesc {
        sampler: u8,
        byte: u8,
    },
    /// A fragment shader's read of one element of a uniform array at a
    /// run-time index (nir_fs.rs; `load_ubo(0, index * stride)`, Mesa's
    /// `LD_PKA`): `dword` the front end's first dword of the array, `comps`
    /// the element's components, `src[0]` the 32-bit integer index. Its
    /// lanes are the element's components, 16-bit in a mediump shader (two
    /// a word of the load).
    UniformIdx {
        dword: u16,
        comps: u8,
    },
    /// A vertex shader's 2D or cube look-up (`txl`; a `tex` is one at 0
    /// there, `lower_invalid_implicit_lod`, pan_compiler.c:118-129):
    /// `src[0]` and `src[1]` the coordinate (a `cube`'s the S word with the
    /// face in its top bits and the T word, `build_cube_desc`), `src[2]`
    /// the level's signed 8.8 word when `explicit` (`EXPLICIT`), else level
    /// 0 (`ZERO_LOD`); four fp32 lanes, the texels
    /// (pan_nir_lower_tex.c:802-973). `sampler` is the GL sampler
    /// binding it reads, `index` its descriptor index in the vertex
    /// tables, set after DCE ([`Nir::bound_samplers`]). `medium`: the
    /// sampler is lowp or mediump, so a mediump operation reads the texels
    /// at 16 bits; a node of 16 bits ([`NirNode::bits`]) is the fp16
    /// look-up (`TEX_SINGLE.16`), its lanes two a register in lane order.
    VsTex {
        explicit: bool,
        cube: bool,
        sampler: u8,
        index: u8,
        medium: bool,
    },
    /// `load_pixel_coord` (nir_fs.rs): the pixel position the hardware
    /// preloads in r59 ([`super::ra::POSITION_XY`]), a 16-bit vec2, x in
    /// lane 0 (the low half) and y in lane 1.
    PixelCoord,
    /// A 32-bit word the hardware preloads in register `reg`
    /// (`bi_preload`, `bi_preload_reg` compiler.h): gl_VertexID's r60 and
    /// gl_InstanceID's r61 of an IDVS vertex shader (`load_vertex_id`,
    /// `load_instance_id`, bifrost_compile.c:2039-2051: a `MOV.i32` of the
    /// register, no base added on the malloc-IDVS flow), the primitive
    /// flags' r58 that gl_FrontFacing compares (:2017-2029; nir_fs.rs).
    Preload {
        reg: u8,
    },
    /// gl_FragCoord's z and w (`load_frag_coord_z`/`_w`, not lowered: a
    /// colour that reads one is refused, nir_fs.rs).
    FragCoordZw,
    /// A phi at the join of an `if` (`nir_lower_vars_to_ssa`'s, scalar
    /// after `nir_lower_phis_to_scalar`): `src[0]` the value from the then
    /// side, `src[1]` from the else side, in the join block's predecessor
    /// order (`bi_emit_phi_deferred`).
    Phi,
    /// A fragment shader's `demote_if` (nir_fs.rs): `src[0]` the
    /// comparison's 32-bit mask, the lane discarded where it is set. What
    /// GLSL's `if (c) discard;` becomes (`nir_discard_if`, glsl_to_nir.cpp:
    /// 812-825, under `discard_is_demote`; the GL front end's second
    /// `nir_opt_peephole_select` with `discard_ok`, gl_nir_linker.c:107-111,
    /// folds an `if` around a lone `demote` the same way). No value: a
    /// root for [`dce`], never speculated ([`Nir::allowed`]).
    Discard,
}

/// An `if` of the shader as a structured region (`nir_if`): its condition
/// (a comparison's 32-bit mask), the block before it, the first and last
/// blocks of each side (a side with a nested `if` has several) and the
/// join block after it, where its phis stand. `live` is false once the
/// region was flattened ([`Nir::peephole_select`]) or removed
/// ([`Nir::dead_cf`]). `guard` marks the `if` around a pass of a loop
/// `complex_unroll` unrolled (nir_opt_loop_unroll.c:338): its then side the
/// `break`'s, empty, its else side the rest of the loop
/// ([`Nir::invert_guards`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IfRegion {
    pub cond: Val,
    pub before: u8,
    pub then_first: u8,
    pub then_last: u8,
    pub else_first: u8,
    pub else_last: u8,
    pub after: u8,
    pub live: bool,
    pub guard: bool,
    /// The kept loop ([`LoopRegion`], its index in [`Nir::loops`]) whose
    /// exit the then side's last block jumps to: a `break` ends the side
    /// (`bi_emit_jump`, bifrost_compile.c:85-100), so the side does not
    /// reach the join.
    pub brk: Option<u8>,
    /// The region's place among the regions in the order the walk closed
    /// them (an inner one first), which is the order `emit_if` and
    /// `emit_loop` add their blocks' edges in.
    pub seq: u8,
    /// A guard of a loop whose test and `break` NIR merged into one
    /// terminator (nir_fs.rs `merged_bound`), which
    /// `complex_unroll_single_terminator` unrolls ([`Nir::invert_ladder`]).
    pub single: bool,
    /// Mesa built the `if` on `inot` of a compare (`nir_push_if(b,
    /// nir_inot(b, is_null))`, `pan_nir_load_va_tex_size`, pan_nir.h:111):
    /// `cond` is the inverse compare already (`inot(ieq(a, b))` is `ine(a,
    /// b)`, nir_opt_algebraic.py, what a kept `if` tests: texmore_46's
    /// `ICMP_OR.v2u16.ne.m1` before its `BRANCHZ`); a flattened one's
    /// `bcsel(inot(c), then, else)` is `bcsel(c, else, then)`
    /// (nir_opt_algebraic.py:971), so the peephole flips the compare back
    /// and swaps the sides (texmore_46's `ICMP_OR.v2u16.eq.m1` then
    /// `CSEL.u32.eq r2, r0, 0x0, r2^, 0x0`).
    pub inot: bool,
}

impl IfRegion {
    pub const NONE: IfRegion = IfRegion {
        cond: Val::Undef,
        before: 0,
        then_first: 0,
        then_last: 0,
        else_first: 0,
        else_last: 0,
        after: 0,
        live: false,
        guard: false,
        brk: None,
        seq: 0,
        single: false,
        inot: false,
    };
}

/// A loop Mesa keeps (no limiting terminator, so `nir_opt_loop_unroll`
/// leaves it, nir_opt_loop_unroll.c:1112-1114), as `emit_loop`
/// (bifrost_compile.c:3738-3770) lays it out: the block before it
/// (`pre`), the header (`continue_block`, where the phis of the values the
/// body carries stand), the body's blocks up to the `latch`, which jumps
/// back to the header, and the `exit` (`break_block`), entered by the
/// `break`s alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopRegion {
    pub pre: u8,
    pub header: u8,
    pub latch: u8,
    pub exit: u8,
    /// See [`IfRegion::seq`].
    pub seq: u8,
}

impl LoopRegion {
    pub const NONE: LoopRegion = LoopRegion { pre: 0, header: 0, latch: 0, exit: 0, seq: 0 };
}

impl Kind {
    /// `nir_can_move_instr` for the move options bifrost_nir.c:375-378
    /// gives (`nir_move_load_ubo | nir_move_load_input | ...`).
    const fn movable(self) -> bool {
        matches!(self, Kind::Input { .. } | Kind::Uniform { .. } | Kind::Sysval(_))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NirNode {
    pub kind: Kind,
    pub src: [Val; 3],
    /// The value's bit size (`nir_def::bit_size`): 32, or 16 for `f2f16`
    /// and an `fmax` of two fp16 values.
    pub bits: u8,
    /// Its lanes (`nir_def::num_components`) for an ALU operation: 1, or 2
    /// for a 16-bit pair (`bi_vectorize_filter`'s width).
    pub width: u8,
}

/// What the builder's code can produce, one bit each: the population the
/// tests reference (tests.rs `the_uniform_population_is_referenced`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Prod {
    AttrLane,
    UniformFloat,
    UniformVec2,
    UniformVec3,
    UniformVec4,
    UniformMat4Column,
    /// A load that keeps fewer lanes than its uniform has.
    UniformShrunk,
    Fadd,
    /// An `fadd` with a constant source (`FADD_IMM`).
    FaddConst,
    Fmul,
    /// A front-end `ffma` of a matrix product.
    MatProduct,
    /// A dot product (`v * m`).
    Dot,
    /// The late fusion.
    Fused,
    /// The late fusion with a constant addend.
    FusedConstAddend,
    /// An instruction equal to an earlier one.
    Cse,
    /// `x + 0.0`, `x * 1.0`, `x * 0.0`.
    Identity,
    /// An operation on constants only.
    Folded,
    /// The viewport transform of a constant w (1/w folds).
    ViewportConstW,
    /// The viewport transform of a computed w.
    ViewportW,
    /// Two attribute loads whose first use is one instruction.
    SharedFirstUse,
    /// A varying: the copy of an attribute.
    Varying,
    /// A varying lane computed from an attribute (arithmetic).
    VaryingComputed,
    /// A varying lane that is an attribute's lane at another position
    /// (a swizzle, or a part of the attribute).
    VaryingSwizzle,
    /// A gl_PointSize write: its fp16 store after the position's.
    PointSize,
    /// A product with a constant other than 0.0, 1.0, -1.0 and 2.0 (`FMA
    /// a, c, -0.0`).
    FmulConst,
    /// A product with -1.0: an fneg.
    FmulMinusOne,
    /// A product with 2.0 left after the fusion: `fadd(a, a)`.
    FmulTwo,
    /// A product with a negated factor: `fneg(fmul(a, b))`.
    FnegProduct,
    /// The late fusion of a negated product: `ffma(fneg(a), b, c)`.
    FusedNegated,
    /// A varying stored in part or not at all: the program's fragment
    /// shader reads only those components (the link's compaction).
    VaryingCompacted,
    /// A varying lane that is convergent (a look-up at uniform
    /// coordinates; it reads no attribute): Mesa's link promotes it to flat
    /// and stores `ffma(x, 0.0, x)`, exact ([`build`]).
    VaryingConvergent,
}

impl Prod {
    pub const ALL: [Prod; 31] = [
        Prod::AttrLane,
        Prod::UniformFloat,
        Prod::UniformVec2,
        Prod::UniformVec3,
        Prod::UniformVec4,
        Prod::UniformMat4Column,
        Prod::UniformShrunk,
        Prod::Fadd,
        Prod::FaddConst,
        Prod::Fmul,
        Prod::MatProduct,
        Prod::Dot,
        Prod::Fused,
        Prod::FusedConstAddend,
        Prod::Cse,
        Prod::Identity,
        Prod::Folded,
        Prod::ViewportConstW,
        Prod::ViewportW,
        Prod::SharedFirstUse,
        Prod::Varying,
        Prod::VaryingComputed,
        Prod::VaryingSwizzle,
        Prod::PointSize,
        Prod::FmulConst,
        Prod::FmulMinusOne,
        Prod::FmulTwo,
        Prod::FnegProduct,
        Prod::FusedNegated,
        Prod::VaryingCompacted,
        Prod::VaryingConvergent,
    ];
}

/// A program as Mesa's NIR has it: the nodes (ids stable) and their order.
pub struct Nir<'s> {
    pub nodes: &'s mut [NirNode],
    pub n: usize,
    /// Node ids in program order; `len` of them.
    pub order: &'s mut [u16],
    pub len: usize,
    pub live: &'s mut [bool],
    pub prods: u32,
    /// The walk is in the shader's own code (not the viewport transform).
    user: bool,
    /// The nodes below this id are the shader's own code's.
    user_end: usize,
    /// The generic varyings, in Mesa's layout order, at their offsets in
    /// the (compacted) layout.
    pub varyings: [VaryingAt; MAX_VARYINGS],
    pub nvar: usize,
    /// Each varying's components stored (`varyings`' order): all of them,
    /// or the fragment shader's reads for a program's vertex shader.
    pub masks: [u8; MAX_VARYINGS],
    /// Each node's basic block (`nir_instr::block`); the entry block is 0.
    /// The order lists a block's nodes together, blocks in layout order
    /// (the then side, the else side, then the join; `emit_cf_list`).
    pub block: &'s mut [u8],
    /// The block the walk adds nodes to.
    pub cur: u8,
    pub nblocks: u8,
    /// Each block's immediate dominator (the block an `if`'s sides and join
    /// are entered from): what `nir_opt_cse` may reuse across blocks.
    pub dom: [u8; MAX_BLOCKS],
    pub ifs: [IfRegion; MAX_IFS],
    pub nifs: usize,
    pub loops: [LoopRegion; MAX_LOOPS],
    pub nloops: usize,
    /// The regions closed so far ([`IfRegion::seq`]).
    pub nseq: u8,
    /// A vertex shader's varying lanes that are constants ([`build`]): the
    /// ones Mesa's link propagates into the fragment shader.
    pub consts: super::link::VaryingConsts,
    /// A constant source of the operation being built differs between the
    /// lanes of its IR vector (nir_fs.rs `alu32`: femtovg-shaped
    /// `vec3(1.0, 2.0, 3.0) * u`). NIR's rules on one value (`fmul(a,
    /// 1.0)`, `fmul(a, 0.0)`, `fmul(a, -1.0)`, `fadd(a, 0.0)`) match the
    /// vector constant whole (nir_search.c `match_value`: a constant in a
    /// pattern must equal every lane read), so they do not fire on its
    /// lanes here; [`Nir::alu`] records it per node ([`Nir::nonuniform`]).
    pub(super) nonuniform_const: bool,
    /// Each node built under [`Nir::nonuniform_const`]: its constant
    /// source is one lane of a vector constant, and so are the constants
    /// folded from it (the reassociation nir_fs.rs `alu32` makes).
    pub nonuniform: &'s mut [bool],
    /// Each fp16 uniform lane's uniform (its first dword; `u16::MAX` for
    /// any other node): a mediump shader's components of one uniform are
    /// one `load_ubo` in NIR, however many lane nodes the walk makes of
    /// them (nir_fs.rs; [`Nir::load_read_above`]).
    pub(super) uniform_load: &'s mut [u16],
    /// The vertex walk models Mesa's IR where it numbers the attribute
    /// locations ([`build`]): a lane's fold that only the back end's
    /// scalarisation lets NIR make ([`vs_fmul`]) is not made.
    numbering_walk: bool,
    /// The walk made such a fold ([`vs_fmul`]).
    late_fold: bool,
    /// The nodes whose arithmetic is exact, one flag per node id: those an
    /// invariant output's value reads ([`Nir::mark_exact`], [`build`]); no
    /// late fusion of them ([`fuse`]).
    exact: &'s mut [bool],
    /// The IR value being built is one an invariant output reads
    /// ([`invariant_values`]): the inexact rules of [`Nir::alu`] are not
    /// applied to it.
    exact_build: bool,
    /// The attribute locations Mesa numbers, one bit each
    /// ([`Nir::attribute_record`]); set by [`build`].
    pub counted: u64,
    /// The attribute locations the values of the shader's output lanes
    /// read, every lane as if the fragment shader read it, one bit each;
    /// set by the walk that makes the late folds ([`build`]).
    output_inputs: u64,
    /// The attribute locations the value of each varying lane reads (lane
    /// `c` of location `l` at `4 * l + c`), one bit each; set by the
    /// numbering walk ([`build`]).
    lane_inputs: [u64; 4 * MAX_VARYINGS],
    /// The reads a shader alone is built for when it stores a constant
    /// lane: its lanes less the constant ones ([`walk`]). The numbering
    /// walk stores the same lanes ([`build`]).
    own_reads: Option<([(u8, u8); MAX_VARYINGS], usize)>,
}

const F32_ZERO: u32 = 0;
const F32_NEG_ZERO: u32 = 0x8000_0000;
const F32_ONE: u32 = 0x3f80_0000;
const F32_MINUS_ONE: u32 = 0xbf80_0000;
const F32_TWO: u32 = 0x4000_0000;
const F32_MINUS_TWO: u32 = 0xc000_0000;
const F32_THREE: u32 = 0x4040_0000;
/// `MAX_W_RECIP_ABS` (nir_lower_viewport_transform.c:33): 2^15.
const W_RECIP_MIN: u32 = 0xc700_0000;
const W_RECIP_MAX: u32 = 0x4700_0000;
/// 1.0 / 16.0 and 16.0 * 256.0, `build_sfixed_8_8`'s factors.
pub(super) const LOD_DIV16: u32 = 0x3d80_0000;
pub(super) const LOD_8_8: u32 = 0x4580_0000;
/// 0.5: `build_cube_coord2_face`'s `0.5 / max{x, y, z}` and its addend.
const F32_HALF: u32 = 0x3f00_0000;
/// `BITFIELD_MASK(29)`: the S coordinate's bits of the cube descriptor,
/// the face index above them (`build_cube_desc`).
pub(super) const CUBE_S_MASK: u32 = 0x1fff_ffff;
/// 1.0 in fp16: `nir_lower_point_size`'s minimum (bifrost_nir.c passes
/// 1.0 and no maximum).
const F16_ONE: u32 = 0x3c00;

/// Every store to an invariant output (`invariant`; to a non-invariant one
/// when false) is a copy: an attribute, a uniform or a constant, read
/// directly or through locals stored the same way (no ALU operation, no
/// look-up, no indexed read).
fn output_copies(m: &Module<'_>, invariant: bool) -> bool {
    fn copy(m: &Module<'_>, v: ValueId, depth: u8) -> bool {
        match m.def(v) {
            Some(IrOp::LoadInput { .. } | IrOp::LoadUniform { .. } | IrOp::ConstF32(_)) => true,
            Some(IrOp::LoadLocal { local, index: None, .. }) if depth < 8 => m.instrs.iter().all(|op| match op {
                IrOp::StoreLocal { local: l, index, src, .. } if l == local => {
                    index.is_none() && copy(m, src.val, depth + 1)
                }
                _ => true,
            }),
            _ => false,
        }
    }
    m.instrs.iter().all(|op| match op {
        IrOp::StoreOutput { slot, src }
            if m.outputs.iter().any(|o| o.is_invariant() == invariant && o.slot() == *slot) =>
        {
            copy(m, src.val, 0)
        }
        _ => true,
    })
}

fn is_zero(v: Val) -> bool {
    matches!(v, Val::Const(F32_ZERO | F32_NEG_ZERO))
}

/// A float constant the folding may compute with: finite, and normal or
/// zero (NIR folds subnormals under the shader's denorm mode, which is not
/// transcribed).
pub(super) fn plain(bits: u32) -> bool {
    let e = (bits >> 23) & 0xff;
    e != 0xff && (e != 0 || bits & 0x7f_ffff == 0)
}

/// `nir_opt_constant_folding` (`nir_constant_expressions.py`) of the
/// integer [`Alu`] operations on constant operands `k` whose folded shapes
/// a reference draws byte-equal, the result of `bits` bits: the explicit
/// level's `f2i16` of a finite float in range and its `u2u32`
/// (`build_sfixed_8_8` on a constant, [`Nir::sfixed_8_8`]:
/// `textureCubeLod(a, d, 6.5)`'s 8.8 level `0x680`,
/// deqp_constrnd_lod_const_vs's FAU word), and `ior` (deqp_cfold2_const_other_ior's
/// `!false || false`). The other integer and boolean folds (the compares, `inot`,
/// the 32-bit arithmetic and conversions) are as exact, but the shapes they
/// let through differ from Mesa's after the fold (the select chain of a
/// uniform array read at a constant index, light_amount_2_2 and
/// deqp_cfold2_zero_operand_icmp; a mediump `inot` of a bool,
/// deqp_cfold2_const_call_other_bvec): they keep the refusal. `None`: not
/// such an operation, or a conversion out of range.
fn fold_int_const(op: Alu, bits: u8, k: [u32; 3]) -> Option<u32> {
    let f = f32::from_bits(k[0]);
    match (op, bits) {
        (Alu::F2i, 16) if f > -32769.0 && f < 32768.0 => Some(u32::from(f as i16 as u16)),
        (Alu::U2u32, 32) => Some(k[0] & 0xffff),
        (Alu::Ior, 32) => Some(k[0] | k[1]),
        _ => None,
    }
}

/// `nir_opt_constant_folding` of one lane of the front end's integer and
/// boolean operation `a` (a boolean is NIR's 32-bit 0 / ~0 after
/// `pan_nir_lower_bool_to_bitsize`), and the rules of nir_opt_algebraic.py
/// that a constant or a repeated operand decides: `iadd(a, 0)`,
/// `imul(a, 1)`, `imul(a, 0)`, `iand(a, 0)`, `iand(a, ~0)`, `ior(a, 0)`,
/// `ior(a, ~0)`, `iand(a, a)`, `ior(a, a)`, `ixor(a, a)` (each a
/// commutative match). `None` when nothing folds. The integer arithmetic
/// an unrolled loop's counter leaves folds so (its step, its test, its
/// conversion), and so do the int and bool constants GLSL's front end does
/// not (case 72's `iv.x + iv.y == 3 && bv.x && !bv.y`).
pub(super) fn fold_int(a: AluOp, x: Val, y: Val) -> Option<Val> {
    let ci = |v: Val| match v {
        Val::Const(c) => Some(c),
        _ => None,
    };
    let mask = |t: bool| Val::Const(if t { u32::MAX } else { 0 });
    let k = |c: u32| Some(Val::Const(c));
    let f = f32::from_bits;
    match (a, ci(x), ci(y)) {
        (AluOp::Iadd, Some(p), Some(q)) => k(p.wrapping_add(q)),
        (AluOp::Imul, Some(p), Some(q)) => k(p.wrapping_mul(q)),
        (AluOp::Ineg, Some(p), _) => k(p.wrapping_neg()),
        (AluOp::Ishl, Some(p), Some(q)) => k(p << (q & 31)),
        (AluOp::Ishr, Some(p), Some(q)) => k(((p as i32) >> (q & 31)) as u32),
        (AluOp::Ushr, Some(p), Some(q)) => k(p >> (q & 31)),
        (AluOp::Iand, Some(p), Some(q)) => k(p & q),
        (AluOp::Ior, Some(p), Some(q)) => k(p | q),
        (AluOp::Ixor, Some(p), Some(q)) => k(p ^ q),
        (AluOp::Ilt, Some(p), Some(q)) => Some(mask((p as i32) < (q as i32))),
        (AluOp::Ile, Some(p), Some(q)) => Some(mask((p as i32) <= (q as i32))),
        (AluOp::Ult, Some(p), Some(q)) => Some(mask(p < q)),
        (AluOp::Ule, Some(p), Some(q)) => Some(mask(p <= q)),
        (AluOp::Ieq, Some(p), Some(q)) => Some(mask(p == q)),
        (AluOp::Ine, Some(p), Some(q)) => Some(mask(p != q)),
        (AluOp::I2f, Some(p), _) => k((p as i32 as f32).to_bits()),
        (AluOp::F2i, Some(p), _) => k(f(p) as i32 as u32),
        (AluOp::B2f, Some(p), _) => k(if p != 0 { F32_ONE } else { F32_ZERO }),
        (AluOp::B2i, Some(p), _) => k(u32::from(p != 0)),
        (AluOp::I2b, Some(p), _) => Some(mask(p != 0)),
        (AluOp::F2b, Some(p), _) => Some(mask(f(p) != 0.0)),
        (AluOp::Bnot | AluOp::Inot, Some(p), _) => k(!p),
        /* a float counter's test (`for (float i = 0.0; i < 8.0; ..)`) */
        (AluOp::Flt, Some(p), Some(q)) => Some(mask(f(p) < f(q))),
        (AluOp::Fle, Some(p), Some(q)) => Some(mask(f(p) <= f(q))),
        (AluOp::Feq, Some(p), Some(q)) => Some(mask(f(p) == f(q))),
        (AluOp::Iadd | AluOp::Ior | AluOp::Ixor, Some(0), _) | (AluOp::Imul, Some(1), _) => Some(y),
        (AluOp::Iadd | AluOp::Ior | AluOp::Ixor, _, Some(0)) | (AluOp::Imul, _, Some(1)) => Some(x),
        (AluOp::Iand | AluOp::Imul, Some(0), _) | (AluOp::Iand | AluOp::Imul, _, Some(0)) => k(0),
        (AluOp::Ior, Some(u32::MAX), _) | (AluOp::Ior, _, Some(u32::MAX)) => k(u32::MAX),
        (AluOp::Iand, Some(u32::MAX), _) => Some(y),
        (AluOp::Iand, _, Some(u32::MAX)) => Some(x),
        (AluOp::Iand | AluOp::Ior, _, _) if x == y => Some(x),
        (AluOp::Ixor, _, _) if x == y => k(0),
        _ => None,
    }
}

/// The order `nir_lower_vars_to_ssa` registers the shader's locals in
/// (`register_variable_uses`, nir_lower_vars_to_ssa.c:501-540: the first
/// `load_deref` or `store_deref` of each variable, in block order), which
/// decides where its phis stand: `nir_phi_builder_finish`
/// (nir_phi_builder.c:296) inserts each variable's phis at the start of
/// their block, variable after variable, so a block's phis are in the
/// **reverse** of this order. Measured on Mesa 26.2.1's per-pass NIR of
/// intloop_loop_nested (`NIR_DEBUG=print`) and the final NIR of
/// intloop_loop_exit, builtin_98, builtin_45, intloop_divnest,
/// intloop_contloops and intloop_loop_nested_vs:
///
/// * A local declared with a constant initialiser (`int i = 0`, `float s
///   = 0.0`, `vec4 c = vec4(0.0)`) is stored at the top of `main` by
///   glsl_to_nir (the GLSL IR `constant_initializer`), before any other
///   instruction, so these locals are registered first. Their order is
///   the GLSL IR's declaration list: each scope's declarations at the
///   scope's head in reverse declaration order (ast_to_hir pushes a
///   declaration at the head of its block), the scopes in program order
///   (main's body, then each nested body as it is entered). loop_nested's
///   nine counters and three vectors come out `i4, c, i3, b, i2, a, i1,
///   res, coords` then the inner `j`s loop by loop.
/// * Every other local is registered at its first reference in program
///   order.
///
/// The index of each local in that order (`u16::MAX` for one never
/// referenced). A loop header's or a join's phis are the referenced
/// locals' in descending index ([`super::nir_fs`] `walk_kept_loop`).
pub fn registration_order(m: &Module<'_>) -> [u16; MAX_LOCALS] {
    /* a constant expression: a constant, or an operation of constant
     * expressions (builtin_98's `int last = -1`, the negation of `1`);
     * ast_to_hir folds such an initialiser into the declaration's
     * `constant_initializer`, as it does a literal */
    fn const_expr(m: &Module<'_>, v: ValueId, depth: u32) -> bool {
        match m.def(v) {
            Some(IrOp::ConstF32(_) | IrOp::ConstI32(_) | IrOp::ConstBool(_)) => true,
            Some(&IrOp::Alu(a, s)) if depth < 16 => {
                s[..super::nir_fs::ir_arity(a)].iter().all(|x| const_expr(m, x.val, depth + 1))
            }
            _ => false,
        }
    }
    /* per local: the first reference's position in a pre-order walk, the
     * scope (a body's first node) of its first store, whether that store
     * is of a constant */
    let mut first_ref = [u32::MAX; MAX_LOCALS];
    let mut scope = [0u32; MAX_LOCALS];
    let mut konst = [false; MAX_LOCALS];
    /* the scopes in the order they are entered */
    let mut scopes = [u32::MAX; MAX_LOCALS];
    let mut nscopes = 0usize;
    struct Walk<'m, 'a> {
        m: &'m Module<'a>,
        pos: u32,
    }
    fn enter(scopes: &mut [u32; MAX_LOCALS], nscopes: &mut usize, r: ir::Range) -> u32 {
        if let Some(i) = scopes[..*nscopes].iter().position(|&s| s == r.first) {
            return i as u32;
        }
        if *nscopes < scopes.len() {
            scopes[*nscopes] = r.first;
            *nscopes += 1;
        }
        (*nscopes - 1) as u32
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        w: &mut Walk<'_, '_>,
        r: ir::Range,
        first_ref: &mut [u32; MAX_LOCALS],
        scope: &mut [u32; MAX_LOCALS],
        konst: &mut [bool; MAX_LOCALS],
        scopes: &mut [u32; MAX_LOCALS],
        nscopes: &mut usize,
    ) {
        let s = enter(scopes, nscopes, r);
        for n in w.m.nodes_of(r) {
            match *n {
                IrNode::Instr(id) => {
                    w.pos += 1;
                    let local = match w.m.instr(id) {
                        Some(&IrOp::StoreLocal { local, src, .. }) => {
                            let l = local.0 as usize;
                            if l < MAX_LOCALS && first_ref[l] == u32::MAX {
                                scope[l] = s;
                                konst[l] = const_expr(w.m, src.val, 0);
                            }
                            Some(l)
                        }
                        Some(&IrOp::LoadLocal { local, .. }) => Some(local.0 as usize),
                        _ => None,
                    };
                    if let Some(l) = local.filter(|&l| l < MAX_LOCALS) {
                        if first_ref[l] == u32::MAX {
                            first_ref[l] = w.pos;
                        }
                    }
                }
                IrNode::If { then_, else_, .. } => {
                    walk(w, then_, first_ref, scope, konst, scopes, nscopes);
                    walk(w, else_, first_ref, scope, konst, scopes, nscopes);
                }
                IrNode::Loop { body } => walk(w, body, first_ref, scope, konst, scopes, nscopes),
                IrNode::Break | IrNode::Continue | IrNode::Return => {}
            }
        }
    }
    let mut w = Walk { m, pos: 0 };
    walk(&mut w, m.body, &mut first_ref, &mut scope, &mut konst, &mut scopes, &mut nscopes);
    let n = m.locals.len().min(MAX_LOCALS);
    /* the constant-initialised locals: by scope, each scope's in reverse
     * declaration order (the declaration order is the front end's local
     * order, lower.rs `declare_var`) */
    let mut key = [(u32::MAX, u32::MAX); MAX_LOCALS];
    for (l, k) in key.iter_mut().enumerate().take(n) {
        if first_ref[l] == u32::MAX {
            continue;
        }
        *k = if konst[l] { (scope[l], u32::MAX - l as u32) } else { (u32::MAX - 1, first_ref[l]) };
    }
    let mut out = [u16::MAX; MAX_LOCALS];
    let mut rank = 0u16;
    loop {
        let next = (0..n).filter(|&l| out[l] == u16::MAX && key[l] != (u32::MAX, u32::MAX)).min_by_key(|&l| key[l]);
        let Some(l) = next else { break };
        out[l] = rank;
        rank += 1;
    }
    out
}

/// The uniform a front-end dword falls in, and its component count in
/// Mesa's packed block (one dword per component, `PackedDriverUniformStorage`):
/// the types the back end takes.
pub fn uniform_of<'m>(m: &Module<'m>, dword: u16) -> Result<(&'m ir::UniformDecl<'m>, u16), MaliError> {
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
        /* a float, vec2, vec3 or vec4 array, read element by element
         * ([`array_element`]; gl_test cases 149 and 150's shape,
         * uniarray_vsconst, deqp_uniarray_*_const_vs) */
        if array_element(u).is_some() {
            return Ok((u, u.dwords));
        }
        return Err(MaliError::UniformArray);
    }
    let comps = match u.gl_type as u32 {
        GL_FLOAT => 1,
        GL_FLOAT_VEC2 => 2,
        GL_FLOAT_VEC3 => 3,
        GL_FLOAT_VEC4 => 4,
        GL_FLOAT_MAT4 => 16,
        /* a mat2 or mat3: Mesa's packed columns, no padding (4 and 9 words,
         * `glsl_count_dword_slots`; vsmat2_highp, vsmat3_highp), each
         * column its own load ([`matrix_cols`]) */
        GL_FLOAT_MAT2 => 4,
        GL_FLOAT_MAT3 => 9,
        /* a bvec's lanes are 32-bit words, as a bool's (vsbvec3) */
        GL_BOOL_VEC2 => 2,
        GL_BOOL_VEC3 => 3,
        GL_BOOL_VEC4 => 4,
        /* an int or a bool is one 32-bit word a component, raw bits
         * (docs/notes/mali-int-uniforms.md: Mesa's packed block, a word
         * per component; intuni_ishr's `ivec2 v` at `u2.w0`, `u2.w1`) */
        GL_INT | GL_UNSIGNED_INT | GL_BOOL => 1,
        GL_INT_VEC2 | GL_UNSIGNED_INT_VEC2 => 2,
        GL_INT_VEC3 | GL_UNSIGNED_INT_VEC3 => 3,
        /* intuni_intrt's `uvec4 u_v`, `.xyz` at `u2.w0` to `u3.w0` */
        GL_INT_VEC4 | GL_UNSIGNED_INT_VEC4 => 4,
        _ => return Err(MaliError::UniformType),
    };
    Ok((u, comps))
}

/// The element of a float, vec2, vec3 or vec4 array uniform `u`: (its
/// front-end dword stride, its components). The front end strides an
/// element to the alignment of its vector (reflect.rs `type_layout`: a vec3
/// takes 4 dwords); Mesa's block does not, and a mediump array is fp16
/// there, each element from a word of its own (docs/notes/mali-deqp-uniarray.md
/// sections 3 and 7: `u_arr[k]` of a `mediump float` at byte 4k, of a vec2
/// at 4k, of a vec3 and a vec4 at 8k; a struct member's array fp32, one word
/// a component, nothing aligned). None: not an array of those.
pub(super) fn array_element(u: &ir::UniformDecl<'_>) -> Option<(u16, u16)> {
    if u.array_len <= 1 {
        return None;
    }
    let (stride, comps) = match u.gl_type as u32 {
        GL_FLOAT => (1, 1),
        GL_FLOAT_VEC2 => (2, 2),
        GL_FLOAT_VEC3 => (4, 3),
        GL_FLOAT_VEC4 => (4, 4),
        _ => return None,
    };
    let n = u16::from(u.array_len);
    (u.dwords == stride * n || u.dwords == stride * (n - 1) + comps).then_some((stride, comps))
}

/// The load Mesa makes of front-end dword `dword` of array `u`
/// ([`array_element`]): its element's first dword and its components.
pub(super) fn element_load(u: &ir::UniformDecl<'_>, dword: u16) -> Option<(u16, u16)> {
    let (stride, comps) = array_element(u)?;
    Some((u.dword_off + (dword - u.dword_off) / stride * stride, comps))
}

/// A square float matrix's (columns, rows, front-end column stride): the
/// front end strides a column to the alignment of a vector of its rows
/// (reflect.rs `type_layout`: a mat2 column 2 dwords, a mat3's 4), Mesa's
/// packed block does not ([`select::UboLayout`] lays one row a column).
pub(super) fn matrix_cols(gl_type: u16) -> Option<(u16, u16, u16)> {
    match gl_type as u32 {
        GL_FLOAT_MAT2 => Some((2, 2, 2)),
        GL_FLOAT_MAT3 => Some((3, 3, 4)),
        GL_FLOAT_MAT4 => Some((4, 4, 4)),
        _ => None,
    }
}

/// Uniform `u` of a vertex shader is an fp16 uniform: a float, vec2, vec3,
/// vec4 or mat4 that is no array, declared mediump or lowp (`fp16` names
/// those). `lower_variables_visitor::visit(ir_variable)`
/// (lower_precision.cpp:1023-1060) retypes it to 16 bits when the stage
/// has `fp16_const_buffers`, which panfrost sets for every stage from v6
/// (pan_screen.c:588); the block holds it as halves, two a word
/// ([`select::UboLayout::vs`]). A struct's member stays fp32 there (the
/// base type must be a float; mod.rs gives `fp16` plain uniforms only).
/// Another type or an array of that precision has no reference: refused
/// (mod.rs, [`MaliError::UniformPrecision`]).
pub(super) fn fp16_uniform(u: &ir::UniformDecl<'_>, fp16: &dyn Fn(&str) -> bool) -> bool {
    let float = matches!(
        u.gl_type as u32,
        GL_FLOAT | GL_FLOAT_VEC2 | GL_FLOAT_VEC3 | GL_FLOAT_VEC4 | GL_FLOAT_MAT2 | GL_FLOAT_MAT3 | GL_FLOAT_MAT4
    );
    /* a float, vec2, vec3 or vec4 array too: each element fp16 from a word
     * of its own (deqp_uniarray_*_const_vs, [`array_element`]) */
    float && (u.array_len <= 1 || array_element(u).is_some()) && fp16(u.name)
}

/// Every read of IR value `v` is the index of a uniform array read
/// (`LoadUniformIdx`): an int uniform the back end takes as raw bits for
/// the element's byte offset only (uniarray_highp, uniarray_vs). Any other
/// integer arithmetic has no reference ([`MaliError::UniformType`] at the
/// uniform, as before).
pub(super) fn index_only(m: &Module<'_>, v: ValueId) -> bool {
    let mut uses = 0;
    let mut other = false;
    for op in m.instrs.iter() {
        match op {
            IrOp::LoadUniformIdx { index, .. } if index.val == v => uses += 1,
            _ => op.for_each_src(|s| other |= s.val == v),
        }
    }
    uses > 0 && !other
}

/// Every read of IR value `v` is its conversion to a float (`float(s.c)`,
/// `i2f32`): an `int` uniform's word converted in place
/// (deqp_uniarray_struct_member_vs's `S32_TO_F32 r2, u7.w0` of the struct's
/// int member). Any other read keeps [`MaliError::UniformType`].
pub(super) fn int_float_only(m: &Module<'_>, v: ValueId) -> bool {
    let mut uses = 0;
    let mut other = false;
    for op in m.instrs.iter() {
        match op {
            IrOp::Alu(AluOp::I2f, s) if s[0].val == v => uses += 1,
            _ => op.for_each_src(|s| other |= s.val == v),
        }
    }
    uses > 0 && !other
}

/// Every read of IR value `v` is an operand of a signed integer compare
/// (`<`, `<=`, `==`, `!=`) that is a loop's head test (`if (c) {} else
/// break;`, `Lower::break_unless`): an `int` uniform a vertex loop's bound
/// reads as it is, its word an FAU word the compare reads in place
/// (gm2_cf_loop_uni_vs's `ICMP_OR.s32.ge.m1 r5, r4, u11.w0, 0x0`, glmark2
/// loop's `i < VertexLoops`; docs/notes/mali-int-uniforms.md section 2: an
/// int is one 32-bit word a component). Any other read (arithmetic, a
/// compare an `if` or a select reads) has no reference and keeps
/// [`MaliError::UniformType`] at the uniform.
pub(super) fn int_compare_only(m: &Module<'_>, v: ValueId) -> bool {
    /* the head tests of the loops of `nodes`, nested ones too */
    fn head(m: &Module<'_>, nodes: &[IrNode], c: ValueId) -> bool {
        nodes.iter().any(|n| match *n {
            IrNode::Loop { body } => {
                let first = m.nodes_of(body).iter().find(|n| !matches!(n, IrNode::Instr(_)));
                matches!(first, Some(&IrNode::If { cond, then_, else_ })
                    if cond.val == c && then_.is_empty() && m.nodes_of(else_) == [IrNode::Break])
                    || head(m, m.nodes_of(body), c)
            }
            IrNode::If { then_, else_, .. } => head(m, m.nodes_of(then_), c) || head(m, m.nodes_of(else_), c),
            _ => false,
        })
    }
    let mut uses = 0;
    let mut other = false;
    for (i, op) in m.instrs.iter().enumerate() {
        match op {
            IrOp::Alu(AluOp::Ilt | AluOp::Ile | AluOp::Ieq | AluOp::Ine, s)
                if s[..2].iter().any(|x| x.val == v) && head(m, m.nodes_of(m.body), ValueId(i as u32)) =>
            {
                uses += 1
            }
            _ => op.for_each_src(|s| other |= s.val == v),
        }
    }
    uses > 0 && !other
}

/// An fp32 read of a float array's element at an int uniform's index (the
/// vertex stage and a highp fragment shader; uniarray_vs, uniarray_highp):
/// Mesa's 32-bit `load_ubo(0, base + (index << 2))`, one dword an element
/// (docs/notes/mali-uniform-arrays.md), as a [`Kind::UniformIdx`] node of
/// the graph `g`, its one lane the element. Any other array or index form
/// has no reference ([`MaliError::UniformIndexed`]).
pub(super) fn uniform_idx(
    g: &mut Nir<'_>,
    m: &Module<'_>,
    base_dw: u16,
    comps: u8,
    idx: Val,
) -> Result<u16, MaliError> {
    let (u, _) = super::nir_fs::uniform_extent(m, base_dw)?;
    let int_word = match idx {
        Val::Lane(n, 0) => match g.nodes[n as usize].kind {
            Kind::Uniform { dword, .. } => uniform_of(m, dword).is_ok_and(|(i, _)| i.gl_type as u32 == GL_INT),
            /* a vec4 array's element at an int sum (builtin_45's `cols[idx
             * + i]` in a kept loop: `IADD.s32 r5, u2.w0, r4`, then the
             * shift and `LD_PKA.i128`) */
            Kind::Alu(Alu::Iadd) => u.gl_type as u32 == GL_FLOAT_VEC4 && g.nodes[n as usize].bits == 32,
            _ => false,
        },
        _ => false,
    };
    /* a float array's element (one dword), or a vec4 array's (four: the
     * element's 16 bytes, which the selection loads from the lowest lane
     * read to the highest, `select::vec4_fetch`) */
    let shape = match u.gl_type as u32 {
        GL_FLOAT => comps == 1,
        GL_FLOAT_VEC4 => comps == 4,
        _ => false,
    };
    if !shape || u.array_len <= 1 || u.dword_off != base_dw || !int_word {
        return Err(MaliError::UniformIndexed);
    }
    let src = [idx, Val::Undef, Val::Undef];
    g.push_node(NirNode { kind: Kind::UniformIdx { dword: base_dw, comps }, src, bits: 32, width: 1 })
}

/// The uniform at front-end dword `dword` is read by graph `g` only as the
/// int index of fp16 elements at a run-time index ([`uniform_idx16`]): each
/// live reader of its 32-bit node is such a read.
pub(super) fn index16_only(g: &Nir<'_>, dword: u16) -> bool {
    let mut any = false;
    for &id in &g.order[..g.len] {
        let nd = g.nodes[id as usize];
        if !g.live[id as usize] || !matches!(nd.kind, Kind::Uniform { dword: d, .. } if d == dword) {
            continue;
        }
        for (u, _) in g.uses(id).filter(|&(u, _)| g.live[u as usize]) {
            if !matches!(g.nodes[u as usize], NirNode { kind: Kind::UniformIdx { .. }, bits: 16, .. }) {
                return false;
            }
            any = true;
        }
    }
    any
}

/// A vertex shader's read of a mediump vec2, vec3 or vec4 array's element
/// at an int uniform's index ([`fp16_uniform`]: the block holds it as fp16
/// halves, a word an element of two lanes, two of three or four): Mesa's
/// 16-bit `load_ubo(0, base + (index << 2 or 3))`, `LSHIFT_OR.i32` by the
/// byte table's 2 or 3, `IADD_IMM.i32` of the base, `LD_PKA.i32`, `.i48`
/// or `.i64` (deqp_uniarray_dyn_vec{2,3,4}_vs, the fragment walk's
/// `ubo_offset` and `ubo_fetch` in nir_fs.rs), as a 16-bit
/// [`Kind::UniformIdx`] node whose lanes are the element's, two a register
/// ([`vs_is_word16`]). Any other array or index form has no reference
/// ([`MaliError::UniformIndexed`]).
pub(super) fn uniform_idx16(
    g: &mut Nir<'_>,
    m: &Module<'_>,
    base_dw: u16,
    comps: u8,
    idx: Val,
) -> Result<u16, MaliError> {
    let (u, _) = super::nir_fs::uniform_extent(m, base_dw)?;
    let int_word = matches!(idx, Val::Lane(n, 0) if matches!(g.nodes[n as usize],
        NirNode { kind: Kind::Uniform { dword, .. }, bits: 32, .. }
            if uniform_of(m, dword).is_ok_and(|(i, _)| i.gl_type as u32 == GL_INT)));
    let shape = match u.gl_type as u32 {
        GL_FLOAT_VEC2 => comps == 2,
        GL_FLOAT_VEC3 => comps == 3,
        GL_FLOAT_VEC4 => comps == 4,
        _ => false,
    };
    if !shape || u.array_len <= 1 || u.dword_off != base_dw || !int_word {
        return Err(MaliError::UniformIndexed);
    }
    let src = [idx, Val::Undef, Val::Undef];
    g.push_node(NirNode { kind: Kind::UniformIdx { dword: base_dw, comps }, src, bits: 16, width: 1 })
}

impl<'s> Nir<'s> {
    pub(super) fn new(scratch: &Arena<'s>) -> Result<Nir<'s>, MaliError> {
        let oom = |_| MaliError::OutOfScratch;
        let none = NirNode { kind: Kind::Alu(Alu::Fadd), src: [Val::Undef; 3], bits: 32, width: 1 };
        Ok(Nir {
            nodes: scratch.alloc_filled(MAX_NODES, none).map_err(oom)?,
            n: 0,
            order: scratch.alloc_filled(MAX_NODES, 0u16).map_err(oom)?,
            len: 0,
            live: scratch.alloc_filled(MAX_NODES, false).map_err(oom)?,
            prods: 0,
            user: true,
            user_end: 0,
            varyings: [VaryingAt { location: 0, comps: 0, offset: 0 }; MAX_VARYINGS],
            nvar: 0,
            masks: [0; MAX_VARYINGS],
            block: scratch.alloc_filled(MAX_NODES, 0u8).map_err(oom)?,
            cur: 0,
            nblocks: 1,
            dom: [0; MAX_BLOCKS],
            ifs: [IfRegion::NONE; MAX_IFS],
            nifs: 0,
            loops: [LoopRegion::NONE; MAX_LOOPS],
            nloops: 0,
            nseq: 0,
            consts: super::link::VaryingConsts::NONE,
            nonuniform_const: false,
            nonuniform: scratch.alloc_filled(MAX_NODES, false).map_err(oom)?,
            uniform_load: scratch.alloc_filled(MAX_NODES, u16::MAX).map_err(oom)?,
            numbering_walk: false,
            late_fold: false,
            exact: scratch.alloc_filled(MAX_NODES, false).map_err(oom)?,
            exact_build: false,
            counted: 0,
            output_inputs: 0,
            lane_inputs: [0; 4 * MAX_VARYINGS],
            own_reads: None,
        })
    }

    /// A new block dominated by `dom` (entered from it alone, or through
    /// an `if` it ends with); its index, after every block so far.
    pub(super) fn new_block(&mut self, dom: u8) -> Result<u8, MaliError> {
        if self.nblocks as usize >= MAX_BLOCKS {
            return Err(MaliError::BlockTableFull);
        }
        let b = self.nblocks;
        self.dom[b as usize] = dom;
        self.nblocks += 1;
        Ok(b)
    }

    /// Block `a` dominates block `b`: it is `b` or on `b`'s dominator chain.
    pub fn dominates(&self, a: u8, b: u8) -> bool {
        let mut x = b;
        loop {
            if x == a {
                return true;
            }
            if x == 0 {
                return false;
            }
            x = self.dom[x as usize];
        }
    }

    /// The nearest block dominating both `a` and `b`.
    pub fn lca(&self, a: u8, b: u8) -> u8 {
        let mut x = a;
        while !self.dominates(x, b) {
            x = self.dom[x as usize];
        }
        x
    }

    /// The `if` regions still standing.
    pub fn live_ifs(&self) -> impl Iterator<Item = &IfRegion> + '_ {
        self.ifs[..self.nifs].iter().filter(|r| r.live)
    }

    /// Block `b` is inside region `r`: one of its sides' blocks.
    fn inside(&self, r: &IfRegion, b: u8) -> bool {
        let mut x = b;
        loop {
            if x == r.then_first || x == r.else_first {
                return true;
            }
            if x == 0 || x == r.before {
                return false;
            }
            x = self.dom[x as usize];
        }
    }

    /// Block `from`'s nodes, dominator references and region references
    /// become block `into`'s (the blocks merge, `stitch_blocks`).
    fn merge_block(&mut self, from: u8, into: u8) {
        for k in 0..self.n {
            if self.block[k] == from {
                self.block[k] = into;
            }
        }
        for d in self.dom.iter_mut() {
            if *d == from {
                *d = into;
            }
        }
        for r in self.ifs[..self.nifs].iter_mut() {
            for b in
                [&mut r.before, &mut r.then_first, &mut r.then_last, &mut r.else_first, &mut r.else_last, &mut r.after]
            {
                if *b == from {
                    *b = into;
                }
            }
        }
        for r in self.loops[..self.nloops].iter_mut() {
            for b in [&mut r.pre, &mut r.header, &mut r.latch, &mut r.exit] {
                if *b == from {
                    *b = into;
                }
            }
        }
    }

    /// Region `r`'s blocks hold a jump out of it: its then side ends in a
    /// `break`, or a side holds an `if` whose does. NIR keeps such an `if`
    /// (`nir_opt_dead_cf` keeps a block that ends in a jump; the jump is no
    /// instruction `nir_opt_peephole_select` may move, :255).
    fn jumps(&self, r: &IfRegion) -> bool {
        r.brk.is_some() || self.live_ifs().any(|x| x.brk.is_some() && self.inside(r, x.before))
    }

    /// The `demote_if` nodes of the program, in order (nir_fs.rs).
    pub fn discards(&self) -> impl Iterator<Item = u16> + '_ {
        self.order[..self.len].iter().copied().filter(|&id| self.nodes[id as usize].kind == Kind::Discard)
    }

    /// `nir_opt_dead_cf` for an `if` whose sides hold no live node and whose
    /// phis are all dead (after [`dce`]): the region goes, its blocks
    /// become the block before it. The condition's nodes are then dead
    /// too (the next [`dce`]).
    pub fn dead_cf(&mut self) {
        for i in 0..self.nifs {
            let r = self.ifs[i];
            if !r.live || self.jumps(&r) {
                continue;
            }
            let dead = (0..self.len).all(|k| {
                let id = self.order[k];
                let b = self.block[id as usize];
                !(self.inside(&r, b) || b == r.after && self.nodes[id as usize].kind == Kind::Phi)
            });
            if !dead {
                continue;
            }
            self.ifs[i].live = false;
            for b in [r.then_first, r.then_last, r.else_first, r.else_last, r.after] {
                if b != r.before {
                    self.merge_block(b, r.before);
                }
            }
        }
    }

    /// `block_check_for_allowed_instrs` (nir_opt_peephole_select.c:60-325)
    /// with a non-zero limit (so ALU is allowed), expensive ALU allowed,
    /// discards not, over the nodes of block `b`: None when an
    /// instruction may not run speculatively, else the count of the ALU
    /// operations that are neither move-like (`fneg`, `fabs`, `vec2`) nor
    /// `fsat` nor an `fmul` only an `fadd` reads (:247-306). A texture
    /// look-up refuses the block (:319). A uniform read is allowed in the
    /// GL front end's call (`uniforms`: a `load_deref` of a uniform with no
    /// indirect, :110-124) and refused in bifrost's (a `load_ubo` by then,
    /// whose `nir_instr_can_speculate` is false without
    /// `ACCESS_CAN_SPECULATE`, :140); there a uniform lane the block alone
    /// reads is such a load, where a lane a dominating block reads too was
    /// one load there (`nir_opt_cse`). A varying is never a load in a side:
    /// Mesa's GL path loads every fragment input at the shader's start, so
    /// a side reads a value. Measured: fvgtest_cover's scissor `if` (29
    /// operations, `frag[0..2]` and `frag[8]` read in its sides alone)
    /// keeps its branches (tools/mali/testdata/fvgtest_cover.expect.json);
    /// an `if` of twelve operations over varyings first read in its sides
    /// is a `CSEL` (pan_shc-out/s5_peep_big.json), and so is one of one
    /// operation over uniforms read in its side (s5_peep_small's sibling
    /// in pan_shc-out, the first `if` of s5_if_nested).
    fn allowed(&self, b: u8, uniforms: bool) -> Option<u32> {
        let mut count = 0u32;
        for k in 0..self.len {
            let id = self.order[k];
            if self.block[id as usize] != b {
                continue;
            }
            let nd = self.nodes[id as usize];
            match nd.kind {
                Kind::Alu(op) => {
                    for s in nd.src.iter().take(op.arity()) {
                        let Val::Lane(u, c) = *s else { continue };
                        let (ub, uk) = (self.block[u as usize], self.nodes[u as usize].kind);
                        if uk == Kind::Tex && ub == b {
                            return None;
                        }
                        if !uniforms
                            && matches!(uk, Kind::Uniform { .. })
                            && !self.lane_read_above(u, c, b)
                            && !self.load_read_above(u, b)
                        {
                            return None;
                        }
                    }
                    let free = match op {
                        Alu::Fneg | Alu::Fabs | Alu::Vec2 | Alu::Fclamp(super::bir::clamp::ZERO_ONE) => true,
                        Alu::Fmul => self.only_used_by_fadd_or_ffma(id),
                        _ => false,
                    };
                    if !free {
                        count += 1;
                    }
                }
                /* `load_pixel_coord` is allowed and not counted (:170); an
                 * input's node stands where it was first read, its load at
                 * the start (the module text of `allowed`); a uniform's node
                 * is its lanes' FAU words, checked at their reads */
                Kind::PixelCoord | Kind::Input { .. } | Kind::Uniform { .. } => {}
                /* `load_front_face` and `load_instance_id` are allowed too
                 * (:166, :182); `load_vertex_id` is not in the list */
                Kind::Preload { reg } if reg != super::ra::VERTEX_ID => {}
                Kind::Preload { .. } => return None,
                Kind::Tex
                | Kind::TexDesc { .. }
                | Kind::UniformIdx { .. }
                | Kind::VsTex { .. }
                | Kind::FragCoordZw
                | Kind::Phi => return None,
                /* a `demote_if` (:227-232, `discard_ok` false in both calls
                 * that count ALU; the GL front end's third call,
                 * gl_nir_linker.c:107-111, is for a side of nothing but
                 * discards, which nir_fs.rs refuses by name) */
                Kind::Discard => return None,
                Kind::Sysval(_) | Kind::Lea { .. } | Kind::Addr { .. } | Kind::Store { .. } => return None,
            }
        }
        Some(count)
    }

    /// Lane `c` of load `u` is read by a node of a block that strictly
    /// dominates `b` (so NIR's load of it stood there).
    fn lane_read_above(&self, u: u16, c: u8, b: u8) -> bool {
        (0..self.len).any(|k| {
            let id = self.order[k];
            let nb = self.block[id as usize];
            let nd = self.nodes[id as usize];
            let arity = match nd.kind {
                Kind::Alu(op) => op.arity(),
                Kind::Phi => 2,
                Kind::Store { .. } | Kind::Discard => 1,
                _ => 0,
            };
            nb != b && self.dominates(nb, b) && nd.src[..arity].contains(&Val::Lane(u, c))
        })
    }

    /// A lane of the fp16 uniform `u` is a lane of ([`Nir::uniform_load`])
    /// is read by a node of a block that strictly dominates `b`: NIR's one
    /// `load_ubo` of the uniform stands there (cflow_71_deadloop: `u_k.x`
    /// read before the loop, `u_k.y` in its passes alone, every guard a
    /// select).
    fn load_read_above(&self, u: u16, b: u8) -> bool {
        let l = self.uniform_load[u as usize];
        l != u16::MAX
            && (0..self.len).any(|k| {
                let id = self.order[k];
                let nb = self.block[id as usize];
                let nd = self.nodes[id as usize];
                nb != b
                    && self.dominates(nb, b)
                    && nd.src[..nd.arity()]
                        .iter()
                        .any(|v| matches!(*v, Val::Lane(x, _) | Val::Pair(x, _) if self.uniform_load[x as usize] == l))
            })
    }

    /// `is_only_used_by_fadd` for the count: an `fadd` user, or the
    /// `ffma` the front end makes of a matrix product's `fadd(fmul)` (NIR
    /// holds that as the `fmul` and `fadd` the fusion joins later).
    fn only_used_by_fadd_or_ffma(&self, id: u16) -> bool {
        self.uses(id).all(|(u, _)| match self.nodes[u as usize].kind {
            Kind::Alu(Alu::Fadd | Alu::Ffma) => true,
            Kind::Alu(Alu::Fneg) => self.only_used_by_fadd_or_ffma(u),
            _ => false,
        })
    }

    /// `nir_opt_peephole_select` (nir_opt_peephole_select.c:504-601), to
    /// a fixed point as each optimisation loop runs it: an `if` whose sides
    /// are one block each (:536-538), whose instructions may all run
    /// speculatively ([`Nir::allowed`] with `uniforms`) and whose counted
    /// operations number at most `limit` (:543-549) loses its branches:
    /// the sides' nodes join the block before (where they already stand in
    /// the order, :561-573), each phi becomes `bcsel(cond, then, else)`
    /// (:575-597) and the join block merges too. Mesa runs it twice on the
    /// way to BIR: the GL front end's loop with [`PEEPHOLE_LIMIT_GL`] and
    /// uniform reads allowed, then bifrost's with [`PEEPHOLE_LIMIT`] and
    /// the uniforms' `load_ubo`s refused (docs/notes/mali-lowering-map.md,
    /// gap 6). `nir_opt_collapse_if` (:409-502, nested `if`s with empty
    /// else sides) is not transcribed: no femtovg program nests an `if`.
    /// `inots` are the guards the front end's `nir_opt_if` inverted before
    /// its call ([`Nir::inverted_guards`]): each one's `inot` stands in the
    /// block before it, one more counted operation of the side that holds
    /// it (:286-306, an `inot` is no move).
    pub fn peephole_select(&mut self, limit: u32, uniforms: bool, inots: &[bool; MAX_IFS]) {
        let mut progress = true;
        while progress {
            progress = false;
            for i in 0..self.nifs {
                let r = self.ifs[i];
                if !r.live || r.then_first != r.then_last || r.else_first != r.else_last || r.brk.is_some() {
                    continue;
                }
                let (Some(a), Some(b)) = (self.allowed(r.then_first, uniforms), self.allowed(r.else_first, uniforms))
                else {
                    continue;
                };
                let held = (0..self.nifs)
                    .filter(|&j| inots[j] && [r.then_first, r.else_first].contains(&self.ifs[j].before))
                    .count() as u32;
                if a + b + held > limit {
                    continue;
                }
                /* an `if` on `inot` of a compare ([`IfRegion::inot`]):
                 * `bcsel(inot(c), then, else)` is `bcsel(c, else, then)`,
                 * the compare the one the `inot` was of */
                if r.inot {
                    let Val::Lane(c, _) = r.cond else { continue };
                    let flipped = match self.nodes[c as usize].kind {
                        Kind::Alu(Alu::Icmp(Cmpf::Ne)) => Kind::Alu(Alu::Icmp(Cmpf::Eq)),
                        Kind::Alu(Alu::Icmp(Cmpf::Eq)) => Kind::Alu(Alu::Icmp(Cmpf::Ne)),
                        _ => continue,
                    };
                    self.nodes[c as usize].kind = flipped;
                }
                for k in 0..self.len {
                    let id = self.order[k] as usize;
                    if self.block[id] == r.after && self.nodes[id].kind == Kind::Phi {
                        let nd = self.nodes[id];
                        let (t, e) = if r.inot { (nd.src[1], nd.src[0]) } else { (nd.src[0], nd.src[1]) };
                        self.nodes[id] = NirNode { kind: Kind::Alu(Alu::Bcsel), src: [r.cond, t, e], ..nd };
                    }
                }
                self.ifs[i].live = false;
                for b in [r.then_first, r.else_first, r.after] {
                    self.merge_block(b, r.before);
                }
                progress = true;
            }
        }
    }

    /// Side `first` of region `r` (one of its first blocks) holds a node.
    fn side_holds(&self, r: &IfRegion, first: u8) -> bool {
        (0..self.len).any(|k| {
            let mut x = self.block[self.order[k] as usize];
            loop {
                if x == first {
                    return true;
                }
                if x == 0 || x == r.before {
                    return false;
                }
                x = self.dom[x as usize];
            }
        })
    }

    /// The guards ([`IfRegion::guard`]) `opt_if_simplification`
    /// (nir_opt_if.c:741-782) inverts: an `if` whose then side is empty and
    /// whose else side is not gets `inot` of its condition and its sides
    /// swapped. The GL front end's loop runs `nir_opt_if` before its
    /// `nir_opt_peephole_select` (gl_nir_linker.c:90-99), so a guard that
    /// call flattens becomes `bcsel(inot(c), rest, at_break)`, which
    /// `nir_opt_algebraic` turns back into `bcsel(c, at_break, rest)`
    /// (nir_opt_algebraic.py:971); one it leaves is `if (flt ..)` after
    /// `inot(fge(a, b))` is `flt(a, b)` (:739), and bifrost's call flattens
    /// it as `bcsel(flt, rest, at_break)` ([`Nir::invert_guards`]). Measured
    /// on case87_databreak.expect.json: of the eight guards the two
    /// innermost keep `fge` (their sides count 1 and 5 with the inner
    /// guard's `inot`, within [`PEEPHOLE_LIMIT_GL`]) and the six outer ones
    /// are `flt` (the next counts 9). Only the guards are inverted here: an
    /// `if` of the source with an empty then side has no reference.
    pub fn inverted_guards(&self) -> [bool; MAX_IFS] {
        let mut inv = [false; MAX_IFS];
        for (i, r) in self.ifs[..self.nifs].iter().enumerate() {
            inv[i] = r.live && r.guard && !self.side_holds(r, r.then_first) && self.side_holds(r, r.else_first);
        }
        inv
    }

    /// The inverted guards ([`Nir::inverted_guards`]) the front end's
    /// `nir_opt_peephole_select` left: the condition is the inverse
    /// comparison (`inot(fge(a, b))` is `flt(a, b)` and `inot(flt(a, b))`
    /// `fge(a, b)`, nir_opt_algebraic.py:738-739, the compare used once),
    /// the sides swapped and each phi's sources with them
    /// (`rewrite_phi_predecessor_blocks`, nir_opt_if.c:767-768). A compare
    /// read by anything else keeps its `inot`, a form no reference draws.
    pub fn invert_guards(&mut self, inv: &[bool; MAX_IFS]) -> Result<(), MaliError> {
        for i in 0..self.nifs {
            let r = self.ifs[i];
            if !inv[i] || !r.live {
                continue;
            }
            let Val::Lane(c, _) = r.cond else { return Err(MaliError::LoopForm) };
            if self.uses(c).next().is_some() {
                return Err(MaliError::LoopForm);
            }
            let flip = |k: Cmpf| match k {
                Cmpf::Ge => Some(Cmpf::Lt),
                Cmpf::Lt => Some(Cmpf::Ge),
                _ => None,
            };
            self.nodes[c as usize].kind = match self.nodes[c as usize].kind {
                Kind::Alu(Alu::Fcmp(k)) => Kind::Alu(Alu::Fcmp(flip(k).ok_or(MaliError::LoopForm)?)),
                Kind::Alu(Alu::Icmp(k)) => Kind::Alu(Alu::Icmp(flip(k).ok_or(MaliError::LoopForm)?)),
                _ => return Err(MaliError::LoopForm),
            };
            for k in 0..self.len {
                let id = self.order[k] as usize;
                if self.block[id] == r.after && self.nodes[id].kind == Kind::Phi {
                    self.nodes[id].src.swap(0, 1);
                }
            }
            let g = &mut self.ifs[i];
            (g.then_first, g.else_first) = (g.else_first, g.then_first);
            (g.then_last, g.else_last) = (g.else_last, g.then_last);
        }
        Ok(())
    }

    /// The guards of a loop whose test and `break` are one terminator
    /// ([`IfRegion::single`]): `complex_unroll_single_terminator`
    /// (nir_opt_loop_unroll.c:466-527) clones the values the loop leaves
    /// into both sides of its last pass's `if` (the "lcssa vars"), so that
    /// `if`'s then side is not empty and `opt_if_simplification`
    /// (nir_opt_if.c:741-782) leaves it, while every earlier pass's guard,
    /// its then side the empty `break`, is inverted and reaches the
    /// selects as `flt` ([`Nir::invert_guards`]). The front end's count
    /// ([`Nir::inverted_guards`]) does not apply to them: `inots` is
    /// cleared for each. Measured: cflow_71_deadloop.expect.json, whose
    /// ladder has `FCMP_OR.v2f16.ge` for the last pass and `.lt` for the
    /// seven before it (docs/notes/mali-control-flow.md).
    pub fn invert_ladder(&mut self, inots: &mut [bool; MAX_IFS]) -> Result<(), MaliError> {
        let mut inv = [false; MAX_IFS];
        for i in 0..self.nifs {
            let r = self.ifs[i];
            if !r.live || !r.single {
                continue;
            }
            inots[i] = false;
            inv[i] = self.ifs[..self.nifs].iter().any(|x| x.live && x.single && self.inside(&r, x.before));
        }
        self.invert_guards(&inv)
    }

    /// An inverted guard that stays a branch ([`Nir::invert_guards`]; femtovg's
    /// filter_image, whose 24 passes are too large for the peephole) is laid
    /// out as `emit_if` lays out the `if` `opt_if_simplification` made: the
    /// then side (the rest of the pass and the later ones) before the else
    /// side (the `break`'s block), so the blocks of the two sides are
    /// renumbered (fvg_filter_image.expect.json: each guard's `BRANCHZ.eq`
    /// skips the pass, the break's copies stand after it).
    pub fn lay_out_inverted_guards(&mut self) {
        loop {
            let r = self.live_ifs().find(|r| r.guard && r.else_first < r.then_first && r.else_last + 1 == r.then_first);
            let Some(r) = r.copied() else { return };
            let (lo, mid, hi) = (r.else_first, r.then_first, r.then_last);
            let map = |b: u8| match b {
                b if b >= lo && b < mid => b + (hi - mid + 1),
                b if b >= mid && b <= hi => b - (mid - lo),
                b => b,
            };
            for k in 0..self.n {
                self.block[k] = map(self.block[k]);
            }
            let old = self.dom;
            for b in 0..self.nblocks {
                self.dom[map(b) as usize] = map(old[b as usize]);
            }
            for r in self.ifs[..self.nifs].iter_mut() {
                for b in [
                    &mut r.before,
                    &mut r.then_first,
                    &mut r.then_last,
                    &mut r.else_first,
                    &mut r.else_last,
                    &mut r.after,
                ] {
                    *b = map(*b);
                }
            }
            for r in self.loops[..self.nloops].iter_mut() {
                for b in [&mut r.pre, &mut r.header, &mut r.latch, &mut r.exit] {
                    *b = map(*b);
                }
            }
        }
    }

    /// `nir_opt_constant_folding` of an operation whose sources are
    /// constants and `bcsel`s of two constants on one condition
    /// (nir_opt_constant_folding.c:59-140, `is_bcsel_with_two_constants`):
    /// the operation folds on each side and becomes a `bcsel` of the two
    /// results, where it stood. An fneg, a move and a conversion of the
    /// bit size are left (`should_fold_bcsel`); a 16-bit node too (no
    /// reference). Measured: case 87's count after the pass that may
    /// `continue`, `bcsel(c, 2.0, 3.0)`, plus 1.0 in each later pass, is
    /// `bcsel(c, 3.0, 4.0)` and so on (case87_databreak.expect.json, `%7`
    /// to `%17` of docs/notes/mali-loop-break.md).
    pub fn fold_const_selects(&mut self) {
        for k in 0..self.len {
            let id = self.order[k] as usize;
            let nd = self.nodes[id];
            let Kind::Alu(op) = nd.kind else { continue };
            if nd.bits != 32 || nd.width != 1 || matches!(op, Alu::Fneg | Alu::Vec2 | Alu::Bcsel) {
                continue;
            }
            let mut cond = None;
            let mut sides = [[Val::Undef; 3]; 2];
            let mut ok = true;
            for (s, v) in nd.src.iter().enumerate().take(op.arity()) {
                match *v {
                    Val::Const(_) => {
                        sides[0][s] = *v;
                        sides[1][s] = *v;
                    }
                    Val::Lane(n, 0) => match self.nodes[n as usize] {
                        NirNode {
                            kind: Kind::Alu(Alu::Bcsel),
                            src: [c, a @ Val::Const(_), b @ Val::Const(_)],
                            bits: 32,
                            width: 1,
                        } if cond.is_none_or(|x| x == c) => {
                            cond = Some(c);
                            sides[0][s] = a;
                            sides[1][s] = b;
                        }
                        _ => ok = false,
                    },
                    _ => ok = false,
                }
            }
            let Some(c) = cond.filter(|_| ok) else { continue };
            let fold = |nir: &mut Self, s: [Val; 3]| match nir.alu(op, s[0], s[1], s[2]) {
                Ok(v @ Val::Const(_)) => Some(v),
                _ => None,
            };
            let (Some(a), Some(b)) = (fold(self, sides[0]), fold(self, sides[1])) else { continue };
            self.nodes[id] = NirNode { kind: Kind::Alu(Alu::Bcsel), src: [c, a, b], bits: 32, width: 1 };
        }
    }

    /// `nir_opt_cse` of the comparisons the flattening left equal in one
    /// block: the walk made each in its own side (neither dominating the
    /// other, so its eager CSE kept both, [`Nir::alu`]), and once the
    /// sides merged Mesa's next optimisation round keeps the first. Its
    /// readers read the first; the other dies (the next [`dce`]).
    /// Measured: case 87's `top > 0.5`, the `continue`'s test inside the
    /// loop and the colour's after it, one `FCMP_OR` in
    /// case87_databreak.expect.json.
    pub fn cse_compares(&mut self) {
        for k in 0..self.len {
            let id = self.order[k];
            let nd = self.nodes[id as usize];
            if !matches!(nd.kind, Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_))) {
                continue;
            }
            let first = (0..k).map(|j| self.order[j]).find(|&j| {
                let e = self.nodes[j as usize];
                e.kind == nd.kind
                    && e.bits == nd.bits
                    && e.src == nd.src
                    && self.dominates(self.block[j as usize], self.block[id as usize])
            });
            let Some(first) = first else { continue };
            for j in 0..self.len {
                let u = self.order[j] as usize;
                for s in self.nodes[u].src.iter_mut() {
                    if let Val::Lane(n, c) = *s {
                        if n == id {
                            *s = Val::Lane(first, c);
                        }
                    }
                }
            }
            for r in self.ifs[..self.nifs].iter_mut() {
                if let Val::Lane(n, c) = r.cond {
                    if n == id {
                        r.cond = Val::Lane(first, c);
                    }
                }
            }
        }
    }

    fn note(&mut self, p: Prod) {
        self.prods |= 1 << p as u32;
    }

    /// A production of the shader's own code (the viewport transform's
    /// arithmetic is every program's and names none).
    fn note_user(&mut self, p: Prod) {
        if self.user {
            self.note(p);
        }
    }

    /// Node `n` reads an attribute or the vertex or instance ID, itself or
    /// through its sources (`nir_vertex_divergence_analysis`: the value is
    /// divergent).
    fn reads_attribute(&self, n: u16) -> bool {
        self.reads(n, |k| matches!(k, Kind::Input { .. } | Kind::Preload { .. }))
    }

    /// Node `n` reads a texture look-up, itself or through its sources: a
    /// value Mesa's link neither propagates into the fragment shader (a
    /// `tex` is no uniform expression, nir_opt_varyings.c:2846-2878) nor
    /// rewrites.
    fn reads_tex(&self, n: u16) -> bool {
        self.reads(n, |k| matches!(k, Kind::VsTex { .. }))
    }

    /// Node `n` reads a uniform array's element at a run-time index, itself
    /// or through its sources ([`Kind::UniformIdx`]).
    fn reads_indexed(&self, n: u16) -> bool {
        self.reads(n, |k| matches!(k, Kind::UniformIdx { .. }))
    }

    /// Node `n` is arithmetic on uniforms and constants alone: no
    /// attribute, look-up, run-time index or phi behind it (a lane Mesa's
    /// link promotes to flat, [`super::link::VaryingConsts::push_flat`]).
    fn uniform_only(&self, n: u16) -> bool {
        !self.reads(n, |k| !matches!(k, Kind::Uniform { .. } | Kind::Alu(_)))
    }

    /// Node `n` is convergent (`nir_vertex_divergence_analysis`): no
    /// attribute, vertex or instance ID or phi behind it. A run-time
    /// indexed read at an int uniform and a look-up at such values are,
    /// so Mesa's link promotes their lanes to flat as it does those of
    /// uniforms alone (nir_opt_varyings.c:4964-5002; measured:
    /// uniarray_vs.expect.json and unitype_116v.expect.json load
    /// `.src_flat32`).
    fn convergent(&self, n: u16) -> bool {
        !self.reads(n, |k| matches!(k, Kind::Input { .. } | Kind::Preload { .. } | Kind::Phi))
    }

    /// Node `n` is, or reads through its sources, a node of a kind `f`
    /// accepts.
    fn reads(&self, n: u16, f: impl Fn(Kind) -> bool) -> bool {
        let mut seen = [0u64; MAX_NODES.div_ceil(64)];
        let mut stack = [0u16; MAX_NODES];
        let mut sp = 1;
        stack[0] = n;
        while sp > 0 {
            sp -= 1;
            let x = stack[sp] as usize;
            if seen[x / 64] & (1 << (x % 64)) != 0 {
                continue;
            }
            seen[x / 64] |= 1 << (x % 64);
            let nd = self.nodes[x];
            if f(nd.kind) {
                return true;
            }
            for v in nd.src {
                /* each node is pushed at most once a source, at most three
                 * a node: the unvisited fit */
                if let Val::Lane(y, _) | Val::Pair(y, _) = v {
                    if seen[y as usize / 64] & (1 << (y % 64)) == 0 && sp < stack.len() {
                        stack[sp] = y;
                        sp += 1;
                    }
                }
            }
        }
        false
    }

    pub fn has(&self, p: Prod) -> bool {
        self.prods & (1 << p as u32) != 0
    }

    /// A vertex shader's live look-ups ([`Kind::VsTex`]) as (descriptor
    /// index, sampler binding), in program order: the bindings the stage
    /// reads at indices 0, 1, .. of its tables (one pair per look-up; a
    /// sampler read twice repeats).
    pub fn bound_samplers(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.order[..self.len].iter().filter_map(|&id| match self.nodes[id as usize].kind {
            Kind::VsTex { sampler, index, .. } => Some((index, sampler)),
            _ => None,
        })
    }

    /// The descriptor indices of the live cube look-ups ([`Kind::VsTex`]
    /// with `cube`), in program order ([`super::MaliShader::cube_samplers`]).
    pub fn cube_samplers(&self) -> impl Iterator<Item = u8> + '_ {
        self.order[..self.len].iter().filter_map(|&id| match self.nodes[id as usize].kind {
            Kind::VsTex { cube: true, index, .. } => Some(index),
            _ => None,
        })
    }

    /// The attribute record of location `loc`, which is its `LD_ATTR`
    /// index: how many of the locations Mesa numbers ([`Nir::counted`])
    /// are below `loc` (docs/notes/mali-builtin-inputs.md, "Attribute
    /// records").
    pub fn attribute_record(&self, loc: u8) -> Result<u8, MaliError> {
        let below = 1u64.checked_shl(u32::from(loc)).ok_or(MaliError::Defect(Defect::IrIndex))? - 1;
        Ok((self.counted & below).count_ones() as u8)
    }

    /// The attribute locations a load in the order reads, one bit each.
    pub(super) fn inputs_read(&self) -> Result<u64, MaliError> {
        let mut read = 0u64;
        for &id in &self.order[..self.len] {
            if let Kind::Input { loc, .. } = self.nodes[id as usize].kind {
                read |= 1u64.checked_shl(u32::from(loc)).ok_or(MaliError::Defect(Defect::IrIndex))?;
            }
        }
        Ok(read)
    }

    /// The attribute locations the values `roots` read, themselves or
    /// through their sources, one bit each.
    fn inputs_of(&self, roots: &[Val]) -> Result<u64, MaliError> {
        let mut seen = [0u64; MAX_NODES.div_ceil(64)];
        let mut stack = [0u16; MAX_NODES];
        let mut sp = 0;
        let mut read = 0u64;
        /* a node is marked when pushed, so each is pushed once: the stack
         * holds them all */
        let push = |v: Val, seen: &mut [u64], stack: &mut [u16], sp: &mut usize| {
            if let Val::Lane(y, _) | Val::Pair(y, _) = v {
                let y = y as usize;
                if seen[y / 64] & (1 << (y % 64)) == 0 {
                    seen[y / 64] |= 1 << (y % 64);
                    *stack.get_mut(*sp).ok_or(MaliError::Defect(Defect::IrIndex))? = y as u16;
                    *sp += 1;
                }
            }
            Ok::<(), MaliError>(())
        };
        for &v in roots {
            push(v, &mut seen, &mut stack, &mut sp)?;
        }
        while sp > 0 {
            sp -= 1;
            let nd = self.nodes[stack[sp] as usize];
            if let Kind::Input { loc, .. } = nd.kind {
                read |= 1u64.checked_shl(u32::from(loc)).ok_or(MaliError::Defect(Defect::IrIndex))?;
            }
            for v in nd.src {
                push(v, &mut seen, &mut stack, &mut sp)?;
            }
        }
        Ok(read)
    }

    fn push(&mut self, kind: Kind, src: [Val; 3]) -> Result<u16, MaliError> {
        self.push_sized(kind, src, 32)
    }

    fn push_sized(&mut self, kind: Kind, src: [Val; 3], bits: u8) -> Result<u16, MaliError> {
        self.push_node(NirNode { kind, src, bits, width: 1 })
    }

    /// Marks exact every node that the values `roots` read, through every
    /// source: glsl_to_nir makes an invariant output's assignment exact
    /// (with `propagate_invariance`, every temporary it reads), and
    /// `nir_opt_cse` keeps `exact` on the one instruction left of two equal
    /// ones (nir_instr_set.c: `exact` is or-ed in), so a term an invariant
    /// output shares with another output is exact in both
    /// (deqp_misc2_invariant_cse).
    pub(super) fn mark_exact(&mut self, roots: &[Val]) {
        let node = |v: Val| match v {
            Val::Lane(n, _) | Val::Pair(n, _) => Some(n as usize),
            _ => None,
        };
        for &v in roots {
            if let Some(n) = node(v).filter(|&n| n < self.n) {
                self.exact[n] = true;
            }
        }
        /* a fixpoint: a node rewritten in place may read a later one */
        loop {
            let mut changed = false;
            for x in (0..self.n).rev() {
                if !self.exact[x] {
                    continue;
                }
                for s in self.nodes[x].src {
                    if let Some(n) = node(s).filter(|&n| n < self.n && !self.exact[n]) {
                        self.exact[n] = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Node `nd`, appended to the program order.
    pub(super) fn push_node(&mut self, nd: NirNode) -> Result<u16, MaliError> {
        if self.n >= MAX_NODES {
            return Err(MaliError::TooManyValues);
        }
        let id = self.n as u16;
        self.nodes[self.n] = nd;
        self.block[self.n] = self.cur;
        self.order[self.len] = id;
        self.n += 1;
        self.len += 1;
        Ok(id)
    }

    /// The node of a load, CSE'd by what it loads; `comps` grows to the
    /// widest load of the same thing.
    pub(super) fn load(&mut self, kind: Kind) -> Result<u16, MaliError> {
        for i in 0..self.n {
            let k = self.nodes[i].kind;
            let hit = match (k, kind) {
                (Kind::Input { loc: a, .. }, Kind::Input { loc: b, .. }) => a == b,
                (Kind::Uniform { dword: a, .. }, Kind::Uniform { dword: b, .. }) => a == b,
                (Kind::Sysval(a), Kind::Sysval(b)) => a == b,
                (Kind::Preload { reg: a }, Kind::Preload { reg: b }) => a == b,
                _ => false,
            };
            if hit {
                self.nodes[i].kind = match (k, kind) {
                    (Kind::Input { loc, comps: a }, Kind::Input { comps: b, .. }) => {
                        Kind::Input { loc, comps: a.max(b) }
                    }
                    (Kind::Uniform { dword, comps: a }, Kind::Uniform { comps: b, .. }) => {
                        Kind::Uniform { dword, comps: a.max(b) }
                    }
                    (k, _) => k,
                };
                return Ok(i as u16);
            }
        }
        self.push(kind, [Val::Undef; 3])
    }

    /// [`Nir::load`] of 16-bit lanes (a vertex shader's fp16 uniform,
    /// [`fp16_uniform`]).
    pub(super) fn load16(&mut self, kind: Kind) -> Result<u16, MaliError> {
        let n = self.load(kind)?;
        self.nodes[n as usize].bits = 16;
        Ok(n)
    }

    /// One 32-bit scalar operation, folded, simplified and CSE'd as NIR
    /// would (a highp fragment shader's too, nir_fs.rs).
    pub(super) fn alu(&mut self, op: Alu, a: Val, b: Val, c: Val) -> Result<Val, MaliError> {
        let bits = if op == Alu::F2f16 { 16 } else { 32 };
        self.alu_sized(op, bits, a, b, c)
    }

    /// `build_cube_desc` (pan_nir_lower_tex.c:103-128) over
    /// `build_cube_coord2_face` (:68-100) of the direction `(x, y, z)`, in
    /// the builder's order: the face transform's two lanes, the face's S and
    /// T, `0.5 / max{x, y, z}` as `lower_fdiv` leaves it (`fmul(0.5,
    /// frcp(max))`), `fsat(ffma(s, k, 0.5))` for each, the face packed above
    /// S's bit 29. `va_lower_tex` makes it in either stage (it does not look
    /// at the stage): the two staging words S and T.
    pub(super) fn cube_desc(&mut self, x: Val, y: Val, z: Val) -> Result<(Val, Val), MaliError> {
        let max = self.alu(Alu::Cubeface1, x, y, z)?;
        let face = self.alu(Alu::Cubeface2, x, y, z)?;
        let s = self.alu(Alu::CubeSsel, z, x, face)?;
        let t = self.alu(Alu::CubeTsel, y, z, face)?;
        let r = self.alu(Alu::Frcp, max, Val::Undef, Val::Undef)?;
        let k = self.alu(Alu::Fmul, Val::Const(F32_HALF), r, Val::Undef)?;
        let s = self.alu(Alu::Ffma, s, k, Val::Const(F32_HALF))?;
        let s = self.alu(Alu::Fclamp(clamp::ZERO_ONE), s, Val::Undef, Val::Undef)?;
        let t = self.alu(Alu::Ffma, t, k, Val::Const(F32_HALF))?;
        let t = self.alu(Alu::Fclamp(clamp::ZERO_ONE), t, Val::Undef, Val::Undef)?;
        Ok((self.alu(Alu::BitfieldSelect, Val::Const(CUBE_S_MASK), s, face)?, t))
    }

    /// An explicit level's staging word, `u2u32(build_sfixed_8_8(lod))`:
    /// f2i16(fsat_signed(lod * 1/16) * 4096) (pan_nir_lower_tex.c:136-141,
    /// :934-937).
    pub(super) fn sfixed_8_8(&mut self, level: Val) -> Result<Val, MaliError> {
        let x = self.alu(Alu::Fmul, level, Val::Const(LOD_DIV16), Val::Undef)?;
        let x = self.alu(Alu::Fclamp(clamp::M1_1), x, Val::Undef, Val::Undef)?;
        let x = self.alu(Alu::Fmul, x, Val::Const(LOD_8_8), Val::Undef)?;
        let x = self.alu_sized(Alu::F2i, 16, x, Val::Undef, Val::Undef)?;
        self.alu(Alu::U2u32, x, Val::Undef, Val::Undef)
    }

    /// The same at the value's bit size `bits`.
    fn alu_sized(&mut self, op: Alu, bits: u8, a: Val, b: Val, c: Val) -> Result<Val, MaliError> {
        /* a compare reads a -0.0 operand as +0.0: the rules of
         * nir_opt_algebraic.py:771-776 make a new 0.0 for the compare, and
         * the IR's sides are already where those rules put them
         * (glsl.rs `neg_zero_compares`) */
        let pz = |v: Val| if v == Val::Const(F32_NEG_ZERO) { Val::Const(F32_ZERO) } else { v };
        let (a, b) = if matches!(op, Alu::Fcmp(_)) && bits == 32 { (pz(a), pz(b)) } else { (a, b) };
        let srcs = [a, b, c];
        if srcs[..op.arity()].contains(&Val::Undef) {
            return Err(MaliError::PositionLane);
        }
        let consts: [Option<u32>; 3] = srcs.map(|v| match v {
            Val::Const(x) => Some(x),
            _ => None,
        });
        if consts[..op.arity()].iter().all(|c| c.is_some()) && !op.is_intrinsic() {
            /* the fp16 operations: `f2f16` of a value fp16 holds exactly
             * (then the rounding cannot matter), `fmax` of two fp16 */
            match (op, bits) {
                (Alu::F2f16, _) => {
                    let h = select::f16_exact(consts[0].unwrap_or(0)).ok_or(MaliError::Fp16Inexact)?;
                    return Ok(Val::Const(h as u32));
                }
                (Alu::Fmax, 16) => {
                    let f = |k: usize| f32::from_bits(fau::half_to_float(consts[k].unwrap_or(0) as u16));
                    return Ok(Val::Const(if f(0) >= f(1) { consts[0] } else { consts[1] }.unwrap_or(0)));
                }
                _ => {}
            }
            if op == Alu::Fneg {
                /* the sign bit, as NIR's constant folding flips it */
                return Ok(Val::Const(consts[0].unwrap_or(0) ^ F32_NEG_ZERO));
            }
            if op == Alu::Fabs {
                return Ok(Val::Const(consts[0].unwrap_or(0) & !F32_NEG_ZERO));
            }
            let k = |i: usize| consts[i].unwrap_or(0);
            /* the boolean operands (0 / ~0, a NaN pattern as a float):
             * `bcsel` of a constant condition is the arm it takes, `b2f`
             * is 1.0 or 0.0 (nir_opt_constant_folding; the GLSL front end
             * folds `c ? a : b` of constants the same) */
            match op {
                Alu::Bcsel => {
                    let r = if k(0) != 0 { k(1) } else { k(2) };
                    self.note_user(Prod::Folded);
                    return Ok(Val::Const(r));
                }
                Alu::B2f => {
                    self.note_user(Prod::Folded);
                    return Ok(Val::Const(if k(0) != 0 { F32_ONE } else { F32_ZERO }));
                }
                _ => {}
            }
            /* an integer operation of constants reads its operands as
             * ints, not float bits ([`fold_int_const`]: an explicit
             * level of a constant, an `ior` of bools) */
            if let Some(r) = fold_int_const(op, bits, [k(0), k(1), k(2)]) {
                self.note_user(Prod::Folded);
                return Ok(Val::Const(r));
            }
            let f = |k: usize| f32::from_bits(consts[k].unwrap_or(0));
            /* the viewport transform's clamp of `1/w`: `frcp` of a zero w
             * is the IEEE infinity of its sign, which `fmax` and `fmin`
             * with ±2^15 take to a bound (nir_lower_viewport_transform.c:
             * 83-88; deqp_cfold2_zero_operand_rcp_inf, `gl_Position =
             * vec4(0.0)`: w is 32768.0). The shader's own code still
             * refuses an infinity in or out */
            let clamp_inf = !self.user && bits == 32 && matches!(op, Alu::Fmax | Alu::Fmin);
            let ok = |c: u32| plain(c) || (clamp_inf && f32::from_bits(c).is_infinite());
            if consts[..op.arity()].iter().any(|c| !ok(c.unwrap_or(0))) {
                return Err(MaliError::ConstantFold);
            }
            /* exact, from the shared fold (no libm): what the GLSL front
             * end's `floorf`, `ceilf`, `truncf` and compares give
             * (ir_expression_operation.py:561-563, :642-645) */
            let exact = |a: AluOp| fold::fold_float(a, k(0), k(1), 0).ok_or(MaliError::ConstantFold);
            let r = match op {
                Alu::Fadd => f(0) + f(1),
                Alu::Fmul => f(0) * f(1),
                /* unfused, `a * b` then `+ c`: a constant `ffma` is the
                 * front end's expansion of a built-in on constants
                 * (`smoothstep`, `mod`, `dot`), which Mesa's GLSL front end
                 * folds as `a * b + c` with two roundings
                 * (ir_expression_operation.py:755; deqp_constop_ce_mod's
                 * 0x40233340 against the fused 0x40233335) */
                Alu::Ffma => f(0) * f(1) + f(2),
                Alu::Ffloor => f32::from_bits(exact(AluOp::Ffloor)?),
                Alu::Fceil => f32::from_bits(exact(AluOp::Fceil)?),
                Alu::Ftrunc => f32::from_bits(exact(AluOp::Ftrunc)?),
                /* `fmin(fmax(a, lo), hi)` of the clamp's bounds */
                Alu::Fclamp(m) => match m {
                    clamp::ZERO_INF => f(0).max(0.0),
                    clamp::M1_1 => f(0).clamp(-1.0, 1.0),
                    clamp::ZERO_ONE => f(0).clamp(0.0, 1.0),
                    _ => f(0),
                },
                Alu::Fcmp(c) => {
                    let (x, y) = (f(0), f(1));
                    let t = match c {
                        Cmpf::Eq => x == y,
                        Cmpf::Gt => x > y,
                        Cmpf::Ge => x >= y,
                        Cmpf::Ne => x != y,
                        Cmpf::Lt => x < y,
                        Cmpf::Le => x <= y,
                    };
                    self.note_user(Prod::Folded);
                    return Ok(Val::Const(if t { u32::MAX } else { 0 }));
                }
                Alu::Frcp => 1.0 / f(0),
                Alu::Fmax => f(0).max(f(1)),
                Alu::Fmin => f(0).min(f(1)),
                /* `1.0F / sqrtf(x)`, the GLSL front end's `rsq`
                 * (ir_expression_operation.py:453) and NIR's (:267), with
                 * the correctly rounded root: `normalize` of a constant
                 * (gm2_arithconst_3/4, [`const_dot`]) */
                Alu::Frsq => match fold::fold_float(AluOp::Frsq, consts[0].unwrap_or(0), 0, 0) {
                    Some(r) => f32::from_bits(r),
                    None => return Err(MaliError::ConstantFold),
                },
                /* the front end folds a constant expression: a constant
                 * operand here comes from a shape no reference draws */
                Alu::Icmp(_)
                | Alu::F2i
                | Alu::I2f
                | Alu::U2f32
                | Alu::U2u32
                | Alu::F2f32
                | Alu::Iadd
                | Alu::Ishl
                | Alu::BitfieldSelect
                | Alu::Iand
                | Alu::Ior
                | Alu::Imax
                | Alu::Imin
                | Alu::Iabs
                | Alu::Ushr
                | Alu::Imul
                | Alu::I2i32
                | Alu::F2i32
                | Alu::Inot
                | Alu::Ishr
                | Alu::U8f32(_)
                | Alu::Ixor
                | Alu::Isub
                | Alu::Ineg
                | Alu::U2f
                | Alu::F2u
                | Alu::Extract16(_)
                | Alu::B2i
                | Alu::Ucmp(_)
                | Alu::Fsin
                | Alu::Fcos
                | Alu::Flog2
                | Alu::Fexp2
                | Alu::Fpow
                | Alu::Fddx
                | Alu::Fddy => return Err(MaliError::ConstantFold),
                Alu::F2f16
                | Alu::Bcsel
                | Alu::B2f
                | Alu::Fneg
                | Alu::Fabs
                | Alu::Vec2
                | Alu::Mov
                | Alu::Cubeface1
                | Alu::Cubeface2
                | Alu::CubeSsel
                | Alu::CubeTsel => return Err(MaliError::Defect(Defect::IrIndex)),
            };
            /* an overflow to infinity or a denormal of `+`, `*`, `min`, `max`
             * stays as IEEE gives it: the GL path sets no float controls, so
             * `nir_eval_const_opcode` neither flushes nor rounds toward zero
             * (nir_constant_expressions.py:97-98; deqp_constop_cf_inf_fs's
             * 0x7f800000, cf_denorm_fs's 0x00000001). A NaN and the
             * reciprocals' (`rcp(0.0)`) have no reference */
            /* `frcp(±0.0)` is IEEE's `±inf` the same way: the arm of
             * the front end's `atan` a constant select drops (`u =
             * bcsel(|x| <= 1.0, x, rcp(x))`, deqp_constrnd_atan_zero_lanes_vs's
             * 0 lanes). An infinity that another fold reads is not
             * `plain` and stays refused there, but in the viewport
             * transform's clamp of `1/w` (`clamp_inf` above) */
            let ieee = (matches!(op, Alu::Fadd | Alu::Fmul | Alu::Fmin | Alu::Fmax) && !r.is_nan())
                || (op == Alu::Frcp && r.is_infinite());
            if !plain(r.to_bits()) && !ieee {
                return Err(MaliError::ConstantFold);
            }
            self.note_user(Prod::Folded);
            return Ok(Val::Const(r.to_bits()));
        }
        /* the rules on one constant value match the IR's vector constant
         * whole ([`Nir::nonuniform_const`]) */
        let uniform = !self.nonuniform_const;
        match op {
            Alu::Fadd if uniform && (is_zero(a) || is_zero(b)) => {
                self.note_user(Prod::Identity);
                return Ok(if is_zero(a) { b } else { a });
            }
            /* `fadd(fneg(a), fadd(a, b))` and `fadd(a, fadd(fneg(a), b))`
             * are `fcanonicalize(b)`, which is `b` (nir_opt_algebraic.py:271-272,
             * inexact `~fadd`: GLSL ES 1.00 has no `precise`; a commutative
             * match on both sums): the terrain noise's `abs(x2 - x1)` of
             * `x2 = x1 + x0` is `abs(x0)` (gm2_locals_noise,
             * docs/notes/mali-local-arrays.md section 7) */
            Alu::Fadd if bits == 32 => {
                /* `fadd(a, fneg(a))` is 0.0 (nir_opt_algebraic.py's inexact
                 * `~fadd(a, fneg(a))`; the eager CSE here made the two
                 * products of `a * b - a * b` one node,
                 * deqp_misc2_invariant_cse's `v_unrelated`) */
                let neg_of = |s: &Self, n: Val, x: Val| matches!(n, Val::Lane(id, 0) if s.is_alu(n, Alu::Fneg) && s.nodes[id as usize].src[0] == x);
                if !self.exact_build && (neg_of(self, a, b) || neg_of(self, b, a)) {
                    self.note_user(Prod::Identity);
                    return Ok(Val::Const(F32_ZERO));
                }
                if let Some(r) = self.cancelled_sum(a, b).or_else(|| self.cancelled_sum(b, a)) {
                    self.note_user(Prod::Identity);
                    return Ok(r);
                }
            }
            _ => {}
        }
        match op {
            Alu::Fmul if uniform && (is_zero(a) || is_zero(b)) => {
                self.note_user(Prod::Identity);
                /* the zero's own sign: GLSL's `-0.0` is `neg(0.0)`, and
                 * `fmul(a, fneg(b))` is `fneg(fmul(a, b))` before
                 * `fmul(a, 0.0)` is 0.0 (nir_opt_algebraic.py:277), so
                 * `a * -0.0` is -0.0 (deqp_constop_nz_shared_vs: Mesa's
                 * `bcsel(c, -0.0, ..)` of `a_position * z`) */
                return Ok(if is_zero(a) { a } else { b });
            }
            /* `ffma(0.0, a, b)` is `b` (nir_opt_algebraic.py:305-306) */
            Alu::Ffma if uniform && (is_zero(a) || is_zero(b)) => {
                self.note_user(Prod::Identity);
                return Ok(c);
            }
            Alu::Fmul if uniform && (a == Val::Const(F32_ONE) || b == Val::Const(F32_ONE)) => {
                self.note_user(Prod::Identity);
                return Ok(if a == Val::Const(F32_ONE) { b } else { a });
            }
            /* `fadd(fmul(a, b), fmul(a, c))` is `fmul(a, fadd(b, c))`
             * (nir_opt_algebraic.py:246, the distributive law, a commutative
             * match on both products, no use count): the shared factor
             * first, then the sum, which folds when both are constants. An
             * unrolled loop's `acc += v * float(i + 1)` collapses into one
             * product by this (tools/mali/testdata/s6_discloop: Mesa's
             * `fmul %2.y, 10.0`); NIR saw the two products as distinct
             * instructions where this graph may hold one node (the CSE here
             * is eager), so `fadd(x, x)` of a product is the same rule, not
             * nir_opt_algebraic.py:274's `fmul(a, 2.0)` */
            Alu::Fadd if bits == 32 && self.user => {
                let product = |v: Val| match v {
                    Val::Lane(n, 0) => match self.nodes[n as usize] {
                        NirNode { kind: Kind::Alu(Alu::Fmul), src, bits: 32, width: 1 } => Some((n, [src[0], src[1]])),
                        _ => None,
                    },
                    _ => None,
                };
                if let (Some((na, pa)), Some((nb, pb))) = (product(a), product(b)) {
                    /* the matcher binds `a` to the first product's first
                     * source, then the swapped order: a value before a
                     * constant here, since the product a distributive step
                     * made has the value first and the eager CSE may have
                     * folded it into an earlier constant-first node (lane 0
                     * of s6_discloop's third pass: `fadd(x, x)` of `fmul(3.0,
                     * u)`, which NIR held as `fmul(u, 3)` and `fmul(3, u)`) */
                    let shared = pa
                        .iter()
                        .copied()
                        .find(|x| pb.contains(x) && !matches!(x, Val::Const(_)))
                        .or_else(|| pa.iter().copied().find(|x| pb.contains(x)));
                    if let Some(x) = shared {
                        let other = |p: [Val; 2]| if p[0] == x { p[1] } else { p[0] };
                        let (y, z) = (other(pa), other(pb));
                        let nonuni = self.nonuniform[na as usize] || self.nonuniform[nb as usize];
                        let saved = self.nonuniform_const;
                        self.nonuniform_const |= nonuni;
                        let sum = self.alu(Alu::Fadd, y, z, Val::Undef);
                        let r = sum.and_then(|s| self.alu(Alu::Fmul, x, s, Val::Undef));
                        self.nonuniform_const = saved;
                        return r;
                    }
                }
            }
            /* a multiply by a constant keeps the source's order (Mesa's GLSL
             * path, measured on the board: `0.75 * x` is `FMA 0.75, x`, `x *
             * 0.75` is `FMA x, 0.75`; pan_shc's SPIR-V path puts a scalar
             * after a vector, 0dce654a8:tools/mali/board-ref/2026-10-19-const-order),
             * `-1.0` an fneg (nir_opt_algebraic.py:296), `2.0` turned into
             * `fadd(a, a)` after the fusion ([`late_rules`]) */
            Alu::Fmul if uniform && (a == Val::Const(F32_MINUS_ONE) || b == Val::Const(F32_MINUS_ONE)) => {
                let x = if a == Val::Const(F32_MINUS_ONE) { b } else { a };
                self.note_user(Prod::FmulMinusOne);
                return self.alu(Alu::Fneg, x, Val::Undef, Val::Undef);
            }
            /* `fmul(fneg(a), b)` is `fneg(fmul(a, b))`, `a` the fneg's source
             * on whichever side it stood (nir_opt_algebraic.py:2387, a
             * commutative match); the viewport transform's products too,
             * which bifrost's optimisation loop runs the rules on after
             * `nir_lower_viewport_transform` (a negated position lane,
             * `gl_Position = -a_position`, negmul_vs_neg: the fneg moves to
             * the transform's `ffma`, `FMA.f32 r0, r0^.neg, u0.w0, u0.w1`) */
            Alu::Fmul if self.is_alu(a, Alu::Fneg) && self.is_alu(b, Alu::Fneg) => {
                return Err(MaliError::ArithConstant);
            }
            Alu::Fmul if self.is_alu(a, Alu::Fneg) || self.is_alu(b, Alu::Fneg) => {
                let (n, other) = if self.is_alu(a, Alu::Fneg) { (a, b) } else { (b, a) };
                let Val::Lane(id, _) = n else { return Err(MaliError::Defect(Defect::IrIndex)) };
                let inner = self.nodes[id as usize].src[0];
                self.note_user(Prod::FnegProduct);
                let m = self.alu(Alu::Fmul, inner, other, Val::Undef)?;
                return self.alu(Alu::Fneg, m, Val::Undef, Val::Undef);
            }
            /* `fneg(fneg(a))` is `fcanonicalize(a)` (nir_opt_algebraic.py:230),
             * which is `a` when fp32 denormals are not flushed (:161-167):
             * no instruction (algfold_negneg.expect.json) */
            Alu::Fneg if self.is_alu(a, Alu::Fneg) => {
                let Val::Lane(id, _) = a else { return Err(MaliError::Defect(Defect::IrIndex)) };
                self.note_user(Prod::Identity);
                return Ok(self.nodes[id as usize].src[0]);
            }
            /* `fabs(fneg(a))` and `fabs(fabs(a))` are `fabs(a)` (nir_opt_algebraic.py) */
            Alu::Fabs if self.is_alu(a, Alu::Fneg) => {
                let Val::Lane(id, _) = a else { return Err(MaliError::Defect(Defect::IrIndex)) };
                let inner = self.nodes[id as usize].src[0];
                return self.alu(Alu::Fabs, inner, Val::Undef, Val::Undef);
            }
            Alu::Fabs if self.is_alu(a, Alu::Fabs) => return Ok(a),
            /* a constant times a product with a constant: GLSL IR's
             * `reassociate_constant` (opt_algebraic.cpp:175-215, :276-282)
             * folds the two constants in the front end, in an order not
             * transcribed. A product with a value is left as written
             * (panfrost runs no nir_opt_algebraic_before_ffma; measured:
             * `2.0 * x / u` is `ffma(fadd(x, x), frcp(u), ...)`) */
            Alu::Fmul
                if self.user
                    && consts[..2].iter().any(|c| c.is_some())
                    && (self.is_const_product(a) || self.is_const_product(b)) =>
            {
                return Err(MaliError::ArithConstant);
            }
            /* `ffma(1.0, a, b)` is `fadd(a, b)` and `ffma(-1.0, a, b)` is
             * `fadd(fneg(a), b)` (nir_opt_algebraic.py:311-314, a
             * commutative match on the factors), which the late fusion may
             * fuse again: `dot(v, vec2(1.0, 2.0))` is `ffma(v.y, 2.0, v.x)`
             * and `dot(v, vec2(-1.0, 3.0))` `ffma(v.y, 3.0, fneg(v.x))`
             * (algfold_constfma.expect.json). Any other constant factor
             * keeps its FMA: a dot product by a constant vector,
             * `dot(v.xy, vec2(2.0, 3.0))`, is `FMA v.x, 2.0, t` after `FMA
             * t, v.y, 3.0, -0.0` (tools/mali/testdata/pan_shc-out/
             * s4_motion_dot.json, both stages) */
            Alu::Ffma if consts[..2].iter().any(|c| matches!(c, Some(F32_ONE | F32_MINUS_ONE))) => {
                let k = if matches!(consts[0], Some(F32_ONE | F32_MINUS_ONE)) { 0 } else { 1 };
                let x = if k == 0 { b } else { a };
                let x =
                    if consts[k] == Some(F32_MINUS_ONE) { self.alu(Alu::Fneg, x, Val::Undef, Val::Undef)? } else { x };
                self.note_user(Prod::Identity);
                return self.alu(Alu::Fadd, x, c, Val::Undef);
            }
            _ => {}
        }
        let kind = Kind::Alu(op);
        /* `nir_opt_cse`: an equal instruction in this block or a block that
         * dominates it */
        for k in 0..self.len {
            let i = self.order[k] as usize;
            let nd = self.nodes[i];
            if nd.kind != kind || nd.bits != bits || !self.dominates(self.block[i], self.cur) {
                continue;
            }
            let same = nd.src == srcs
                || (op.commutes() && nd.src[0] == srcs[1] && nd.src[1] == srcs[0] && nd.src[2] == srcs[2]);
            if same {
                self.note_user(Prod::Cse);
                return Ok(Val::Lane(i as u16, 0));
            }
        }
        self.note_user(match op {
            Alu::Fadd if consts[..2].iter().any(|c| c.is_some()) => Prod::FaddConst,
            Alu::Fadd => Prod::Fadd,
            Alu::Fmul if consts[..2].iter().any(|c| c.is_some()) => Prod::FmulConst,
            Alu::Fmul => Prod::Fmul,
            _ => {
                let id = self.push_sized(kind, srcs, bits)?;
                self.nonuniform[id as usize] = self.nonuniform_const;
                return Ok(Val::Lane(id, 0));
            }
        });
        let id = self.push_sized(kind, srcs, bits)?;
        self.nonuniform[id as usize] = self.nonuniform_const;
        Ok(Val::Lane(id, 0))
    }

    /// The `b` of `x + s` when `s` is the fp32 sum `fneg(x) + b` or `b +
    /// fneg(x)`, or when `x` is `fneg(a)` and `s` is `a + b` or `b + a`
    /// (nir_opt_algebraic.py:271-272).
    fn cancelled_sum(&self, x: Val, s: Val) -> Option<Val> {
        let Val::Lane(n, 0) = s else { return None };
        let NirNode { kind: Kind::Alu(Alu::Fadd), src, bits: 32, width: 1 } = self.nodes[n as usize] else {
            return None;
        };
        let neg_of = |v: Val| match v {
            Val::Lane(id, 0) if self.is_alu(v, Alu::Fneg) && self.nodes[id as usize].bits == 32 => {
                Some(self.nodes[id as usize].src[0])
            }
            _ => None,
        };
        for (p, q) in [(src[0], src[1]), (src[1], src[0])] {
            if neg_of(x) == Some(p) || neg_of(p) == Some(x) {
                return Some(q);
            }
        }
        None
    }

    /// `v` is a lane of an ALU operation `op`.
    fn is_alu(&self, v: Val, op: Alu) -> bool {
        matches!(v, Val::Lane(n, _) if self.nodes[n as usize].kind == Kind::Alu(op))
    }

    /// `v` is a product with a constant.
    fn is_const_product(&self, v: Val) -> bool {
        match v {
            Val::Lane(n, _) => {
                let nd = self.nodes[n as usize];
                nd.kind == Kind::Alu(Alu::Fmul) && nd.src[..2].iter().any(|v| matches!(v, Val::Const(_)))
            }
            _ => false,
        }
    }

    pub(super) fn uses(&self, id: u16) -> impl Iterator<Item = (u16, usize)> + '_ {
        (0..self.len).flat_map(move |k| {
            let u = self.order[k];
            let nd = self.nodes[u as usize];
            let arity = nd.arity();
            (0..arity)
                .filter(move |&s| matches!(nd.src[s], Val::Lane(n, _) | Val::Pair(n, _) if n == id))
                .map(move |s| (u, s))
        })
    }
}

impl NirNode {
    /// The sources the node reads (`nir_foreach_src`): an operation's
    /// arity, a phi's two, a store's or discard's one, a look-up's staging
    /// words. The `f2f16` of two 32-bit lanes (`V2F32_TO_V2F16`,
    /// nir_fs.rs `vectorise_int_sources`) reads both.
    pub fn arity(&self) -> usize {
        match self.kind {
            Kind::Alu(Alu::F2f16) if self.width == 2 => 2,
            Kind::Alu(a) => a.arity(),
            Kind::Store { .. } | Kind::UniformIdx { .. } | Kind::Discard => 1,
            /* a fragment look-up's computed coordinate (nir_fs.rs
             * `TexCoord::Computed`; undefined for the other forms) */
            Kind::Phi | Kind::VsTex { explicit: false, .. } => 2,
            /* a cube's third staging source (nir_fs.rs) */
            Kind::Tex | Kind::VsTex { explicit: true, .. } => 3,
            _ => 0,
        }
    }
}

/// The front end's lanes of every IR value, as [`Val`]s. The values' table
/// is in the scratch arena: at [`MAX_VALUES`] it is too large for a
/// compiling thread's stack.
struct Walk<'s> {
    vals: &'s mut [[Val; 4]],
    locals: [[Val; 4]; MAX_LOCALS],
}

fn lane(w: &Walk<'_>, s: ir::Src, i: usize) -> Val {
    w.vals.get(s.val.0 as usize).map_or(Val::Undef, |v| v[s.swizzle[i] as usize & 3])
}

/// A front-end `ffma` is GLSL's `mod(x, y)`, of a constant `y` (gl_test
/// case 221) or of a value (gm2_fma_6's `mod(k, t)`): the front end writes
/// it `ffma(y, fneg(ffloor(fmul(x, frcp(y)))), x)`
/// (builtin_call.rs, "mod"), `srcs` `[y, fneg(..), x]`, the same `x` and
/// `y` (value and swizzle) inside as outside.
fn is_fmod(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let alu = |src: ir::Src, op: AluOp| match m.def(src.val) {
        Some(IrOp::Alu(o, s)) if *o == op => Some(*s),
        _ => None,
    };
    let Some([q, ..]) = alu(srcs[1], AluOp::Fneg).and_then(|[f, ..]| alu(f, AluOp::Ffloor)) else {
        return false;
    };
    let Some([x, r, _]) = alu(q, AluOp::Fmul) else { return false };
    let Some([y, ..]) = alu(r, AluOp::Frcp) else { return false };
    x == srcs[2] && y == srcs[0]
}

/// A front-end `ffma` is GLSL's `mix(a, b, t)`: the front end writes it
/// `ffma(t, fadd(b, fneg(a)), a)` (builtin_call.rs, "mix"), `srcs` `[t,
/// fadd(..), a]`, the same `a` (value and swizzle) inside as outside.
fn is_mix(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let alu = |src: ir::Src, op: AluOp| match m.def(src.val) {
        Some(IrOp::Alu(o, s)) if *o == op => Some(*s),
        _ => None,
    };
    let Some([_, n, _]) = alu(srcs[1], AluOp::Fadd) else { return false };
    alu(n, AluOp::Fneg).is_some_and(|[a, ..]| a == srcs[2])
}

/// A front-end `ffma` is `smoothstep`'s `3.0 - 2.0 * t`: `ffma(-2.0, t,
/// 3.0)` of two constant splats (builtin_call.rs, "smoothstep").
fn is_smoothstep_inner(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let splat = |s: ir::Src, c: u32| match m.def(s.val) {
        Some(IrOp::ConstF32(v)) => s.swizzle.iter().all(|&k| v[k as usize & 3] == c),
        _ => false,
    };
    splat(srcs[0], F32_MINUS_TWO) && splat(srcs[2], F32_THREE)
}

/// A front-end `ffma` is GLSL's `reflect(I, N)`: the front end writes it
/// `ffma(fmul(dot(N, I), -2.0).xxxx, N, I)` (builtin_call.rs, "reflect"),
/// `srcs` the first three, the dot product's first operand `N`. `N` and `I`
/// are no constants (gm2_fma_8's uniform and attribute; a constant's dot
/// folds, which no reference draws).
fn is_reflect(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let Some(IrOp::Alu(AluOp::Fmul, [d, k, _])) = m.def(srcs[0].val) else { return false };
    if [srcs[1], srcs[2]].iter().any(|s| matches!(m.def(s.val), Some(IrOp::ConstF32(_)))) {
        return false;
    }
    let minus_two = matches!(m.def(k.val), Some(IrOp::ConstF32(v)) if v[k.swizzle[0] as usize & 3] == F32_MINUS_TWO);
    let dot = matches!(m.def(d.val),
        Some(IrOp::Alu(AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4, [n, i, _])) if *n == srcs[1] && *i == srcs[2]);
    minus_two && dot && srcs[0].swizzle.iter().all(|&c| c == srcs[0].swizzle[0])
}

/// `x * y` is `smoothstep`'s last product as the front end writes it
/// (builtin_call.rs): `x` the product `t * t` and `y` `ffma(-2.0, t, 3.0)`
/// of the same `t` ([`is_smoothstep_inner`]). Some(`t`).
fn vs_smoothstep_tail(vs: &Nir<'_>, x: Val, y: Val) -> Option<Val> {
    let (Alu::Fmul, [t, t2, _]) = vs_alu_of(vs, x)? else { return None };
    let (Alu::Ffma, [Val::Const(F32_MINUS_TWO), t3, Val::Const(F32_THREE)]) = vs_alu_of(vs, y)? else { return None };
    (t == t2 && t == t3).then_some(t)
}

/// A front-end `ffma` is GLSL's `cross(a, b)`: the front end writes it
/// `ffma(a.yzx, b.zxy, fneg(fmul(a.zxy, b.yzx)))` (builtin_call.rs,
/// "cross"), `srcs` the first three, the same `a` and `b` inside.
fn is_cross(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let alu = |src: ir::Src, op: AluOp| match m.def(src.val) {
        Some(IrOp::Alu(o, s)) if *o == op => Some(*s),
        _ => None,
    };
    let Some([p, q, _]) = alu(srcs[2], AluOp::Fneg).and_then(|[f, ..]| alu(f, AluOp::Fmul)) else {
        return false;
    };
    let sw = |s: ir::Src, k: [u8; 3]| s.swizzle[..3] == k;
    p.val == srcs[0].val
        && q.val == srcs[1].val
        && sw(srcs[0], [1, 2, 0])
        && sw(srcs[1], [2, 0, 1])
        && sw(p, [2, 0, 1])
        && sw(q, [1, 2, 0])
}

/// GLSL's `mod(x, y)` as Mesa lowers `fmod` (`lower_fmod`,
/// nir_opt_algebraic.py:2492): `fsub(x, fmul(y, ffloor(fdiv(x, y))))`, the
/// `fsub` an `fadd` of the `fneg` (`lower_fsub`). `neg_floor` is the walked
/// `fneg(ffloor(..))` of [`is_fmod`]'s shape, its `ffloor` the one
/// kept; the late fusion makes the sum `ffma(-y, floor, x)` ([`fuse`]).
/// Measured: case 221's `mod(id, 2.0) - 0.5` is `FMA.f32 r0,
/// 0x40000000.neg, r1, r0^` over `FADD_IMM.f32 r0, r0^, #0xBF000000`
/// (vsfma_221_instmix.expect.json, with [`vs_fadd`]'s reassociation); a
/// `y` of two uniforms' `mod(k, t)` is `FRCP.f32 r0, u6.w0`, `FMA.f32 r0,
/// u5.w1, r0^, 0x0.neg`, `FROUND.f32.rtn` and `FMA.f32 r0, u6.w0.neg, r0^,
/// r1^` (gm2_fma_6's varying program).
fn vs_fmod(vs: &mut Nir<'_>, x: Val, y: Val, neg_floor: Val) -> Result<Val, MaliError> {
    let floor = match vs_alu_of(vs, neg_floor) {
        Some((Alu::Fneg, [f, ..])) => f,
        _ => return Err(MaliError::FusedMultiplyAdd),
    };
    let p = vs.alu(Alu::Fmul, y, floor, Val::Undef)?;
    let n = vs.alu(Alu::Fneg, p, Val::Undef, Val::Undef)?;
    vs.alu(Alu::Fadd, x, n, Val::Undef)
}

/// A vertex `fadd` of the lanes `x` and `y` of the front end's sources
/// `srcs`: `fadd(fadd(a, b), #c)`, the inner sum used once and of no
/// constant, is `fadd(fadd(a, c), b)`, a product kept for `b`
/// (nir_opt_algebraic.py:2399-2400, "Prefer moving out a multiplication",
/// as nir_fs.rs's `lane32`). Measured: case 221's `mod(id, 2.0) - 0.5`,
/// `FADD_IMM.f32 r0, r0^, #0xBF000000` (`id - 0.5`) before the fused
/// product (vsfma_221_instmix.expect.json).
fn vs_fadd(vs: &mut Nir<'_>, m: &Module<'_>, srcs: &[ir::Src; 3], x: Val, y: Val) -> Result<Val, MaliError> {
    let once = |k: usize| super::nir_fs::ir_uses(m, srcs[k].val) == 1;
    let inner = match (x, y) {
        (o, Val::Const(_)) if once(0) => Some((o, y)),
        (Val::Const(_), o) if once(1) => Some((o, x)),
        _ => None,
    };
    let sum = inner.and_then(|(o, c)| match vs_alu_of(vs, o) {
        Some((Alu::Fadd, [p, q, _])) if !matches!(p, Val::Const(_)) && !matches!(q, Val::Const(_)) => Some((p, q, c)),
        _ => None,
    });
    match sum {
        Some((p, q, c)) => {
            let (a, b) = if !vs_is_fmul(vs, q) && vs_is_fmul(vs, p) { (q, p) } else { (p, q) };
            let s = vs.alu(Alu::Fadd, a, c, Val::Undef)?;
            vs.alu(Alu::Fadd, s, b, Val::Undef)
        }
        None => vs.alu(Alu::Fadd, x, y, Val::Undef),
    }
}

/// Each source of an IR operation of `width` lanes that NIR's rules on one
/// value cannot match while the operation is a vector: a source with a
/// constant lane that is not the same constant in every lane read
/// (nir_search.c `match_value` needs a `load_const` and compares each
/// lane, the sign of a zero too). `d * vec4(0.0, -0.0, 0.0, 0.0)` is
/// neither `fmul(a, 0.0)` nor `fmul(a, -0.0)` until the back end
/// scalarises it.
fn split_const(w: &Walk<'_>, s: &[ir::Src; 3], width: usize) -> [bool; 2] {
    [0, 1].map(|k| {
        let first = lane(w, s[k], 0);
        let lanes = || (0..width).map(|i| lane(w, s[k], i));
        lanes().any(|v| matches!(v, Val::Const(_))) && !lanes().all(|v| v == first && matches!(v, Val::Const(_)))
    })
}

/// One lane of an IR `fmul`. A zero of a `split` source ([`split_const`])
/// folds the lane only in the back end (`bi_optimize_loop`'s
/// `nir_lower_alu_width`, then `nir_opt_algebraic`'s `fmul(a, 0.0)` and
/// `fmul(a, -0.0)`), after Mesa numbers the attribute locations: the walk
/// folds it as the back end does, and notes it ([`build`]); the numbering
/// walk keeps the other factor read, in a node nothing folds.
fn vs_fmul(vs: &mut Nir<'_>, split: [bool; 2], a: Val, b: Val) -> Result<Val, MaliError> {
    let ab = [a, b];
    let late = (0..2).find(|&k| split[k] && is_zero(ab[k]) && !matches!(ab[1 - k], Val::Const(_)));
    if let Some(k) = late {
        if vs.numbering_walk {
            return Ok(Val::Lane(vs.push(Kind::Alu(Alu::Mov), [ab[1 - k], Val::Undef, Val::Undef])?, 0));
        }
        vs.late_fold = true;
    }
    let r = vs.alu(Alu::Fmul, a, b, Val::Undef)?;
    /* the back end's `fmul(a, 0.0)` and `fmul(a, -0.0)` on a lane of a
     * `load_const`: +0.0 either way (nir_opt_algebraic.py:277-278), not
     * the zero's sign the whole-vector rule keeps ([`Nir::alu`]) */
    Ok(if late.is_some() && matches!(r, Val::Const(_)) { Val::Const(F32_ZERO) } else { r })
}

/// "Propagate constants up multiplication chains" (nir_opt_algebraic.py:2393):
/// `fmul(fmul(a, b), #c)`, the inner product read once and of no constant,
/// is `fmul(fmul(a, c), b)`, the commutative operands tried in nir_search's
/// order (the outer `fmul` as written first, then the inner). Measured:
/// gm2_vsarith_4's `0.1 * (g * g)` (glmark2 function's `fract(3d) *
/// fract(3d)`, one `fract` after CSE) is `FMA.f32 r5, r4, 0x3DCCCCCD,
/// 0x0.neg` then `FMA.f32 r4, r5^, r4^, 0x0.neg`. None: not that shape.
fn vs_fmul_const_up(
    vs: &mut Nir<'_>,
    m: &Module<'_>,
    srcs: &[ir::Src; 3],
    x: Val,
    y: Val,
) -> Result<Option<Val>, MaliError> {
    for (k, inner, c) in [(0, x, y), (1, y, x)] {
        let Val::Const(_) = c else { continue };
        let Val::Lane(id, 0) = inner else { continue };
        let Some((Alu::Fmul, [a, b, _])) = vs_alu_of(vs, inner) else { continue };
        if matches!(a, Val::Const(_)) || matches!(b, Val::Const(_)) {
            continue;
        }
        if vs.uses(id).count() != 0 || !(ir_once(m, srcs[k].val) || ir_last_load(m, srcs[k])) {
            continue;
        }
        let ac = vs.alu(Alu::Fmul, a, c, Val::Undef)?;
        return vs.alu(Alu::Fmul, ac, b, Val::Undef).map(Some);
    }
    Ok(None)
}

/// `is_fmul` (nir_search_helpers.h:402-416): a product, or the `fneg` of
/// one.
fn vs_is_fmul(vs: &Nir<'_>, v: Val) -> bool {
    match vs_alu_of(vs, v) {
        Some((Alu::Fmul, _)) => true,
        Some((Alu::Fneg, [inner, ..])) => vs_is_fmul(vs, inner),
        _ => false,
    }
}

/// A front-end `ffma` is a matrix product's term (`mat_times_column` under
/// `gpu-mali`): the column a uniform load of a mat4, the scale a splat lane.
pub(super) fn is_mat_term(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let (col, comps) = match m.def(srcs[0].val) {
        Some(IrOp::LoadUniform { dword_off, comps }) => (*dword_off, u16::from(*comps)),
        _ => return false,
    };
    /* a mat2's or mat3's column too (vsmat3_highp's `u_in1 * vec3`: the
     * `FMA.f32` chain of each lane, the column's words in the FAU) */
    let is_mat = uniform_of(m, col)
        .is_ok_and(|(u, _)| matrix_cols(u.gl_type).is_some_and(|(_, rows, _)| rows == comps) && u.array_len <= 1);
    let splat = srcs[1].swizzle.iter().all(|&s| s == srcs[1].swizzle[0]);
    let acc = matches!(m.def(srcs[2].val), Some(IrOp::Alu(AluOp::Fmul | AluOp::Ffma, _)));
    is_mat && splat && acc
}

/// A matrix product's sum before its last term is one term over a
/// product, `ffma(c0, x, fmul(c1, y))` (`M * vec4(p.xy, 0.0, 1.0)`, the
/// `z` term gone), each read once (`is_used_once`: the sum has no user
/// yet, the product only the sum). Mesa moves the column 3 of a `w` of
/// 1.0 into it (S2 of docs/notes/mali-vs-constant-products.md:
/// `fadd(fadd(fmul, fmul), c3)` is `fadd(fmul(c1, y), fadd(fmul(c0, x),
/// c3))`, nir_opt_algebraic.py:4358-4360, then `ffma(c0, x, c3)` for every
/// lane before `ffma(c1, y, ..)`, gm2_arithconst_1.expect.json). Some:
/// the two terms `[c0, x, c1, y]`.
fn vs_two_terms(vs: &Nir<'_>, acc: Val) -> Option<[Val; 4]> {
    let users = |v: Val| match v {
        Val::Lane(n, 0) => vs.uses(n).count(),
        _ => usize::MAX,
    };
    let (Alu::Ffma, [c0, x, f]) = vs_alu_of(vs, acc)? else { return None };
    let (Alu::Fmul, [c1, y, _]) = vs_alu_of(vs, f)? else { return None };
    (users(acc) == 0 && users(f) == 1).then_some([c0, x, c1, y])
}

/// A dot product of `n` lanes of two constants, as Mesa's GLSL front end
/// folds the built-in call on constant expressions (ast_function.cpp:657-664,
/// `dot_f`, ir_constant_expression.cpp:45-54): `result += a[c] * b[c]` from
/// lane 0, each product and sum rounded on its own. NIR's `fdot3` folds the
/// same sum in the same order (nir_constant_expressions). `normalize` of a
/// constant (`normalize(LightDir.xyz)` of glmark2's `const vec4`,
/// gm2_arithconst_3/4) is this dot, then [`Nir::alu`]'s `frsq` and products
/// on constants. None: a lane is not a constant; a lane or the result that
/// is not a zero or a normal float is refused as the other folds are.
fn const_dot(n: usize, a: impl Fn(usize) -> Val, b: impl Fn(usize) -> Val) -> Result<Option<Val>, MaliError> {
    let mut sum = 0.0f32;
    for c in 0..n {
        let (Val::Const(x), Val::Const(y)) = (a(c), b(c)) else { return Ok(None) };
        if !plain(x) || !plain(y) {
            return Err(MaliError::ConstantFold);
        }
        sum += f32::from_bits(x) * f32::from_bits(y);
    }
    if !plain(sum.to_bits()) {
        return Err(MaliError::ConstantFold);
    }
    Ok(Some(Val::Const(sum.to_bits())))
}

/// IR value `v` is read once in the stage: by one instruction, or, a load
/// of a local, the only load of it (copy propagation hands Mesa's
/// `is_used_once` the value the local holds).
fn ir_once(m: &Module<'_>, v: ir::ValueId) -> bool {
    match m.def(v) {
        Some(IrOp::LoadLocal { local, .. }) => {
            let local = *local;
            super::nir_fs::ir_uses(m, v) == 1
                && m.instrs.iter().filter(|op| matches!(op, IrOp::LoadLocal { local: l, .. } if *l == local)).count()
                    == 1
        }
        _ => super::nir_fs::ir_uses(m, v) == 1,
    }
}

/// IR source `s` is a top-level load of a local read once, and no later
/// node reads the value it loads: the next top-level node that touches the
/// local is a store over every lane `s` reads (or there is none), with no
/// `if` or loop between (copy propagation hands Mesa's `is_used_once` the
/// value stored, which an earlier load of the local does not read:
/// gm2_vsarith_4's `d = process(d)` then `0.1 * d`). The walk's own count
/// of the value's earlier users is the caller's.
fn ir_last_load(m: &Module<'_>, s: ir::Src) -> bool {
    let Some(IrOp::LoadLocal { local, index: None, comps }) = m.def(s.val) else { return false };
    if super::nir_fs::ir_uses(m, s.val) != 1 {
        return false;
    }
    let read = s.swizzle[..usize::from((*comps).clamp(1, 4))].iter().fold(0u8, |a, &c| a | 1 << (c & 3));
    let top = m.nodes_of(m.body);
    let Some(at) = top.iter().position(|n| matches!(n, IrNode::Instr(id) if id.0 == s.val.0)) else { return false };
    for n in &top[at + 1..] {
        match *n {
            IrNode::Instr(id) => match m.instr(id) {
                Some(IrOp::LoadLocal { local: l, .. }) if l == local => return false,
                Some(IrOp::StoreLocal { local: l, index, write_mask, .. }) if l == local => {
                    return index.is_none() && write_mask & read == read
                }
                _ => {}
            },
            IrNode::Return => return true,
            _ => return false,
        }
    }
    true
}

/// A dot product of a vector `v * r` (`r` one value for every lane, as
/// `normalize`'s reciprocal square root) and a constant vector `L`
/// (gm2_arithconst_3/4: `dot(normalize(..), normalize(LightDir.xyz))` of
/// a `const` light): Mesa scalarises the dot into `fadd(fadd(fmul(N.z,
/// L.z), fmul(N.y, L.y)), fmul(N.x, L.x))` (nir_lower_alu_width.c
/// `lower_reduction`, the reverse order of an imprecise dot; the zero lanes
/// of `L` gone first, nir_opt_algebraic.py:559-581), turns each
/// `fmul(fmul(v, r), #L)` of a product read once into `fmul(fmul(v, L), r)`
/// (:2394) and takes `r` out of the sums (:246, `fmul(r, fadd(..))`), which
/// the late fusion makes `FMA v.z, L.z, -0.0`, `FMA v.y, L.y, ..`, `FMA
/// v.x, L.x, ..` and one product by `r` (gm2_arithconst_3.expect.json's
/// varying program). None: not that shape.
fn vs_dot_factored(
    vs: &mut Nir<'_>,
    m: &Module<'_>,
    w: &Walk<'_>,
    s: &[ir::Src; 3],
    n: usize,
) -> Result<Option<Val>, MaliError> {
    for k in 0..2 {
        let (pv, cv) = (|i| lane(w, s[k], i), |i| lane(w, s[1 - k], i));
        let mut lanes = [0usize; 4];
        let mut nl = 0;
        for i in 0..n {
            match cv(i) {
                Val::Const(c) if !plain(c) => return Ok(None),
                Val::Const(c) if c & !F32_NEG_ZERO == 0 => {}
                Val::Const(_) => {
                    lanes[nl] = i;
                    nl += 1;
                }
                _ => return Ok(None),
            }
        }
        if nl == 0 || !ir_once(m, s[k].val) {
            continue;
        }
        let mut terms = [(Val::Undef, Val::Undef); 4];
        let mut r = Val::Undef;
        for (t, &i) in terms.iter_mut().zip(&lanes[..nl]) {
            let p = pv(i);
            let Val::Lane(id, 0) = p else { return Ok(None) };
            let Some((Alu::Fmul, [x, y, _])) = vs_alu_of(vs, p) else { return Ok(None) };
            if vs.uses(id).count() != 0 || matches!(x, Val::Const(_)) || matches!(y, Val::Const(_)) {
                return Ok(None);
            }
            if r != Val::Undef && r != y {
                return Ok(None);
            }
            r = y;
            *t = (x, cv(i));
        }
        let (x, c) = terms[nl - 1];
        let mut acc = vs.alu(Alu::Fmul, x, c, Val::Undef)?;
        if nl == 1 {
            return vs.alu(Alu::Fmul, acc, r, Val::Undef).map(Some);
        }
        for &(x, c) in terms[..nl - 1].iter().rev() {
            acc = vs.alu(Alu::Ffma, x, c, acc)?;
        }
        return vs.alu(Alu::Fmul, r, acc, Val::Undef).map(Some);
    }
    Ok(None)
}

/// The column of the mat4 a matrix product's term ([`is_mat_term`]) reads,
/// 0 to 3.
fn mat_column(m: &Module<'_>, srcs: &[ir::Src; 3]) -> Option<u16> {
    match m.def(srcs[0].val) {
        Some(IrOp::LoadUniform { dword_off, .. }) => uniform_of(m, *dword_off)
            .ok()
            .and_then(|(u, _)| Some((*dword_off - u.dword_off) / matrix_cols(u.gl_type)?.2)),
        _ => None,
    }
}

/// The ALU node a vertex value is, with its sources.
fn vs_alu_of(vs: &Nir<'_>, v: Val) -> Option<(Alu, [Val; 3])> {
    match v {
        Val::Lane(n, 0) => match vs.nodes.get(n as usize) {
            Some(NirNode { kind: Kind::Alu(op), src, width: 1, .. }) => Some((*op, *src)),
            _ => None,
        },
        _ => None,
    }
}

/// `gl_type` is a `bool` or `bvecN` uniform's: a 32-bit word of 0 / ~0 in
/// Mesa's block (`UniformBooleanTrue`), with no precision.
pub(super) fn bool_type(gl_type: u32) -> bool {
    matches!(gl_type, GL_BOOL | GL_BOOL_VEC2 | GL_BOOL_VEC3 | GL_BOOL_VEC4)
}

/// The value is a lane of a uniform load as loaded (a FAU word): a `bool`
/// uniform's 0 / ~0 (`UniformBooleanTrue`, st_extensions.c:1356), which
/// `i2b` leaves as it is.
fn vs_is_uniform_word(vs: &Nir<'_>, v: Val) -> bool {
    matches!(v, Val::Lane(n, _) if matches!(vs.nodes.get(n as usize), Some(NirNode { kind: Kind::Uniform { .. }, .. })))
}

/// The value is a boolean: a compare's mask, a logic operation or `inot`
/// of booleans, `i2b`'s word of a bool uniform (what [`vs_is_uniform_word`]
/// leaves as it is), or a constant 0 / ~0.
fn vs_is_bool(vs: &Nir<'_>, v: Val) -> bool {
    match vs_alu_of(vs, v) {
        Some((Alu::Fcmp(_) | Alu::Icmp(_) | Alu::Ucmp(_) | Alu::Inot, _)) => true,
        Some((Alu::Iand | Alu::Ior | Alu::Ixor, [a, b, _])) => vs_is_bool(vs, a) && vs_is_bool(vs, b),
        Some((Alu::Bcsel, [_, a, b])) => vs_is_bool(vs, a) && vs_is_bool(vs, b),
        _ => matches!(v, Val::Const(0 | u32::MAX)) || vs_is_uniform_word(vs, v),
    }
}

/* ---- a mediump local's fp16 arithmetic in the vertex stage ------------ */

/// The value is a lane of a 16-bit node (a mediump local's narrowed lane,
/// or a lane of a two-lane fp16 operation on such lanes).
fn vs_is_f16(vs: &Nir<'_>, v: Val) -> bool {
    matches!(v, Val::Lane(n, _) if vs.nodes[n as usize].bits == 16)
}

/// An fp32 constant as the fp16 half lower_precision makes of it
/// (`_mesa_float_to_half`, round to nearest even, lower_precision.cpp:
/// 739-746); one fp16 cannot hold is refused.
fn half(c: u32) -> Result<u32, MaliError> {
    select::f16_rtne(c).map(u32::from).ok_or(MaliError::Fp16Inexact)
}

/// The fp32 bits of the fp16 value `c` rounds to: how a 16-bit constant
/// is kept in the walk's [`Val::Const`] (every constant of the walk is fp32
/// bits; [`half`] of this value is exact).
fn as_f16(c: u32) -> Result<u32, MaliError> {
    half(c).map(|h| select::f16_bits(h as u16))
}

/// `fmul` or `fadd` of two constants at 16 bits (`nir_opt_constant_folding`
/// of the `vec2` of constants in a two-lane fp16 operation), each step
/// rounded to fp16, as fp32 bits.
fn fold16(a: AluOp, p: u32, q: u32) -> Result<u32, MaliError> {
    let h = |b: u32| -> Result<f32, MaliError> { as_f16(b).map(f32::from_bits) };
    let r = if a == AluOp::Fmul { h(p)? * h(q)? } else { h(p)? + h(q)? };
    as_f16(r.to_bits())
}

/// The narrowing (`f2f16`) a mediump local's store makes of an integer's
/// float: GLSL's `f2fmp(i2f32(a))` is one `i2fmp` (nir_opt_algebraic.py:
/// 1322) that lowers to the two instructions in place, so the `f2f16`
/// stands right after the conversion in the order (nir_fs.rs
/// `narrow_stored`; deqp_intval_opvs_mul_ivec3_mediump: `S32_TO_F32 r2`
/// then `FADD.f32 r2.h0` for each lane in turn). CSE'd.
fn vs_narrow_stored(vs: &mut Nir<'_>, v: Val) -> Result<Val, MaliError> {
    let u = Val::Undef;
    let nd = NirNode { kind: Kind::Alu(Alu::F2f16), src: [v, u, u], bits: 16, width: 1 };
    if let Some(i) = vs.nodes[..vs.n].iter().position(|x| *x == nd) {
        return Ok(Val::Lane(i as u16, 0));
    }
    let Val::Lane(n, _) = v else { return Err(MaliError::Defect(Defect::IrIndex)) };
    let after = vs.order[..vs.len].iter().position(|&x| x == n).map(|p| p + 1);
    let id = match after {
        Some(k) if k < vs.len => vs.insert_before(k, nd)?,
        _ => vs.push_node(nd)?,
    };
    Ok(Val::Lane(id, 0))
}

/// A 16-bit lane read by a 32-bit operation, an output or a highp local is
/// widened first (`f2f32`, `F16_TO_F32 a.hN`), as GLSL's lower_precision
/// converts a mediump value where a highp expression reads it.
fn vs_widen(vs: &mut Nir<'_>, v: Val) -> Result<Val, MaliError> {
    if vs_is_f16(vs, v) {
        vs.alu(Alu::F2f32, v, Val::Undef, Val::Undef)
    } else {
        Ok(v)
    }
}

/// A lane of a mediump local's load that holds a highp `fadd` (32 bits):
/// lower_precision's `f2fmp` at the assignment, which stays where a 16-bit
/// operation reads it and folds away where a 32-bit one does (`f2f32(f2fmp(a))`
/// is `a`, nir_opt_algebraic.py; deqp_colourpair_ck_lane stores the sum as
/// it is). deqp_posvary_fract_highp_*: `color.yz = res` of `fract`'s `fadd`,
/// read by `color * 0.80`.
fn vs_stored_sum(vs: &Nir<'_>, m: &Module<'_>, s: ir::Src, v: Val) -> bool {
    matches!(m.def(s.val), Some(IrOp::LoadLocal { local, .. }) if m.local_prec_of(*local) == Prec::Medium)
        && matches!(v, Val::Lane(n, 0) if matches!(vs.nodes[n as usize],
            NirNode { kind: Kind::Alu(Alu::Fadd), bits: 32, width: 1, .. }))
}

/// A two-lane 16-bit node, CSE'd (`nir_opt_cse`).
fn vs_node16(vs: &mut Nir<'_>, op: Alu, src: [Val; 3]) -> Result<u16, MaliError> {
    let nd = NirNode { kind: Kind::Alu(op), src, bits: 16, width: 2 };
    if let Some(i) = vs.nodes[..vs.n].iter().position(|x| *x == nd) {
        return Ok(i as u16);
    }
    vs.push_node(nd)
}

/// The halves `x` and `y` as one fp16 pair: the two-lane node they are
/// lanes 0 and 1 of, else their `vec2` (`MKVEC.v2i16 x.h0, y.h0`; with a
/// constant half, `MKVEC.v2i16 0x0.h0, x.h0` or `U16_TO_U32 x.h0` for
/// `vec2(x, 0.0)`, nir_fs.rs `vec16_of`). A constant is its fp16 half.
fn vs_vec16(vs: &mut Nir<'_>, x: Val, y: Val) -> Result<Val, MaliError> {
    if let (Val::Lane(n, 0), Val::Lane(m, 1)) = (x, y) {
        if n == m && vs.nodes[n as usize].width == 2 {
            return Ok(Val::Lane(n, 0));
        }
    }
    /* lanes c and c + 1 of an fp16 look-up are its register c / 2, read
     * whole by a two-lane operation ([`vs_tex16_word`]; gm2_texvert_3's
     * `FMA.v2f16 r0, r0^, ..` of the texels' x and y) */
    if let (Val::Lane(n, c), Val::Lane(m, d)) = (x, y) {
        if n == m && c % 2 == 0 && d == c + 1 && vs_is_word16(vs, n) {
            return Ok(Val::Lane(n, c));
        }
    }
    let h = |v: Val| match v {
        Val::Const(c) => half(c).map(Val::Const),
        v => Ok(v),
    };
    let (x, y) = (h(x)?, h(y)?);
    Ok(Val::Lane(vs_node16(vs, Alu::Vec2, [x, y, Val::Undef])?, 0))
}

/// The source (0 or 1) of an fp16 `fadd` whose every lane is 0.0: the
/// operation is its other source (`fadd(a, 0.0)`, nir_opt_algebraic.py,
/// which GLSL IR's `ir_algebraic` already folds in `res = vec4(0.0); res
/// += u_arr[0]`: deqp_uniarray_*_const_vs's first sum is `u_arr[0] +
/// u_arr[1]`, `FADD.v2f16 r0, u5.w1, r0^`). None for any other.
fn vs_identity16(op: Alu, x: &[[Val; 4]], width: usize) -> Option<usize> {
    if op != Alu::Fadd || width == 0 {
        return None;
    }
    (0..2).find(|&k| {
        x[k][..width].iter().all(|&v| v == Val::Const(F32_ZERO))
            && x[1 - k][..width].iter().all(|&v| !matches!(v, Val::Const(_)))
    })
}

/// Node `n` is the `f2f32` of a [`uniform_sum16`] that reads an fp16 array
/// element at a run-time index ([`uniform_idx16`]): a varying lane of
/// uniforms alone, convergent, which Mesa's link makes flat.
fn widened_sum16(vs: &Nir<'_>, n: u16) -> bool {
    match vs.nodes[n as usize] {
        NirNode { kind: Kind::Alu(Alu::F2f32), src: [Val::Lane(x, _), ..], .. } => {
            uniform_sum16(vs, x) && vs.reads_indexed(x) && !vs.reads_attribute(x)
        }
        _ => false,
    }
}

/// Node `n` is fp16 arithmetic of fp16 uniform loads alone (a mediump
/// array's `res += u_arr[k]`, deqp_uniarray_*_const_vs; a mediump matrix's
/// sums and products): its `f2f32` at the store
/// hides the constant lanes beside it from Mesa's link (`kept`, the
/// `StoreOutput` arm of [`walk`]).
fn uniform_sum16(vs: &Nir<'_>, n: u16) -> bool {
    let nd = vs.nodes[n as usize];
    if nd.bits != 16 {
        return false;
    }
    /* fp16 arithmetic of the uniforms alone: their sums, negations
     * (deqp_unitype_vsmat3_lowp's `u_in0 - u_in1`), products and fmas
     * (deqp_misc_spill_*'s `u_in0 * u_in1`), constants among the operands */
    let of = |v: &Val| match *v {
        Val::Lane(x, _) | Val::Pair(x, _) => uniform_sum16(vs, x),
        Val::Const(_) => true,
        _ => false,
    };
    match nd.kind {
        /* an fp16 array element read at a run-time index
         * (deqp_uniarray_dyn_vec*_vs's `res += u_arr[ui_one]`,
         * [`uniform_idx16`]) */
        Kind::Uniform { .. } | Kind::UniformIdx { .. } => true,
        Kind::Alu(op @ (Alu::Fadd | Alu::Fmul | Alu::Ffma | Alu::Fneg | Alu::Vec2)) => {
            nd.src[..op.arity()].iter().all(of) && nd.src[..op.arity()].iter().any(|v| !matches!(v, Val::Const(_)))
        }
        _ => false,
    }
}

/// The lanes of a `fmul` / `fadd` the front end classed mediump are all
/// constants or 16-bit lanes, one at least a lane: the operation is fp16
/// ([`vs_alu16`]). Another operand (an fp32 lane: a highp value, a boolean's
/// float, which lower_precision does not narrow) keeps the fp32 walk.
fn vs_pairs16(vs: &Nir<'_>, m: &Module<'_>, w: &Walk<'_>, s: &[ir::Src; 3], width: usize) -> bool {
    let mut any = false;
    for k in 0..2 {
        for i in 0..width {
            match lane(w, s[k], i) {
                Val::Const(_) => {}
                v if vs_is_f16(vs, v) => any = true,
                /* narrowed where read ([`vs_stored_sum`]) */
                v if vs_stored_sum(vs, m, s[k], v) => any = true,
                _ => return false,
            }
        }
    }
    any && width.is_multiple_of(2)
}

/// `fmul` or `fadd` of a mediump local's lanes in the vertex stage, as
/// Mesa's NIR holds it: lower_precision computes the vector at 16 bits
/// (its constants fp16), `nir_lower_alu_width` with `bi_vectorize_filter`
/// cuts it into the operations of its aligned lane pairs (width two at 16
/// bits), and a pair whose four operands are constants folds, each step
/// rounded to fp16 (deqp_intval_opvs_add_int_highp: `vec4(float(res), 0.0,
/// 0.0, 1.0) * 0.1 + 0.5`, the pair x, y `U16_TO_U32 r0, r0^.h0` then
/// `FMA.v2f16 r0, r0^, 0x2E660000.h11, 0x38000000.h11` once [`fuse`] joins
/// the two, the pair z, w the constants 0.5 and 0x38cd). The constant side
/// is one fp16 in both halves; two different constants against a pair have
/// no reference. The late `fadd(fmul(a, b), c)` fusion is [`fuse`]'s.
fn vs_alu16(
    vs: &mut Nir<'_>,
    a: AluOp,
    m: &Module<'_>,
    w: &Walk<'_>,
    s: &[ir::Src; 3],
    width: usize,
    out: &mut [Val; 4],
) -> Result<(), MaliError> {
    let u = Val::Undef;
    let mut x = [[u; 4]; 2];
    for k in 0..2 {
        for i in 0..width {
            x[k][i] = lane(w, s[k], i);
            /* the `f2f16` right after the sum ([`vs_narrow_stored`]) */
            if vs_stored_sum(vs, m, s[k], x[k][i]) {
                x[k][i] = vs_narrow_stored(vs, x[k][i])?;
            }
        }
    }
    let konst = |v: Val| matches!(v, Val::Const(_));
    /* the constant side second (both commute) */
    if x[0][..width].iter().all(|&v| konst(v)) && !x[1][..width].iter().all(|&v| konst(v)) {
        x.swap(0, 1);
    }
    let op = if a == AluOp::Fmul { Alu::Fmul } else { Alu::Fadd };
    if let Some(k) = vs_identity16(op, &x, width) {
        out[..width].copy_from_slice(&x[1 - k][..width]);
        vs.note_user(Prod::Identity);
        return Ok(());
    }
    for k in (0..width).step_by(2) {
        let (p, q) = (x[0][k], x[0][k + 1]);
        let (c, d) = (x[1][k], x[1][k + 1]);
        match (p, q, c, d) {
            (Val::Const(p), Val::Const(q), Val::Const(c), Val::Const(d)) => {
                out[k] = Val::Const(fold16(a, p, c)?);
                out[k + 1] = Val::Const(fold16(a, q, d)?);
                vs.note_user(Prod::Folded);
            }
            /* `fmul(a, 1.0)` is `a` (nir_opt_algebraic.py), on the pair
             * `nir_lower_alu_width` cut (gm2_texvert_4: `t * vec4(0.5, 1.0,
             * 1.0, 1.0)` leaves z and w the texels themselves) */
            (Val::Lane(..), Val::Lane(..), Val::Const(a), Val::Const(b))
                if a == F32_ONE && b == F32_ONE && op == Alu::Fmul =>
            {
                out[k] = p;
                out[k + 1] = q;
                vs.note_user(Prod::Identity);
            }
            _ => {
                let v = vs_vec16(vs, p, q)?;
                let kk = match (c, d) {
                    (Val::Const(a), Val::Const(b)) if a == b => Val::Const(half(a)?),
                    /* two constants: one word, the halves in lane order
                     * (gm2_texvert_4's `FMA.v2f16 r0, r0^, u6.w0, 0x0.neg`,
                     * u6.w0 = 0x3C003800 for (0.5, 1.0)); a high half of 0
                     * would read as one half in both, which has no
                     * reference */
                    (Val::Const(a), Val::Const(b)) if half(b)? != 0 => Val::Const(half(a)? | (half(b)? << 16)),
                    (Val::Const(_), _) | (_, Val::Const(_)) => return Err(MaliError::Fp16Inexact),
                    _ => vs_vec16(vs, c, d)?,
                };
                let n = vs_node16(vs, op, [v, kk, u])?;
                out[k] = Val::Lane(n, 0);
                out[k + 1] = Val::Lane(n, 1);
            }
        }
    }
    Ok(())
}

/// The 16-bit form of an operand lane of a mediump operation over fp16
/// uniforms ([`vs_uniform16`]): a 16-bit lane itself, an fp16 uniform's
/// lane under the `f2f32` its load gave it (the lane read at 16 bits, the
/// `f2f32` left to [`dce`]), or with `narrow` a 32-bit attribute lane,
/// narrowed (`f2fmp`, `FADD.f32 r0.h0, r0^, 0x0.neg`). None for any other.
fn vs_lane16(vs: &Nir<'_>, v: Val, narrow: bool) -> Option<Val> {
    if vs_is_f16(vs, v) {
        return Some(v);
    }
    let Val::Lane(n, c) = v else { return None };
    match vs.nodes[n as usize] {
        NirNode { kind: Kind::Alu(Alu::F2f32), src: [s, ..], .. } if vs_is_f16(vs, s) => Some(s),
        NirNode { kind: Kind::Input { .. }, bits: 32, .. } if narrow => Some(Val::Lane(n, c)),
        _ => None,
    }
}

/// The lane is an fp16 uniform's ([`Kind::Uniform`] at 16 bits, or an
/// fp16 array element read at a run-time index, [`uniform_idx16`]), read
/// through its `f2f32` or not.
fn vs_is_uniform16(vs: &Nir<'_>, v: Val) -> bool {
    matches!(vs_lane16(vs, v, false), Some(Val::Lane(n, _))
        if matches!(vs.nodes[n as usize], NirNode { kind: Kind::Uniform { .. } | Kind::UniformIdx { .. }, bits: 16, .. }))
}

/// The lanes of an `fmul` / `fadd` / `ffma` that are computed: those up to
/// the first one an operand leaves undefined (a mat3 or mat2 column in the
/// front end's four-lane matrix product, deqp_unitype_vsmat3_mediump).
fn vs_width16(w: &Walk<'_>, s: &[ir::Src; 3], n: usize, width: usize) -> usize {
    (0..width).take_while(|&i| (0..n).all(|k| lane(w, s[k], i) != Val::Undef)).count()
}

/// A `fmul` / `fadd` / `ffma` / `fneg` the front end classed mediump whose
/// operands are an fp16 uniform's lanes, mediump attributes' lanes, 16-bit
/// lanes and constants, one at least an fp16 uniform's, or 16-bit lanes and
/// constants alone at a width [`vs_pairs16`] does not take (an odd one):
/// Mesa computes it at 16 bits (lower_precision with `fp16_const_buffers`
/// keeps the uniform fp16 and narrows the attribute where the expression
/// reads it), [`vs_alu16u`].
fn vs_uniform16(vs: &Nir<'_>, m: &Module<'_>, w: &Walk<'_>, a: AluOp, s: &[ir::Src; 3], width: usize) -> bool {
    let n = super::nir_fs::ir_arity(a);
    let wd = vs_width16(w, s, n, width);
    let (mut uni, mut half, mut any) = (false, true, false);
    for k in 0..n {
        let narrow = m.prec_of(s[k].val) == Prec::Medium;
        for i in 0..wd {
            let v = lane(w, s[k], i);
            match v {
                Val::Const(_) => {}
                v if vs_lane16(vs, v, narrow).is_some() => {
                    uni |= vs_is_uniform16(vs, v);
                    half &= vs_is_f16(vs, v);
                    any = true;
                }
                _ => return false,
            }
        }
        /* a pair of an fp16 uniform's lanes is its first word or one of
         * its first two lanes twice ([`vs_pair16`]); a later word (a vec4's
         * or a mat4 column's z and w) has no form here and keeps the fp32
         * walk (deqp_misc_spill_*'s `u_in0 * u_in1` of two mat4) */
        for c in (0..wd.saturating_sub(1)).step_by(2) {
            let (p, q) = (lane(w, s[k], c), lane(w, s[k], c + 1));
            if !vs_is_uniform16(vs, p) && !vs_is_uniform16(vs, q) {
                continue;
            }
            let drawn = matches!((vs_lane16(vs, p, false), vs_lane16(vs, q, false)),
                (Some(Val::Lane(a, i)), Some(Val::Lane(b, j))) if a == b && i % 2 == 0 && (j == i + 1 || i == j)
                    || a == b && i == j);
            if !drawn {
                return false;
            }
        }
    }
    wd > 0 && (uni || (half && any))
}

/// Source `s` is mediump: a mediump value, or a load of a mediump local
/// (`in0 = a_in0.wy` of a `mediump vec2`, whose load the front end leaves
/// unclassed).
fn medium_src(m: &Module<'_>, s: ir::Src) -> bool {
    m.prec_of(s.val) == Prec::Medium
        || matches!(m.def(s.val), Some(IrOp::LoadLocal { local, .. }) if m.local_prec_of(*local) == Prec::Medium)
}

/// An `abs` the front end classed mediump whose computed lanes are 16-bit
/// lanes or, through a mediump source, 32-bit attribute lanes: Mesa
/// computes it at 16 bits ([`vs_fabs16`]).
fn vs_abs16(vs: &Nir<'_>, m: &Module<'_>, w: &Walk<'_>, s: &[ir::Src; 3], width: usize) -> bool {
    let narrow = medium_src(m, s[0]);
    let wd = vs_width16(w, s, 1, width);
    wd > 0 && (0..wd).all(|i| vs_lane16(vs, lane(w, s[0], i), narrow).is_some())
}

/// [`vs_abs16`]'s `abs` as Mesa's NIR holds it: each lane narrowed alone
/// (`f2f16`, `FADD.f32 r0.h0, r3^, 0x0.neg`) and its `fabs` an operation of
/// one 16-bit lane, which the back end makes `FADD.v2f16 r0, r0^.abs.h00,
/// 0x0.neg` (deqp_varylink_op_abs_vec2: `abs(a_in0.wy)`, lane x then lane y,
/// each before the pair of the colour that reads it). CSE'd.
fn vs_fabs16(
    vs: &mut Nir<'_>,
    m: &Module<'_>,
    w: &Walk<'_>,
    s: &[ir::Src; 3],
    width: usize,
    out: &mut [Val; 4],
) -> Result<(), MaliError> {
    let u = Val::Undef;
    let narrow = medium_src(m, s[0]);
    let wd = vs_width16(w, s, 1, width);
    for (i, o) in out.iter_mut().enumerate().take(wd) {
        let x = match vs_lane16(vs, lane(w, s[0], i), narrow).ok_or(MaliError::Defect(Defect::IrIndex))? {
            l if vs_is_f16(vs, l) => l,
            l @ Val::Lane(..) => {
                let nd = NirNode { kind: Kind::Alu(Alu::F2f16), src: [l, u, u], bits: 16, width: 1 };
                match vs.nodes[..vs.n].iter().position(|y| *y == nd) {
                    Some(k) => Val::Lane(k as u16, 0),
                    None => Val::Lane(vs.push_node(nd)?, 0),
                }
            }
            _ => return Err(MaliError::Defect(Defect::IrIndex)),
        };
        let nd = NirNode { kind: Kind::Alu(Alu::Fabs), src: [x, u, u], bits: 16, width: 1 };
        let id = match vs.nodes[..vs.n].iter().position(|y| *y == nd) {
            Some(k) => k as u16,
            None => vs.push_node(nd)?,
        };
        *o = Val::Lane(id, 0);
    }
    Ok(())
}

/// A 16-bit node of one lane or two, CSE'd (`nir_opt_cse`).
fn vs_node16w(vs: &mut Nir<'_>, op: Alu, src: [Val; 3], width: u8) -> Result<u16, MaliError> {
    let nd = NirNode { kind: Kind::Alu(op), src, bits: 16, width };
    if let Some(i) = vs.nodes[..vs.n].iter().position(|x| *x == nd) {
        return Ok(i as u16);
    }
    vs.push_node(nd)
}

/// A mediump `fract` whose operand [`vs_abs16`] takes, as Mesa's NIR holds
/// it (deqp_posvary_fract_{mediump,lowp}_{vec2,vec3}: `res = fract(in0)` of
/// a `mediump vec2 in0 = a_in0.wy` or `vec3 in0 = a_in0.zxy`):
/// `lower_ffract`'s `fadd(a, fneg(ffloor(a)))`
/// on the vector at 16 bits, `a` the attribute's narrowed lanes
/// ([`vs_narrow_input`]). `nir_lower_alu_width` cuts each operation into its
/// aligned lane pairs, a pair whose two lanes are not one two-lane value's
/// (`alu_is_swizzled_in_bounds`) into its lanes, the cut pair's results a
/// `vec2`; `nir_opt_vectorize` then joins two `ffloor`s of one two-lane
/// value's lanes 0 and 1 (`FROUND.v2f16.rtn r3, r0` of `vec3`'s x and y).
/// The `fadd` of a cut pair reads the `ffloor` pair's `vec2` lane by lane
/// (`FADD.v2f16 r1, r1^.h00, r2.neg.h00`, `r0^.h00, r2^.neg.h11`).
fn vs_ffract16(
    vs: &mut Nir<'_>,
    m: &Module<'_>,
    w: &Walk<'_>,
    s: &[ir::Src; 3],
    width: usize,
    out: &mut [Val; 4],
) -> Result<(), MaliError> {
    let u = Val::Undef;
    let narrow = medium_src(m, s[0]);
    let wd = vs_width16(w, s, 1, width);
    let mut x = [u; 4];
    for (i, xi) in x.iter_mut().enumerate().take(wd) {
        *xi = vs_lane16(vs, lane(w, s[0], i), narrow).ok_or(MaliError::Defect(Defect::IrIndex))?;
    }
    /* the attribute's lanes narrowed in lane order: an aligned pair both
     * lanes of which are read is one `V2F32_TO_V2F16`
     * ([`vs_narrow_input`]); a pair one lane of which is read is shrunk to
     * that lane (`nir_opt_shrink_vectors`, `FADD.f32 r0.h0, r1^, 0x0.neg`
     * of `a_in0.wy`'s y) */
    let mut read = [0u8; MAX_NODES];
    let mut inputs = [0u16; 4];
    let mut ni = 0;
    for &v in &x[..wd] {
        match v {
            Val::Lane(n, c) if !vs_is_f16(vs, v) => {
                if read[n as usize] == 0 {
                    inputs[ni] = n;
                    ni += 1;
                }
                read[n as usize] |= 1 << c;
            }
            v if vs_is_f16(vs, v) => {}
            _ => return Err(MaliError::Defect(Defect::IrIndex)),
        }
    }
    let mut narrowed = [[u; 4]; 4];
    for (k, &n) in inputs[..ni].iter().enumerate() {
        let mask = read[n as usize];
        for lo in (0..4u8).step_by(2) {
            let pair = mask >> lo & 3;
            if pair == 3 {
                narrowed[k][lo as usize] = vs_narrow_input(vs, n, lo)?;
                narrowed[k][lo as usize + 1] = vs_narrow_input(vs, n, lo + 1)?;
                continue;
            }
            for c in lo..lo + 2 {
                if pair >> (c - lo) & 1 != 0 {
                    let id = vs_node16w(vs, Alu::F2f16, [Val::Lane(n, c), u, u], 1)?;
                    narrowed[k][c as usize] = Val::Lane(id, 0);
                }
            }
        }
    }
    for xi in x[..wd].iter_mut() {
        if let Val::Lane(n, c) = *xi {
            if !vs_is_f16(vs, *xi) {
                let k = inputs[..ni].iter().position(|&i| i == n).ok_or(MaliError::Defect(Defect::IrIndex))?;
                *xi = narrowed[k][c as usize];
            }
        }
    }
    /* a pair of lanes is one two-lane value's lanes 0 and 1 */
    let whole = |vs: &Nir<'_>, p: Val, q: Val| matches!((p, q), (Val::Lane(a, 0), Val::Lane(b, 1)) if a == b && vs.nodes[a as usize].width == 2);
    /* the `ffloor`s: a whole pair one node, a cut pair's lanes alone; a
     * lane alone whose neighbour in its two-lane source is floored alone
     * too is joined with it (`nir_opt_vectorize`) */
    let mut fl = [u; 4];
    let mut cut = [false; 2];
    for k in (0..wd).step_by(2) {
        if k + 1 < wd && whole(vs, x[k], x[k + 1]) {
            let n = vs_node16w(vs, Alu::Ffloor, [x[k], u, u], 2)?;
            fl[k] = Val::Lane(n, 0);
            fl[k + 1] = Val::Lane(n, 1);
        } else {
            cut[k / 2] = k + 1 < wd;
            for i in k..(k + 2).min(wd) {
                fl[i] = u;
            }
        }
    }
    for i in 0..wd {
        if fl[i] != u {
            continue;
        }
        /* the lane's partner: lane 1 of the same two-lane value at another
         * lane of the vector, also cut */
        let partner = (0..wd).find(|&j| {
            j != i
                && fl[j] == u
                && matches!((x[i], x[j]), (Val::Lane(a, 0), Val::Lane(b, 1)) if a == b && vs.nodes[a as usize].width == 2)
        });
        match partner {
            Some(j) => {
                let n = vs_node16w(vs, Alu::Ffloor, [x[i], u, u], 2)?;
                fl[i] = Val::Lane(n, 0);
                fl[j] = Val::Lane(n, 1);
            }
            None => {
                let n = vs_node16w(vs, Alu::Ffloor, [x[i], u, u], 1)?;
                fl[i] = Val::Lane(n, 0);
            }
        }
    }
    /* the `fadd`s, pair by pair as the `ffloor`s were cut */
    for k in (0..wd).step_by(2) {
        if k + 1 < wd && !cut[k / 2] {
            let Val::Lane(f, _) = fl[k] else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let ng = vs_node16w(vs, Alu::Fneg, [Val::Lane(f, 0), u, u], 2)?;
            let n = vs_node16w(vs, Alu::Fadd, [x[k], Val::Lane(ng, 0), u], 2)?;
            out[k] = Val::Lane(n, 0);
            out[k + 1] = Val::Lane(n, 1);
        } else if k + 1 < wd {
            let v = vs_vec16(vs, fl[k], fl[k + 1])?;
            let Val::Lane(vn, _) = v else { return Err(MaliError::Defect(Defect::IrIndex)) };
            let ng = vs_node16w(vs, Alu::Fneg, [Val::Lane(vn, 0), u, u], 2)?;
            for i in 0..2 {
                let n = vs_node16w(vs, Alu::Fadd, [x[k + i], Val::Lane(ng, i as u8), u], 1)?;
                out[k + i] = Val::Lane(n, 0);
            }
        } else {
            let ng = vs_node16w(vs, Alu::Fneg, [fl[k], u, u], 1)?;
            let n = vs_node16w(vs, Alu::Fadd, [x[k], Val::Lane(ng, 0), u], 1)?;
            out[k] = Val::Lane(n, 0);
        }
    }
    Ok(())
}

/// Lane `c` of attribute load `n` narrowed for a 16-bit reader: the
/// `f2fmp` of the whole vector lower_precision makes at the first read,
/// cut by `nir_lower_alu_width` into its aligned pairs and an odd lane
/// alone, in lane order (deqp_unitype_vsmat3_mediump's `vec3`:
/// `V2F32_TO_V2F16 r0, r0^, r1^` for x and y, then `FADD.f32 r1.h0, r2^,
/// 0x0.neg` for z, both before the first product), CSE'd; the lanes no
/// 16-bit operation reads are left to [`dce`].
fn vs_narrow_input(vs: &mut Nir<'_>, n: u16, c: u8) -> Result<Val, MaliError> {
    let u = Val::Undef;
    let Kind::Input { comps, .. } = vs.nodes[n as usize].kind else {
        return Err(MaliError::Defect(Defect::IrIndex));
    };
    let mut got = Val::Undef;
    for lo in (0..comps).step_by(2) {
        let (src, width) = if lo + 1 < comps {
            ([Val::Lane(n, lo), Val::Lane(n, lo + 1), u], 2)
        } else {
            ([Val::Lane(n, lo), u, u], 1)
        };
        let nd = NirNode { kind: Kind::Alu(Alu::F2f16), src, bits: 16, width };
        let f = match vs.nodes[..vs.n].iter().position(|y| *y == nd) {
            Some(i) => i as u16,
            None => vs.push_node(nd)?,
        };
        if c & !1 == lo {
            got = Val::Lane(f, c - lo);
        }
    }
    Ok(got)
}

/// [`vs_uniform16`]'s operation as Mesa's NIR holds it: the vector at 16
/// bits cut into its aligned lane pairs and an odd lane alone
/// (`nir_lower_alu_width` with `bi_vectorize_filter`), each pair's
/// operand an fp16 uniform's word read whole (`u5.w1`), one 16-bit lane in
/// both halves (`r0.h00`, the narrowed attribute of `u_in0 + a_coords`), a
/// two-lane node read whole, a constant's half, or else the `vec2` of its
/// two lanes; a lane alone reads its half (`r1.h11`, `u7.w0.h00`).
/// deqp_unitype_vsmat2_mediump: `FADD.v2f16 r1, u5.w1, r0.h00` per column,
/// `FADD.v2f16 r2, r1.h11, r0.h11` for `res[0][1] + res[1][1]`;
/// deqp_unitype_vsmat3_mediump: the front end's `fmul` of column 1 and
/// `ffma`s of columns 0 and 2, each `FMA.v2f16` of x, y then of z alone;
/// deqp_unitype_vsmat3_lowp: the `fneg` of `u_in0 - u_in1`, the `.neg` of
/// the sum's source.
fn vs_alu16u(
    vs: &mut Nir<'_>,
    a: AluOp,
    m: &Module<'_>,
    w: &Walk<'_>,
    s: &[ir::Src; 3],
    width: usize,
    out: &mut [Val; 4],
) -> Result<(), MaliError> {
    let u = Val::Undef;
    let n = super::nir_fs::ir_arity(a);
    let width = vs_width16(w, s, n, width);
    let mut x = [[u; 4]; 3];
    for k in 0..n {
        let narrow = m.prec_of(s[k].val) == Prec::Medium;
        for i in 0..width {
            x[k][i] = match lane(w, s[k], i) {
                c @ Val::Const(_) => c,
                v => match vs_lane16(vs, v, narrow).ok_or(MaliError::Defect(Defect::IrIndex))? {
                    l if vs_is_f16(vs, l) => l,
                    Val::Lane(i, c) => vs_narrow_input(vs, i, c)?,
                    _ => return Err(MaliError::Defect(Defect::IrIndex)),
                },
            };
        }
    }
    let op = match a {
        AluOp::Fmul => Alu::Fmul,
        AluOp::Ffma => Alu::Ffma,
        AluOp::Fneg => Alu::Fneg,
        _ => Alu::Fadd,
    };
    let one = |v: Val| match v {
        Val::Const(c) => half(c).map(Val::Const),
        v => Ok(v),
    };
    if let Some(k) = vs_identity16(op, &x, width) {
        out[..width].copy_from_slice(&x[1 - k][..width]);
        vs.note_user(Prod::Identity);
        return Ok(());
    }
    for k in (0..width).step_by(2) {
        if k + 1 == width {
            let mut src = [u; 3];
            for (j, sj) in src.iter_mut().enumerate().take(n) {
                *sj = one(x[j][k])?;
            }
            let nd = NirNode { kind: Kind::Alu(op), src, bits: 16, width: 1 };
            let id = match vs.nodes[..vs.n].iter().position(|y| *y == nd) {
                Some(i) => i as u16,
                None => vs.push_node(nd)?,
            };
            out[k] = Val::Lane(id, 0);
            continue;
        }
        let mut src = [u; 3];
        for (j, sj) in src.iter_mut().enumerate().take(n) {
            *sj = vs_pair16(vs, x[j][k], x[j][k + 1])?;
        }
        if src[..n].iter().all(|v| matches!(v, Val::Const(_))) {
            return Err(MaliError::Defect(Defect::IrIndex));
        }
        let n = vs_node16(vs, op, src)?;
        out[k] = Val::Lane(n, 0);
        out[k + 1] = Val::Lane(n, 1);
    }
    Ok(())
}

/// Two 16-bit lanes as one operand of a two-lane fp16 operation
/// ([`vs_alu16u`]): an fp16 uniform's lanes 0 and 1 are its first word
/// ([`Val::Pair`] `H01`), one lane twice its half in both
/// ([`Val::Pair`] `H00` / `H11`), a constant twice its half; anything else
/// [`vs_vec16`].
fn vs_pair16(vs: &mut Nir<'_>, p: Val, q: Val) -> Result<Val, MaliError> {
    match (p, q) {
        (Val::Const(a), Val::Const(b)) if a == b => Ok(Val::Const(half(a)?)),
        /* a later word of an fp16 uniform (a vec4's or a mat4 column's z
         * and w): that word's own node, read as a first word is
         * (deqp_unitype_fsivec4's `vMatrix * vTest`: `FMA.v2f16 r2, u7.w0,
         * r0.h11, 0x0.neg` of column 1's zw, `MOV.i32 r2, u5.w0` of
         * `vTest.zw` read by its halves) */
        (Val::Lane(n, c), Val::Lane(m, d))
            if n == m
                && c >= 2
                && (d == c + 1 && c % 2 == 0 || c == d)
                && matches!(vs.nodes[n as usize], NirNode { kind: Kind::Uniform { .. }, bits: 16, .. }) =>
        {
            let Kind::Uniform { dword, comps } = vs.nodes[n as usize].kind else {
                return Err(MaliError::Defect(Defect::IrIndex));
            };
            let w = c & !1;
            let k = vs.load16(Kind::Uniform { dword: dword + u16::from(w), comps: (comps - w).min(2) })?;
            vs_pair16(vs, Val::Lane(k, c - w), Val::Lane(k, d - w))
        }
        (Val::Lane(n, c), Val::Lane(m, d)) if n == m => {
            let nd = vs.nodes[n as usize];
            let uniform = matches!(nd.kind, Kind::Uniform { .. });
            match (c, d) {
                (0, 1) if uniform => Ok(Val::Pair(n, Swz::H01)),
                (c, d) if c == d && c < 2 && (uniform || nd.width <= 2) => {
                    Ok(Val::Pair(n, if c == 0 { Swz::H00 } else { Swz::H11 }))
                }
                _ => vs_vec16(vs, p, q),
            }
        }
        _ => vs_vec16(vs, p, q),
    }
}

/// Node `n` is an fp16 vertex look-up ([`Kind::VsTex`] at 16 bits).
pub(super) fn vs_is_tex16(vs: &Nir<'_>, n: u16) -> bool {
    matches!(vs.nodes[n as usize], NirNode { kind: Kind::VsTex { .. }, bits: 16, .. })
}

/// Node `n` loads its 16-bit lanes two a register in lane order: an fp16
/// vertex look-up ([`vs_is_tex16`]) or an fp16 array element read at a
/// run-time index ([`uniform_idx16`], the `LD_PKA` words).
pub(super) fn vs_is_word16(vs: &Nir<'_>, n: u16) -> bool {
    vs_is_tex16(vs, n) || matches!(vs.nodes[n as usize], NirNode { kind: Kind::UniformIdx { .. }, bits: 16, .. })
}

/// Source `v` of a two-lane fp16 operation `nd` is a whole register of an
/// fp16 look-up or indexed element ([`vs_is_word16`]): lane `c` (even) for
/// its lanes `c` and `c + 1` ([`vs_vec16`]). A `vec2` reads its sources one
/// half each.
pub(super) fn vs_tex16_word(vs: &Nir<'_>, nd: &NirNode, v: Val) -> bool {
    matches!(v, Val::Lane(n, c) if c % 2 == 0 && vs_is_word16(vs, n))
        && nd.bits == 16
        && nd.width == 2
        && nd.kind != Kind::Alu(Alu::Vec2)
}

/// The IR values `a` and `b` compute the same thing by their instructions
/// (an input, a constant, a uniform, or the same operation over such),
/// the shape `nir_opt_cse` merges; `depth` bounds the walk. Anything else
/// (a local's read, an indexed uniform) is not compared: false.
fn ir_same(m: &Module<'_>, a: ValueId, b: ValueId, depth: u8) -> bool {
    if a == b {
        return true;
    }
    let same = |x: ir::Src, y: ir::Src| x.swizzle == y.swizzle && ir_same(m, x.val, y.val, depth - 1);
    match (m.def(a), m.def(b)) {
        _ if depth == 0 => false,
        (Some(x @ (IrOp::LoadInput { .. } | IrOp::ConstF32(_) | IrOp::LoadUniform { .. })), Some(y)) => x == y,
        (Some(IrOp::Alu(p, s)), Some(IrOp::Alu(q, t))) => {
            p == q && (0..super::nir_fs::ir_arity(*p)).all(|k| same(s[k], t[k]))
        }
        (
            Some(&IrOp::Tex { dim, shadow, proj, ret, sampler, coord, lod, offset }),
            Some(&IrOp::Tex {
                dim: dim2,
                shadow: shadow2,
                proj: proj2,
                ret: ret2,
                sampler: sampler2,
                coord: coord2,
                lod: lod2,
                offset: offset2,
            }),
        ) => {
            (dim, shadow, proj, ret, sampler, offset) == (dim2, shadow2, proj2, ret2, sampler2, offset2)
                && same(coord, coord2)
                && match (lod, lod2) {
                    (TexLod::Lod(x), TexLod::Lod(y)) => same(x, y),
                    (x, y) => x == y,
                }
        }
        _ => false,
    }
}

/// The vertex look-up `id` of a lowp or mediump sampler is fp16 in
/// Mesa's code ([`Kind::VsTex`]`::medium`): a mediump `fmul`, `fadd`,
/// `ffma` or `fneg` reads it, or reads a look-up `nir_opt_cse` makes one
/// with it ([`ir_same`]; gm2_texvert_4's `v_a` beside `v_b`).
fn vs_tex_read16(m: &Module<'_>, id: ir::InstrId) -> bool {
    let me = ValueId(id.0);
    let tex = |v: ValueId| v == me || (matches!(m.def(v), Some(IrOp::Tex { .. })) && ir_same(m, v, me, 8));
    m.instrs.iter().enumerate().any(|(i, op)| match op {
        IrOp::Alu(a @ (AluOp::Fmul | AluOp::Fadd | AluOp::Ffma | AluOp::Fneg), s) => {
            m.prec_of(ValueId(i as u32)) == Prec::Medium && s[..super::nir_fs::ir_arity(*a)].iter().any(|x| tex(x.val))
        }
        _ => false,
    })
}

/// `n / d` of two 32-bit signed integers in the vertex stage: the
/// `nir_lower_idiv` sequence node for node as nir_fs.rs `Fs::idiv` builds
/// it (`emit_udiv` under `nir_lower_idiv_fast`'s absence: the reciprocal
/// estimate by `frcp` scaled by `0x4f7ffffe`, refined once by the 16-bit
/// halves' `umul_high`, the quotient by another, two corrections and the
/// sign; deqp_intval_opvs_div_ivec2_highp, the sequence twice). Both
/// operands are run-time values here: none of the folds of the fragment
/// walk's `iop` applies, except `extract_u16(extract_u16(a, 1), 0)` as its
/// source.
fn vs_idiv(vs: &mut Nir<'_>, n: Val, d: Val) -> Result<Val, MaliError> {
    let u = Val::Undef;
    let op = |vs: &mut Nir<'_>, op: Alu, a: Val, b: Val, c: Val| -> Result<Val, MaliError> {
        if let (Alu::Extract16(0), Some((Alu::Extract16(1), _))) = (op, vs_alu_of(vs, a)) {
            return Ok(a);
        }
        vs.alu(op, a, b, c)
    };
    let umul_high = |vs: &mut Nir<'_>, x: Val, y: Val| -> Result<Val, MaliError> {
        let x0 = op(vs, Alu::Extract16(0), x, u, u)?;
        let y0 = op(vs, Alu::Extract16(0), y, u, u)?;
        let x1 = op(vs, Alu::Extract16(1), x, u, u)?;
        let y1 = op(vs, Alu::Extract16(1), y, u, u)?;
        let p00 = op(vs, Alu::Imul, x0, y0, u)?;
        let p01 = op(vs, Alu::Imul, x0, y1, u)?;
        let p10 = op(vs, Alu::Imul, x1, y0, u)?;
        let p11 = op(vs, Alu::Imul, x1, y1, u)?;
        let carry = op(vs, Alu::Extract16(1), p00, u, u)?;
        let s1 = op(vs, Alu::Iadd, p10, carry, u)?;
        let s1l = op(vs, Alu::Extract16(0), s1, u, u)?;
        let s1h = op(vs, Alu::Extract16(1), s1, u, u)?;
        let s2 = op(vs, Alu::Iadd, p01, s1l, u)?;
        let s2h = op(vs, Alu::Extract16(1), s2, u, u)?;
        let t = op(vs, Alu::Iadd, s2h, s1h, u)?;
        op(vs, Alu::Iadd, p11, t, u)
    };
    let a = op(vs, Alu::Iabs, n, u, u)?;
    let b = op(vs, Alu::Iabs, d, u, u)?;
    let sign = op(vs, Alu::Ixor, n, d, u)?;
    let f = op(vs, Alu::U2f, b, u, u)?;
    let r = op(vs, Alu::Frcp, f, u, u)?;
    let m = op(vs, Alu::Fmul, r, Val::Const(IDIV_RCP_SCALE), u)?;
    let e = op(vs, Alu::F2u, m, u, u)?;
    let t = op(vs, Alu::Imul, b, e, u)?;
    let neg = op(vs, Alu::Ineg, t, u, u)?;
    let hi = umul_high(vs, e, neg)?;
    let e = op(vs, Alu::Iadd, e, hi, u)?;
    let q = umul_high(vs, a, e)?;
    let qd = op(vs, Alu::Imul, q, b, u)?;
    let rem = op(vs, Alu::Isub, a, qd, u)?;
    let c1 = op(vs, Alu::Ucmp(Cmpf::Ge), rem, b, u)?;
    let i1 = op(vs, Alu::B2i, c1, u, u)?;
    let q = op(vs, Alu::Iadd, q, i1, u)?;
    let rs = op(vs, Alu::Isub, rem, b, u)?;
    let rem = op(vs, Alu::Bcsel, c1, rs, rem)?;
    let c2 = op(vs, Alu::Ucmp(Cmpf::Ge), rem, b, u)?;
    let i2 = op(vs, Alu::B2i, c2, u, u)?;
    let q = op(vs, Alu::Iadd, q, i2, u)?;
    let nq = op(vs, Alu::Ineg, q, u, u)?;
    let neg_sign = op(vs, Alu::Icmp(Cmpf::Lt), sign, Val::Const(0), u)?;
    op(vs, Alu::Bcsel, neg_sign, nq, q)
}

/// `nir_lower_idiv`'s scale of the divisor's reciprocal, 4294966784.0 (the
/// largest float below 2^32: the estimate never overshoots;
/// intbig_case79.stderr's `fmul %17, %18 (4.294967e+09)`); the fragment
/// walk's too.
pub(super) const IDIV_RCP_SCALE: u32 = 0x4f7f_fffe;

/// A constant in [0, 1] (`is_zero_to_one`).
fn zero_to_one(v: Val) -> bool {
    matches!(v, Val::Const(c) if (0.0..=1.0).contains(&f32::from_bits(c)))
}

/// A vertex `fmax(p, q)` or `fmin(p, q)` as NIR's rules leave it
/// (docs/notes/mali-vs-compare-clamp.md section 3, vsarith_182_fsat and
/// _fsatfolded): `fmax(a, 0.0)` is `fclamp_pos(a)` (bifrost_nir_algebraic.py:34),
/// `fmin(fclamp_pos(a), 1.0)` and `fmax(fmin(a, 1.0), 0.0)` are `fsat(a)`
/// (nir_opt_algebraic.py:1088, :1091); with a bound `b` in [0, 1] on the
/// other side, `fmin(fmax(a, 0.0), b)` is `fsat(fmin(a, b))` and
/// `fmin(fmax(a, b), 1.0)` `fsat(fmax(a, b))` (the `FMIN.f32.clamp_0_1` and
/// `FMAX.f32.clamp_0_1` of the reference: opt.rs folds the saturate into
/// its producer). The node an outer rule replaces is left to [`dce`]. A
/// bound is a constant: two computed operands are a plain `FMAX`/`FMIN`
/// no vertex reference draws.
fn vs_clamp(vs: &mut Nir<'_>, a: AluOp, p: Val, q: Val) -> Result<Val, MaliError> {
    let u = Val::Undef;
    let (x, k) = match (p, q) {
        (_, Val::Const(_)) => (p, q),
        (Val::Const(_), _) => (q, p),
        _ => return Err(MaliError::Arithmetic),
    };
    let inner = vs_alu_of(vs, x);
    if a == AluOp::Fmax {
        if k == Val::Const(F32_ZERO) {
            return match inner {
                Some((Alu::Fmin, [b, Val::Const(F32_ONE), _])) => vs.alu(Alu::Fclamp(clamp::ZERO_ONE), b, u, u),
                Some((Alu::Fmin, [_, c, _])) if zero_to_one(c) => vs.alu(Alu::Fclamp(clamp::ZERO_ONE), x, u, u),
                _ => vs.alu(Alu::Fclamp(clamp::ZERO_INF), x, u, u),
            };
        }
        /* `fmax(fmin(a, 1.0), b)`, a clamp to [b, 1]: no reference */
        return match inner {
            Some((Alu::Fmin, [_, Val::Const(F32_ONE), _])) if zero_to_one(k) => Err(MaliError::Arithmetic),
            _ => vs.alu(Alu::Fmax, x, k, u),
        };
    }
    match inner {
        Some((Alu::Fclamp(clamp::ZERO_INF), [b, ..])) if k == Val::Const(F32_ONE) => {
            vs.alu(Alu::Fclamp(clamp::ZERO_ONE), b, u, u)
        }
        Some((Alu::Fclamp(clamp::ZERO_INF), [b, ..])) if zero_to_one(k) => {
            let n = vs.alu(Alu::Fmin, b, k, u)?;
            vs.alu(Alu::Fclamp(clamp::ZERO_ONE), n, u, u)
        }
        Some((Alu::Fclamp(clamp::ZERO_INF), _)) => Err(MaliError::Arithmetic),
        Some((Alu::Fmax, [_, c, _])) if k == Val::Const(F32_ONE) && zero_to_one(c) => {
            vs.alu(Alu::Fclamp(clamp::ZERO_ONE), x, u, u)
        }
        _ => vs.alu(Alu::Fmin, x, k, u),
    }
}

/// One side of a product of dEQP's mediump `compare_float` results
/// (`abs(a - b) < c ? 1.0 : 0.0`, uniform_api's `compare_vec2`,
/// `compare_mat2`): the walk's `b2f(flt(fabs(f2f32(x)), c))` of a 16-bit
/// lane `x`, `x - 0.0` read as `x` (`fadd(a, -0.0)` is `a`,
/// nir_opt_algebraic.py), or a product already folded
/// ([`vs_cmp_and`]): (the fp16 value compared, true for a leaf whose
/// `fabs` is still to make, `c` as an fp16 half).
fn vs_cmp_side(vs: &Nir<'_>, v: Val) -> Option<(Val, bool, u16)> {
    let (Alu::B2f, [m, ..]) = vs_alu_of(vs, v)? else { return None };
    match vs_alu_of(vs, m)? {
        (Alu::Fcmp(Cmpf::Lt), [a, Val::Const(c), _]) if vs.nodes[lane_node(m)? as usize].bits == 32 => {
            /* `fabs(f2f32(x))`, or `f2f32(fabs(x))` of the 16-bit `fabs`
             * the walk makes of a mediump `abs` ([`vs_fabs16`]) */
            let x = match vs_alu_of(vs, a)? {
                (Alu::Fabs, [w, ..]) => {
                    let (Alu::F2f32, [x, ..]) = vs_alu_of(vs, w)? else { return None };
                    x
                }
                (Alu::F2f32, [w, ..]) if vs_is_f16(vs, w) => {
                    let (Alu::Fabs, [x, ..]) = vs_alu_of(vs, w)? else { return None };
                    x
                }
                _ => return None,
            };
            if !vs_is_f16(vs, x) {
                return None;
            }
            /* `x - 0.0`: the identity at the link */
            let x = match vs_alu_of(vs, x) {
                Some((Alu::Fadd, [y, Val::Const(0 | 0x8000 | F32_NEG_ZERO), _])) if vs_is_f16(vs, y) => y,
                _ => x,
            };
            Some((x, true, select::f16_rtne(c)?))
        }
        (Alu::I2i32, [k, ..]) => {
            let (Alu::Fcmp(Cmpf::Lt), [mx, Val::Const(c16), _]) = vs_alu_of(vs, k)? else { return None };
            Some((mx, false, (c16 & 0xffff) as u16))
        }
        _ => None,
    }
}

/// The node a lane `v` is of.
fn lane_node(v: Val) -> Option<u16> {
    match v {
        Val::Lane(n, _) => Some(n),
        _ => None,
    }
}

/// `fmul(b2f(a), b2f(b))` of two [`vs_cmp_side`]s of one constant `c`:
/// `b2f(iand(a, b))` (nir_opt_algebraic.py:1679), `iand(flt(x, c), flt(y,
/// c))` is `flt(fmax(x, y), c)` (:1159), all at 16 bits as lower_precision
/// leaves the mediump compares; the `b2f32` of the 16-bit mask is
/// `S16_TO_S32` then `CSEL.u32.eq` (deqp_unitype_vsapi_mat2: `FMAX.v2f16 r0,
/// u5.w1.abs.h00, u5.w1.abs.h11` per column, their `FMAX.v2f16`, then
/// `FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u6.w1, 0x0` of the fp16 0.05 pair).
fn vs_cmp_and(vs: &mut Nir<'_>, p: Val, q: Val) -> Result<Option<Val>, MaliError> {
    let (Some((x, xl, c)), Some((y, yl, d))) = (vs_cmp_side(vs, p), vs_cmp_side(vs, q)) else {
        return Ok(None);
    };
    if c != d {
        return Ok(None);
    }
    let h = c;
    let u = Val::Undef;
    let x = if xl { vs.alu_sized(Alu::Fabs, 16, x, u, u)? } else { x };
    let y = if yl { vs.alu_sized(Alu::Fabs, 16, y, u, u)? } else { y };
    let mx = vs.alu_sized(Alu::Fmax, 16, x, y, u)?;
    let k = vs.alu_sized(Alu::Fcmp(Cmpf::Lt), 16, mx, Val::Const(u32::from(h) * 0x1_0001), u)?;
    let m = vs.alu(Alu::I2i32, k, u, u)?;
    Ok(Some(vs.alu(Alu::B2f, m, u, u)?))
}

/// A vertex `any` / `all` of a compare vector, as Mesa's passes leave the
/// reduction (docs/notes/mali-vs-compare-clamp.md section 2,
/// vsarith_86_anyall): `all(equal(v.xy, 0.0))` is `feq(fadd(fabs(y),
/// fabs(x)), 0.0)` (nir_opt_algebraic.py:1213) and `any(lessThan(c,
/// v.xyz))` of a constant `c` is `flt(c, fmax(fmax(z, x), y))` (:1178,
/// then :1150). The compare's sources are read here; its own nodes are
/// left to [`dce`]. Other widths, compares and lane orders are shapes no
/// vertex reference draws.
fn vs_reduce(vs: &mut Nir<'_>, m: &Module<'_>, w: &Walk<'_>, a: AluOp, s: ir::Src) -> Result<Val, MaliError> {
    let u = Val::Undef;
    /* the front end repeats the last lane read into the spare ones */
    let n = (1..=4).find(|&n| s.swizzle[n - 1..].iter().all(|&c| c == s.swizzle[3])).unwrap_or(4);
    let (op, cs) = match m.def(s.val) {
        Some(&IrOp::Alu(op, cs)) => (op, cs),
        _ => return Err(MaliError::Arithmetic),
    };
    /* a compare of two constant vectors folds, lane by lane, then the
     * reduction (`nir_opt_constant_folding` of the `ball_fequal3` GLSL's
     * `==` of vectors makes; deqp_constop_struct_param_out_vs: `s.red ==
     * vec3(1.0, 0.0, 0.0)` of an `out` struct's constant members, no
     * instruction left) */
    let konst = |k: usize, i: usize| match lane(w, cs[k], s.swizzle[i] as usize) {
        Val::Const(c) => Some(f32::from_bits(c)),
        _ => None,
    };
    if (0..n).all(|i| konst(0, i).is_some() && konst(1, i).is_some()) {
        let t = |i: usize| {
            let (x, y) = (konst(0, i).unwrap_or(0.0), konst(1, i).unwrap_or(0.0));
            match op {
                AluOp::Feq => Some(x == y),
                AluOp::Fne => Some(x != y),
                AluOp::Flt => Some(x < y),
                AluOp::Fle => Some(x <= y),
                _ => None,
            }
        };
        let r = match a {
            AluOp::Ball => (0..n).try_fold(true, |acc, i| t(i).map(|b| acc && b)),
            AluOp::Bany => (0..n).try_fold(false, |acc, i| t(i).map(|b| acc || b)),
            _ => None,
        };
        if let Some(r) = r {
            return Ok(Val::Const(if r { u32::MAX } else { 0 }));
        }
    }
    /* lane i of the reduction: the compare's lane, of one vector's lane i
     * against a constant on the side `side` */
    let side = |k: usize| -> Option<[Val; 4]> {
        let mut v = [u; 4];
        let mut c = None;
        let mut node = None;
        for (i, o) in v.iter_mut().enumerate().take(n) {
            let j = s.swizzle[i] as usize;
            let (p, q) = (lane(w, cs[k], j), lane(w, cs[1 - k], j));
            match (p, q) {
                (Val::Lane(nd, l), Val::Const(_)) if l as usize == i && node.is_none_or(|x| x == nd) => {
                    if c.is_some_and(|c| c != q) {
                        return None;
                    }
                    (node, c, *o) = (Some(nd), Some(q), p);
                }
                _ => return None,
            }
        }
        c.map(|c| {
            v[3] = c;
            v
        })
    };
    /* `all(b == bvec3(false))` of booleans (a bool uniform's words, 0 /
     * ~0): `inot(ior(ior(b.y, b.x), b.z))`, the `ball_iequal3` of `ieq(b,
     * false)` lanes De Morgan's (nir_opt_algebraic.py), whose `bcsel` is
     * the `CSEL.u32.eq` of the OR against 0 (deqp_unitype_vsbvec3:
     * `LSHIFT_OR.i32 r0, u6.w0, 0x0.b00, r0^` of y and x, then of z,
     * then `CSEL.u32.eq r0, r0^, 0x0, 0x3F800000, 0x0`); a true of 1 or
     * ~0 is the same bytes. Other widths have no reference */
    if a == AluOp::Ball && op == AluOp::Ieq && n == 3 {
        let mut b = [u; 3];
        for (i, x) in b.iter_mut().enumerate() {
            let j = s.swizzle[i] as usize;
            *x = match (lane(w, cs[0], j), lane(w, cs[1], j)) {
                (p, Val::Const(0)) | (Val::Const(0), p) if vs_is_bool(vs, p) && !matches!(p, Val::Const(_)) => p,
                _ => return Err(MaliError::Arithmetic),
            };
        }
        let zy = vs.alu(Alu::Ior, b[2], b[1], u)?;
        let any = vs.alu(Alu::Ior, zy, b[0], u)?;
        return vs.alu(Alu::Inot, any, u, u);
    }
    match (a, op, n) {
        (AluOp::Ball, AluOp::Feq, 2) => {
            let v = side(0).or_else(|| side(1)).ok_or(MaliError::Arithmetic)?;
            if v[3] != Val::Const(F32_ZERO) {
                return Err(MaliError::Arithmetic);
            }
            let y = vs.alu(Alu::Fabs, v[1], u, u)?;
            let x = vs.alu(Alu::Fabs, v[0], u, u)?;
            let sum = vs.alu(Alu::Fadd, y, x, u)?;
            vs.alu(Alu::Fcmp(Cmpf::Eq), sum, Val::Const(F32_ZERO), u)
        }
        (AluOp::Bany, AluOp::Flt, 3) => {
            /* `c < v`: the vector on the right */
            let v = side(1).ok_or(MaliError::Arithmetic)?;
            let mx = vs.alu(Alu::Fmax, v[2], v[0], u)?;
            let mx = vs.alu(Alu::Fmax, mx, v[1], u)?;
            vs.alu(Alu::Fcmp(Cmpf::Lt), v[3], mx, u)
        }
        _ => Err(MaliError::Arithmetic),
    }
}

/// One step of the vertex walk: an instruction, or the bounds of an `if`
/// whose sides [`vs_steps`] flattens.
#[derive(Clone, Copy)]
enum Step {
    Instr(ir::InstrId),
    If(ir::Src),
    Else,
    EndIf,
    /* a node the walk refuses, where it stands */
    Refuse(MaliError),
    /// A top-level loop of [`vs_loop_shape`]'s form: its body, and the index
    /// of its [`Step::LoopEnd`]. The body's steps follow: its instructions
    /// and its head test ([`Step::Head`]).
    LoopBegin(ir::Range, u32),
    /// The loop's head test, `if (c) {} else break;` (`Lower::break_unless`).
    Head(ir::Src),
    /// The end of a loop's body: the index of its [`Step::LoopBegin`].
    LoopEnd(u32),
}

/// A loop body the vertex walk takes (plan #367 lower-int-loops,
/// docs/notes/mali-int-loops.md): instructions and one head test `if (c)
/// {} else break;` (`Lower::break_unless`), the first control-flow node,
/// nothing else. A `for` of a constant bound is unrolled ([`walk`]), one of
/// a run-time bound kept (gm2_cf_loop_uni_vs, glmark2 loop's vs 3cb0); an
/// `if`, a `continue` or a second `break` has no vertex reference. A loop
/// of this form after the head test nests (deqp_loops_vs_nested_const:
/// counted loops in a counted loop, all unrolled, the inner ones first as
/// `nir_opt_loop_unroll` takes them); a kept one nested in another, or
/// around one, is refused when it is walked (`intloop_loop_nested_vs`).
fn vs_loop_shape(m: &Module<'_>, body: ir::Range) -> bool {
    let mut head = false;
    for nd in m.nodes_of(body) {
        match *nd {
            IrNode::Instr(_) => {}
            IrNode::If { then_, else_, .. } if !head && then_.is_empty() && m.nodes_of(else_) == [IrNode::Break] => {
                head = true
            }
            IrNode::Loop { body } if head && vs_loop_shape(m, body) => {}
            _ => return false,
        }
    }
    head
}

/// The steps [`vs_steps`] makes of a loop of [`vs_loop_shape`]'s form.
fn vs_loop_steps(m: &Module<'_>, body: ir::Range) -> usize {
    2 + m
        .nodes_of(body)
        .iter()
        .map(|nd| match *nd {
            IrNode::Loop { body } => vs_loop_steps(m, body),
            _ => 1,
        })
        .sum::<usize>()
}

/// The steps of loop `body` ([`vs_loop_shape`]) into `steps` from `k`:
/// [`Step::LoopBegin`], the body's instructions, its head test and its
/// nested loops, then [`Step::LoopEnd`].
fn put_loop(m: &Module<'_>, body: ir::Range, steps: &mut [Step], k: &mut usize) {
    let begin = *k;
    steps[*k] = Step::LoopBegin(body, (begin + vs_loop_steps(m, body) - 1) as u32);
    *k += 1;
    for nd in m.nodes_of(body) {
        match *nd {
            IrNode::Instr(id) => {
                steps[*k] = Step::Instr(id);
                *k += 1;
            }
            IrNode::If { cond, .. } => {
                steps[*k] = Step::Head(cond);
                *k += 1;
            }
            IrNode::Loop { body } => put_loop(m, body, steps, k),
            _ => {
                steps[*k] = Step::Refuse(MaliError::LoopVertex);
                *k += 1;
            }
        }
    }
    steps[*k] = Step::LoopEnd(begin as u32);
    *k += 1;
}

/// The vertex shader's body as [`Step`]s. An `if` is flattened when each
/// side is straight code without a look-up (`nir_opt_peephole_select`
/// turns it into selects, vsarith_188_predif: docs/notes/mali-vs-abs-if.md);
/// a nested `if`, a loop, a jump, a look-up or more than
/// [`PEEPHOLE_LIMIT_GL`] operations under it stays refused, at
/// the `if`'s place in the walk (an earlier refusal comes first).
fn vs_steps<'s>(m: &Module<'_>, scratch: &Arena<'s>) -> Result<&'s [Step], MaliError> {
    let nodes = m.nodes_of(m.body);
    /* a side's operations, at most `PEEPHOLE_LIMIT_GL` (the GL front end's
     * peephole limit), counted as the IR's ALU instructions */
    let side = |r: ir::Range| -> Result<usize, MaliError> {
        let mut alu = 0;
        vs_side(m, r, &mut alu, true)
    };
    let flat = |then_: ir::Range, else_: ir::Range| -> Result<usize, MaliError> { Ok(side(then_)? + side(else_)?) };
    /* the taken side of a top-level `if` of a constant condition */
    let taken = |cond: ir::Src, then_: ir::Range, else_: ir::Range| {
        vs_const_cond(m, cond).map(|t| if t { then_ } else { else_ })
    };
    let mut n = 0;
    for nd in nodes {
        n += match *nd {
            IrNode::If { cond, then_, else_ } if taken(cond, then_, else_).is_some() => {
                let r = taken(cond, then_, else_).unwrap_or(then_);
                vs_side(m, r, &mut 0, false).map_or(1, |k| k.max(1))
            }
            IrNode::If { then_, else_, .. } => flat(then_, else_).map_or(1, |k| k + 3),
            IrNode::Loop { body } if vs_loop_shape(m, body) => vs_loop_steps(m, body),
            _ => 1,
        };
    }
    let steps = scratch.alloc_filled(n, Step::Else).map_err(|_| MaliError::OutOfScratch)?;
    let mut k = 0;
    macro_rules! put {
        ($s:expr) => {{
            steps[k] = $s;
            k += 1;
        }};
    }
    for (j, nd) in nodes.iter().enumerate() {
        match *nd {
            IrNode::Instr(id) => put!(Step::Instr(id)),
            IrNode::Return if j + 1 == nodes.len() => {}
            /* a counted or uniform-bounded `for` ([`vs_loop_shape`]) */
            IrNode::Loop { body } if vs_loop_shape(m, body) => put_loop(m, body, steps, &mut k),
            /* intloop_loop_nested_vs: Mesa keeps the loops as blocks */
            IrNode::Loop { .. } => put!(Step::Refuse(MaliError::LoopVertex)),
            /* an `if` of a constant condition is its taken side, inline
             * ([`vs_const_cond`]): no select and no peephole limit */
            IrNode::If { cond, then_, else_ } if taken(cond, then_, else_).is_some() => {
                let r = taken(cond, then_, else_).unwrap_or(then_);
                match vs_side(m, r, &mut 0, false) {
                    Ok(_) => vs_side_instrs(m, r, &mut |id| put!(Step::Instr(id))),
                    Err(e) => put!(Step::Refuse(e)),
                }
            }
            IrNode::If { then_, else_, .. } if flat(then_, else_).is_err() => {
                put!(Step::Refuse(flat(then_, else_).err().unwrap_or(MaliError::ControlFlow)))
            }
            IrNode::If { cond, then_, else_ } => {
                put!(Step::If(cond));
                vs_side_instrs(m, then_, &mut |id| put!(Step::Instr(id)));
                put!(Step::Else);
                vs_side_instrs(m, else_, &mut |id| put!(Step::Instr(id)));
                put!(Step::EndIf);
            }
            _ => put!(Step::Refuse(MaliError::ControlFlow)),
        }
    }
    Ok(&steps[..k])
}

/// The value of an `if`'s condition that is a boolean constant: GLSL's
/// `if (false)` and `if (true)`, which Mesa's GLSL optimizer removes
/// before NIR (`opt_if_simplification.cpp`: the taken side replaces the
/// `if`), so `nir_opt_peephole_select` never sees it
/// (deqp_ctrlflow_vs_nested's `if (false)` under an `if` of a compare,
/// byte-equal).
fn vs_const_cond(m: &Module<'_>, cond: ir::Src) -> Option<bool> {
    ir_const_lane(m, cond.val, usize::from(cond.swizzle[0] & 3), 0).map(|c| c != 0)
}

/// Lane `l` of IR value `v` when it is a constant expression of ints and
/// bools: a constant, or an operation [`fold_int`] folds on constant
/// lanes (deqp_constrnd_int_bits_vs's `const bool l = int(1) * 1 ==
/// int(int(1.0))`, which Mesa's GLSL front end makes a constant). `None`
/// for anything else.
fn ir_const_lane(m: &Module<'_>, v: ir::ValueId, l: usize, depth: u32) -> Option<u32> {
    match m.def(v)? {
        IrOp::ConstF32(c) => c.get(l).copied(),
        IrOp::ConstI32(c) => c.get(l).map(|&x| x as u32),
        IrOp::ConstBool(c) => c.get(l).map(|&b| if b { u32::MAX } else { 0 }),
        /* a local stored once in the stage (a global `const`, which the
         * front end stores at the top of `main`): the stored lane */
        &IrOp::LoadLocal { local, index: None, .. } if depth < 16 => {
            let mut stores = m.instrs.iter().filter_map(|op| match op {
                IrOp::StoreLocal { local: s, index, src, write_mask } if *s == local => {
                    Some((*index, *src, *write_mask))
                }
                _ => None,
            });
            match (stores.next(), stores.next()) {
                (Some((None, src, mask)), None) if mask & (1 << l) != 0 => {
                    ir_const_lane(m, src.val, usize::from(src.swizzle[l] & 3), depth + 1)
                }
                _ => None,
            }
        }
        &IrOp::Alu(a, s) if depth < 16 => {
            let n = super::nir_fs::ir_arity(a);
            if n > 2 {
                return None;
            }
            let mut x = [Val::Undef; 2];
            for (i, xi) in x.iter_mut().enumerate().take(n) {
                let lane = usize::from(s[i].swizzle[l] & 3);
                *xi = Val::Const(ir_const_lane(m, s[i].val, lane, depth + 1)?);
            }
            match fold_int(a, x[0], x[1])? {
                Val::Const(c) => Some(c),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The steps a side of a flattened vertex `if` counts ([`vs_steps`]): its
/// instructions, and those of the taken side of a nested `if` of a
/// constant condition ([`vs_const_cond`]). With `limit`, a look-up or
/// more than [`PEEPHOLE_LIMIT_GL`] ALU instructions (counted in `alu`)
/// refuse, as `nir_opt_peephole_select` keeps such an `if`; without it
/// (the taken side of a constant `if`, which is no `if` in Mesa's NIR)
/// only control flow does.
fn vs_side(m: &Module<'_>, r: ir::Range, alu: &mut usize, limit: bool) -> Result<usize, MaliError> {
    let mut n = 0;
    for nd in m.nodes_of(r) {
        match *nd {
            IrNode::Instr(id) => {
                match m.instr(id) {
                    Some(IrOp::Tex { .. } | IrOp::TexSize { .. } | IrOp::LoadUniformIdx { .. }) if limit => {
                        return Err(MaliError::ControlFlow)
                    }
                    Some(IrOp::Alu(..)) if limit && *alu == PEEPHOLE_LIMIT_GL as usize => {
                        return Err(MaliError::ControlFlow)
                    }
                    Some(IrOp::Alu(..)) => *alu += 1,
                    _ => {}
                }
                n += 1;
            }
            IrNode::If { cond, then_, else_ } => match vs_const_cond(m, cond) {
                Some(t) => n += vs_side(m, if t { then_ } else { else_ }, alu, limit)?,
                None => return Err(MaliError::ControlFlow),
            },
            IrNode::Loop { .. } => return Err(MaliError::LoopVertex),
            _ => return Err(MaliError::ControlFlow),
        }
    }
    Ok(n)
}

/// The instructions of a side [`vs_side`] took, in order.
fn vs_side_instrs(m: &Module<'_>, r: ir::Range, put: &mut dyn FnMut(ir::InstrId)) {
    for nd in m.nodes_of(r) {
        match *nd {
            IrNode::Instr(id) => put(id),
            IrNode::If { cond, then_, else_ } => {
                if let Some(t) = vs_const_cond(m, cond) {
                    vs_side_instrs(m, if t { then_ } else { else_ }, put);
                }
            }
            _ => {}
        }
    }
}

/// What a side of a flattened vertex `if` writes: the locals and the
/// outputs.
#[derive(Clone, Copy)]
struct VsState {
    locals: [[Val; 4]; MAX_LOCALS],
    position: Option<[Val; 4]>,
    point_size: Option<Val>,
    varyings: [Option<[Val; 4]>; MAX_VARYINGS],
}

/// The join of a flattened `if` (`nir_opt_peephole_select`): a value both
/// sides leave equal is kept, another is `bcsel(c, then, else)`, the
/// `CSEL.u32.eq` of va_lower_isel (vsarith_188_predif: one per differing
/// lane, in the output's lane order). A varying lane written under the `if`
/// is a phi when Mesa's link runs (nir_opt_varyings before the select is
/// made): `phis` gets its (location, lane), a constant there stays stored
/// (`IADD_IMM.i32 r2, 0x0, #0x3F800000`, the 1.0 both sides store). A
/// value one side leaves undefined (`vsarith_partial_varying`, item 6 of
/// docs/notes/mali-vs-alu-ops.md) and a point size the sides set apart
/// (`vsarith_psiz_if`) are refused.
fn vs_join(
    vs: &mut Nir<'_>,
    c: Val,
    pre: &VsState,
    t: &VsState,
    e: &mut VsState,
    phis: &mut u64,
) -> Result<(), MaliError> {
    let sel = |vs: &mut Nir<'_>, a: Val, b: Val| -> Result<Val, MaliError> {
        if a == b {
            return Ok(a);
        }
        if a == Val::Undef || b == Val::Undef {
            return Err(MaliError::ControlFlow);
        }
        vs.alu(Alu::Bcsel, c, a, b)
    };
    for loc in 0..MAX_VARYINGS {
        let (a, b) = match (t.varyings[loc], e.varyings[loc]) {
            (Some(a), Some(b)) => (a, b),
            (None, None) => continue,
            _ => return Err(MaliError::ControlFlow),
        };
        let mut out = [Val::Undef; 4];
        for i in 0..4 {
            out[i] = sel(vs, a[i], b[i])?;
            if pre.varyings[loc].is_none_or(|p| p[i] != out[i]) && loc * 4 + i < 64 {
                *phis |= 1 << (loc * 4 + i);
            }
        }
        e.varyings[loc] = Some(out);
    }
    e.position = match (t.position, e.position) {
        (Some(a), Some(b)) => {
            let mut out = [Val::Undef; 4];
            for i in 0..4 {
                out[i] = sel(vs, a[i], b[i])?;
            }
            Some(out)
        }
        (None, None) => None,
        _ => return Err(MaliError::ControlFlow),
    };
    /* a point size the sides set apart: vsarith_psiz_if orders its loads
     * and stores in a way this walk does not make yet */
    if t.point_size != e.point_size {
        return Err(MaliError::ControlFlow);
    }
    for l in 0..MAX_LOCALS {
        for i in 0..4 {
            let (a, b) = (t.locals[l][i], e.locals[l][i]);
            /* a local one side does not define is dead after the `if`
             * unless read: undefined, as NIR's phi of an undef folds */
            e.locals[l][i] = if a == Val::Undef || b == Val::Undef { Val::Undef } else { sel(vs, a, b)? };
        }
    }
    Ok(())
}

/// Build the program of a vertex shader. `reads`: the fragment shader's
/// reads of the program (location, component mask), when the vertex shader
/// is built for one: then only those components are stored, compacted as
/// Mesa's link does ([`compact_layout`]); `moved`: the fragment shader's
/// expressions the link moves here (motion.rs), computed before the
/// stores and stored in their lanes; `fp16`: the uniforms declared mediump
/// or lowp ([`fp16_uniform`]).
pub fn build<'s>(
    m: &Module<'_>,
    scratch: &Arena<'s>,
    reads: Option<&[(u8, u8)]>,
    moved: &[super::motion::Moved],
    fp16: &dyn Fn(&str) -> bool,
) -> Result<Nir<'s>, MaliError> {
    let mut vs = walk(m, scratch, reads, moved, fp16, false)?;
    vs.counted = vs.inputs_read()?;
    /* Mesa numbers the attribute locations a load still reads where its
     * IR is vectors (`nir_recompute_io_bases` at the end of
     * `nir_lower_io_passes` and of `nir_opt_varyings_bulk`); the back end
     * scalarises later (`bi_optimize_loop`'s `nir_lower_alu_width`) and
     * its `nir_opt_algebraic` may then remove a load, whose location stays
     * counted. The walk is per lane, so when it made such a fold
     * ([`vs_fmul`]) the locations are those of a second walk without it,
     * whose loads are a superset of the first's (it keeps a node where the
     * first folds one). A second walk that refuses is a form not
     * modelled. So is a varying lane such a fold makes a constant (or a
     * lane of uniforms, or equal to another) where the numbering depends
     * on the fold: Mesa's link (`nir_opt_varyings`, on the vectors) keeps
     * the lane computed and its location read, where our link would
     * propagate the constant into the fragment shader ([`Nir::consts`])
     * and leave the location unread (dEQP 1405_0's `vec4(.., d, ..) *
     * vec4(.., -0.0, b, a)`, `d`). The numbering depends on it when such a
     * lane of the second walk reads a location no output lane of the
     * first reads, each lane as if the fragment shader read it
     * ([`Nir::lane_inputs`], [`Nir::output_inputs`]: the link's constant
     * pass reads none). Where it does not, the first walk stands as it is,
     * as without the second (the factor a uniform, dEQP
     * basic_expression.vertex.12's `d`, or an attribute read in another
     * lane) */
    if vs.late_fold {
        /* a shader alone stores the lanes the first walk does not make
         * constant, whatever the second makes of them */
        let own = vs.own_reads;
        let reads = match (reads, &own) {
            (None, Some((own, n))) => Some(&own[..*n]),
            _ => reads,
        };
        let numbered = walk(m, scratch, reads, moved, fp16, true).map_err(|e| match e {
            MaliError::OutOfScratch => e,
            _ => MaliError::AttributeNumbering,
        })?;
        let counted = numbered.inputs_read()?;
        if vs.counted & !counted != 0 {
            return Err(MaliError::Defect(Defect::IrIndex));
        }
        let mut depends = 0u64;
        for o in m.outputs.iter().filter(|o| o.kind == IoKind::Varying) {
            for c in 0..o.components.min(4) {
                let link = |k: &super::link::VaryingConsts| {
                    (k.get(o.location, c), k.flat(o.location, c), k.class(o.location, c))
                };
                if link(&vs.consts) != link(&numbered.consts) {
                    depends |=
                        numbered.lane_inputs.get(usize::from(o.location) * 4 + usize::from(c)).copied().unwrap_or(0);
                }
            }
        }
        if depends & !vs.output_inputs != 0 {
            return Err(MaliError::AttributeNumbering);
        }
        vs.counted = counted;
        /* a lane the fold makes a constant is no constant to the link
         * (it runs before the back end's fold): Mesa stores it computed,
         * and the fragment shader loads it (gm2_arithconst_4's `diffuse *
         * Albedo.rgb` of `Albedo = vec4(0.0, 0.0, 1.0, 1.0)`: `IADD_IMM.i32
         * r1, 0x0, #0x0` stored into lanes 0 and 1, the `.v3` load) */
        for o in m.outputs.iter().filter(|o| o.kind == IoKind::Varying) {
            for c in 0..o.components.min(4) {
                let (l, k) = (o.location, &numbered.consts);
                if vs.consts.get(l, c).is_some() && k.get(l, c).is_none() && !k.flat(l, c) {
                    vs.consts.remove(l, c);
                }
            }
        }
    }
    Ok(vs)
}

/// Where Mesa's link resolves each lane of the IR operation `id` of `width`
/// lanes, `out` its lanes as the walk made them (`nir_scalar_resolved`,
/// nir_opt_varyings.c `deduplicate_outputs` and
/// `propagate_uniform_expressions`): 0 for a lane that is a scalar of its
/// own, or 1 + 4 * `id` + lane for a component of a vector operation, which
/// the link neither folds nor looks through. The link's NIR is vectors
/// (`nir_lower_alu_width` runs in the back end, after it), so:
///
/// * an operation of one lane is its own scalar;
/// * one whose lanes are all constants is folded (`nir_opt_constant_folding`);
/// * `a * 1.0` and `a + 0.0`, the constant the same in every lane, are `a`
///   (nir_opt_algebraic matches a vector constant lane by lane): the lanes
///   are `a`'s (`of`; varymerge_equal's `u * vec3(1.0)`, gm2_arithconst_3's
///   `diffuse * Albedo.rgb` of ones: merged);
/// * any other is kept: each lane a component of it, a constant lane too
///   (gm2_arithconst_4's `diffuse * vec3(0.0, 0.0, 1.0)`: both zeroes
///   stored; deqp_varylink_draw_single).
fn vec_origins(
    w: &Walk<'_>,
    id: u32,
    a: AluOp,
    s: &[ir::Src; 3],
    width: usize,
    out: &[Val; 4],
    of: impl Fn(ir::Src, usize) -> u16,
) -> [u16; 4] {
    let width = width.min(4);
    let lanes = &out[..width];
    if lanes.iter().filter(|v| **v != Val::Undef).count() <= 1 || lanes.iter().all(|v| matches!(v, Val::Const(_))) {
        return [0; 4];
    }
    let unit = match a {
        AluOp::Fmul => Some(F32_ONE),
        AluOp::Fadd => Some(F32_ZERO),
        _ => None,
    };
    let mut made = [0u16; 4];
    if let Some(k) = unit.and_then(|u| (0..2).find(|&k| (0..width).all(|i| lane(w, s[k], i) == Val::Const(u)))) {
        for (i, t) in made.iter_mut().enumerate().take(width) {
            *t = of(s[1 - k], i);
        }
        return made;
    }
    for (i, t) in made.iter_mut().enumerate().take(width) {
        *t = (1 + 4 * id as usize + i).min(u16::MAX as usize) as u16;
    }
    made
}

/// The loop the vertex walk is in ([`Step::LoopBegin`]).
#[derive(Clone, Copy)]
enum VsLoop {
    None,
    /// A counted loop, unrolled: its [`Step::LoopEnd`]'s index, the passes
    /// walked and the first node of the
    /// first pass (the cost bound, nir_opt_loop_unroll.c:924-925).
    Unroll {
        end: usize,
        pass: usize,
        first: usize,
        /// The trip count Mesa guesses from an array the counter
        /// subscripts (nir_fs.rs `guessed_trips`): the head test is taken
        /// while the counter is below it, its run-time bound dropped.
        guess: Option<u32>,
    },
    /// A loop Mesa keeps: its state is the walk's `kept` ([`KeptLoop`]).
    Kept,
}

/// A vertex loop Mesa keeps ([`vs_kept_begin`]).
#[derive(Clone, Copy)]
struct KeptLoop {
    lp: u8,
    pre: u8,
    header: u8,
    /// The header's phis: (node, local, lane).
    phis: [(u16, u8, u8); MAX_LOCALS],
    nphi: usize,
    /// The pass counter's phi and its next value, set by the head test
    /// ([`vs_kept_head`]).
    cap_phi: u16,
    next: Val,
    /// The head test's `break` block and the locals there, which the
    /// exit takes.
    brk: u8,
    brk_locals: [[Val; 4]; MAX_LOCALS],
    /// The nodes of the loop start here ([`vs_kept_end`]).
    first: usize,
}

/// A loop of [`vs_loop_shape`]'s form whose head test reads a run-time
/// value (an `int` uniform's word: gm2_cf_loop_uni_vs, glmark2 loop's vs
/// 3cb0): Mesa keeps it (no limiting terminator, nir_opt_loop_unroll.c:
/// 1112-1114) and `emit_loop` lays it out (bifrost_compile.c:3738-3770), as
/// the fragment walk's kept loop ([`super::nir_fs`] `walk_kept_loop`, whose
/// blocks and regions this makes the same): the header holds one phi per
/// local lane the body stores, in the reverse of the locals' registration
/// order ([`registration_order`]), each taking the value before the loop
/// and the body's last one, then the pass counter's phi. The counter is
/// the DELIBERATE deviation from Mesa of every kept loop of this back end
/// (docs/notes/mali-loop-board-failures.md: `LOOP_CAP` passes at most, so
/// a large bound cannot run a vertex job past the GPU's timeout). The body
/// stores no output (no reference).
fn vs_kept_begin(
    vs: &mut Nir<'_>,
    m: &Module<'_>,
    body: ir::Range,
    locals: &mut [[Val; 4]; MAX_LOCALS],
) -> Result<KeptLoop, MaliError> {
    if vs.nloops >= MAX_LOOPS || vs.nifs >= MAX_IFS {
        return Err(MaliError::BlockTableFull);
    }
    let nodes = m.nodes_of(body);
    let mut masks = [0u8; MAX_LOCALS];
    for nd in nodes {
        let IrNode::Instr(id) = *nd else { continue };
        match m.instr(id) {
            Some(&IrOp::StoreLocal { local, index: None, write_mask, .. }) => {
                *masks.get_mut(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))? |= write_mask;
            }
            Some(IrOp::StoreLocal { .. }) => return Err(MaliError::IndexedLocal),
            Some(IrOp::StoreOutput { .. }) => return Err(MaliError::LoopVertex),
            _ => {}
        }
    }
    let init = |l: ir::LocalId, _float: bool| match locals.get(l.0 as usize).map(|v| v[0]) {
        Some(Val::Const(bits)) => Some(bits),
        _ => None,
    };
    let limit =
        crate::shaderc::online::loop_count::constant_loop_entries(m, body, &init).unwrap_or(super::select::LOOP_CAP);
    let lp = vs.nloops as u8;
    vs.nloops += 1;
    let pre = vs.cur;
    let header = vs.new_block(pre)?;
    vs.cur = header;
    let reg = registration_order(m);
    let mut order = [(0u16, 0usize); MAX_LOCALS];
    let mut nl = 0;
    for (l, &mask) in masks.iter().enumerate() {
        if mask != 0 {
            order[nl] = (u16::MAX - reg[l], l);
            nl += 1;
        }
    }
    order[..nl].sort_unstable();
    let mut s = KeptLoop {
        lp,
        pre,
        header,
        phis: [(0, 0, 0); MAX_LOCALS],
        nphi: 0,
        cap_phi: 0,
        next: Val::Undef,
        brk: 0,
        brk_locals: *locals,
        first: vs.n,
    };
    for &(_, l) in &order[..nl] {
        for c in 0..4 {
            if masks[l] & (1 << c) == 0 {
                continue;
            }
            let v = locals[l][c];
            /* a local the body sets before it reads it: no phi */
            if v == Val::Undef {
                if !super::nir_fs::declared_in(m, nodes, l, c) {
                    return Err(MaliError::LoopVertex);
                }
                continue;
            }
            let bits = match v {
                Val::Lane(n, _) => vs.nodes[n as usize].bits,
                _ => 32,
            };
            /* a 16-bit (mediump) carried lane has no vertex reference */
            if bits != 32 || s.nphi == s.phis.len() {
                return Err(MaliError::LoopVertex);
            }
            let id = vs.push_node(NirNode { kind: Kind::Phi, src: [v, Val::Undef, Val::Undef], bits, width: 1 })?;
            locals[l][c] = Val::Lane(id, 0);
            s.phis[s.nphi] = (id, l as u8, c as u8);
            s.nphi += 1;
        }
    }
    let nd = NirNode { kind: Kind::Phi, src: [Val::Const(limit), Val::Undef, Val::Undef], bits: 32, width: 1 };
    s.cap_phi = vs.push_node(nd)?;
    Ok(s)
}

/// The head test of kept loop `s` on mask `c` (`if (c) {} else break;`):
/// `opt_if_simplification` swaps the sides under `inot(c)`, the inverse
/// compare (nir_opt_algebraic.py: `inot(ilt(a, b))` is `ige(a, b)`), and
/// the pass counter's `count - 1`, its compare with 0, is OR-ed into it
/// ([`vs_kept_begin`]). The `if`'s then side is the `break`, its else side
/// empty; the rest of the body follows in the join, as the fragment walk's
/// `walk_if` builds the head of a kept loop.
fn vs_kept_head(vs: &mut Nir<'_>, s: &mut KeptLoop, c: Val, locals: &[[Val; 4]; MAX_LOCALS]) -> Result<(), MaliError> {
    let Val::Lane(n, 0) = c else { return Err(MaliError::LoopVertex) };
    let nd = vs.nodes[n as usize];
    let inv = match nd.kind {
        Kind::Alu(Alu::Icmp(Cmpf::Lt)) => Cmpf::Ge,
        Kind::Alu(Alu::Icmp(Cmpf::Ge)) => Cmpf::Lt,
        Kind::Alu(Alu::Icmp(Cmpf::Eq)) => Cmpf::Ne,
        Kind::Alu(Alu::Icmp(Cmpf::Ne)) => Cmpf::Eq,
        /* a float or unsigned test has no vertex reference */
        _ => return Err(MaliError::LoopVertex),
    };
    let test = vs.alu(Alu::Icmp(inv), nd.src[0], nd.src[1], Val::Undef)?;
    let next = vs.alu(Alu::Iadd, Val::Lane(s.cap_phi, 0), Val::Const(u32::MAX), Val::Undef)?;
    let stop = vs.alu(Alu::Icmp(Cmpf::Lt), next, Val::Const(0), Val::Undef)?;
    let cond = vs.alu(Alu::Ior, test, stop, Val::Undef)?;
    s.next = next;
    let before = vs.cur;
    let then_first = vs.new_block(before)?;
    let else_first = vs.new_block(before)?;
    let after = vs.new_block(before)?;
    vs.cur = after;
    s.brk = then_first;
    s.brk_locals = *locals;
    let seq = vs.nseq;
    vs.nseq += 1;
    vs.ifs[vs.nifs] = IfRegion {
        cond,
        before,
        then_first,
        then_last: then_first,
        else_first,
        else_last: else_first,
        after,
        live: true,
        guard: false,
        brk: Some(s.lp),
        seq,
        single: false,
        inot: false,
    };
    vs.nifs += 1;
    Ok(())
}

/// The end of kept loop `s`'s body: the header's phis take the body's last
/// values (the latch's), the exit block follows, entered by the head
/// test's `break` alone, with the locals there. A load or look-up first
/// made in the loop (its node would stand in a block the exit does not
/// follow) has no reference.
fn vs_kept_end(vs: &mut Nir<'_>, s: &KeptLoop, locals: &mut [[Val; 4]; MAX_LOCALS]) -> Result<(), MaliError> {
    if s.next == Val::Undef {
        return Err(MaliError::LoopVertex);
    }
    for id in s.first..vs.n {
        if matches!(vs.nodes[id].kind, Kind::Input { .. } | Kind::VsTex { .. } | Kind::UniformIdx { .. })
            && vs.block[id] != s.pre
        {
            return Err(MaliError::LoopVertex);
        }
    }
    let latch = vs.cur;
    for &(id, l, c) in &s.phis[..s.nphi] {
        let v = locals[l as usize][c as usize];
        /* a lane the body leaves as it found it (`nir_opt_remove_phis`) */
        if v == Val::Lane(id, 0) || v == Val::Undef {
            return Err(MaliError::LoopVertex);
        }
        vs.nodes[id as usize].src[1] = v;
    }
    vs.nodes[s.cap_phi as usize].src[1] = s.next;
    let exit = vs.new_block(s.brk)?;
    vs.cur = exit;
    *locals = s.brk_locals;
    let seq = vs.nseq;
    vs.nseq += 1;
    vs.loops[s.lp as usize] = LoopRegion { pre: s.pre, header: s.header, latch, exit, seq };
    Ok(())
}

/// The IR values an invariant output reads, one flag per value: glsl's
/// `propagate_invariance` makes every variable an invariant output's
/// assignment reads invariant too, and glsl_to_nir makes the assignments
/// to them exact. A local read is every store to it. The pragma's
/// outputs are refused before this matters ([`build`]).
fn invariant_values<'s>(m: &Module<'_>, scratch: &Arena<'s>) -> Result<&'s mut [bool], MaliError> {
    let marks = scratch.alloc_filled(m.instrs.len(), false).map_err(|_| MaliError::OutOfScratch)?;
    for (i, op) in m.instrs.iter().enumerate() {
        if let IrOp::StoreOutput { slot, .. } = op {
            marks[i] = m.outputs.iter().any(|o| o.is_invariant() && o.slot() == *slot);
        }
    }
    loop {
        let mut changed = false;
        for i in (0..m.instrs.len()).rev() {
            if !marks[i] {
                continue;
            }
            let op = &m.instrs[i];
            let mut mark = |v: ValueId| {
                if let Some(x) = marks.get_mut(v.0 as usize).filter(|x| !**x) {
                    *x = true;
                    changed = true;
                }
            };
            op.for_each_src(|s| mark(s.val));
            if let IrOp::LoadLocal { local, .. } = *op {
                for (j, o) in m.instrs.iter().enumerate() {
                    if matches!(*o, IrOp::StoreLocal { local: l, .. } if l == local) {
                        mark(ValueId(j as u32));
                    }
                }
            }
        }
        if !changed {
            return Ok(marks);
        }
    }
}

/// The vertex walk of [`build`]; `numbering` models Mesa's IR where it
/// numbers the attribute locations ([`Nir::numbering_walk`]).
fn walk<'s>(
    m: &Module<'_>,
    scratch: &Arena<'s>,
    reads: Option<&[(u8, u8)]>,
    moved: &[super::motion::Moved],
    fp16: &dyn Fn(&str) -> bool,
    numbering: bool,
) -> Result<Nir<'s>, MaliError> {
    if m.instrs.len() > MAX_VALUES {
        return Err(MaliError::TooManyValues);
    }
    if m.locals.len() > MAX_LOCALS {
        return Err(MaliError::TooManyLocals);
    }
    let mut vs = Nir::new(scratch)?;
    vs.numbering_walk = numbering;
    let vals = scratch.alloc_filled(MAX_VALUES, [Val::Undef; 4]).map_err(|_| MaliError::OutOfScratch)?;
    let w = &mut Walk { vals, locals: [[Val::Undef; 4]; MAX_LOCALS] };
    let mut position: Option<[Val; 4]> = None;
    let mut point_size: Option<Val> = None;
    let mut varyings: [Option<[Val; 4]>; MAX_VARYINGS] = [None; MAX_VARYINGS];
    /* a flattened `if` ([`vs_steps`]): its condition, the state before it
     * and its then side's; `phis`, the varying lanes it wrote ([`vs_join`]) */
    let mut cond = Val::Undef;
    let (mut pre, mut then) = (None::<VsState>, None::<VsState>);
    let mut phis = 0u64;
    /* the constant lanes of a mediump vector stored beside computed ones,
     * which the vertex shader stores (the `StoreOutput` arm below) */
    let mut kept = 0u64;
    /* those beside fp16 uniform sums alone: flat ([`uniform_sum16`]) */
    let mut kept_uniform = 0u64;
    let mut joined = false;
    /* where Mesa's link resolves each lane (`nir_scalar_resolved`, through
     * `mov` and `vec`): 0 a scalar of its own (the lane's value names it),
     * or 1 + 4 * value + lane, a component of the vector operation `value`
     * ([`vec_origins`]); per IR value, per local and per varying lane */
    let origins = scratch.alloc_filled(MAX_VALUES, [0u16; 4]).map_err(|_| MaliError::OutOfScratch)?;
    let mut local_origins = [[0u16; 4]; MAX_LOCALS];
    let mut varying_origins = [[0u16; 4]; MAX_VARYINGS];
    /* the loop being walked ([`Step::LoopBegin`]) */
    let mut lp = VsLoop::None;
    /* the loops around it, innermost last (a nested counted loop,
     * [`vs_loop_shape`]) */
    let mut outer = [VsLoop::None; 4];
    let mut nouter = 0;
    let mut kept_loop: Option<KeptLoop> = None;
    let steps = vs_steps(m, scratch)?;
    let exact_values = invariant_values(m, scratch)?;
    let mut k = 0;
    while k < steps.len() {
        let step = steps[k];
        k += 1;
        let id = match step {
            Step::Instr(id) => {
                vs.exact_build = exact_values.get(id.0 as usize).copied().unwrap_or(false);
                id
            }
            Step::Refuse(e) => return Err(e),
            /* a counted loop (its head test folds on the counter's
             * constants) is unrolled, pass after pass, as `simple_unroll`
             * does (nir_opt_loop_unroll.c:105; gm2_intloop_const_vs); one
             * whose test reads a run-time value is kept ([`vs_kept_begin`];
             * gm2_cf_loop_uni_vs) */
            Step::LoopBegin(body, end) => {
                let Some(head) = m.nodes_of(body).iter().find_map(|n| match *n {
                    IrNode::If { cond, .. } => Some(cond),
                    _ => None,
                }) else {
                    return Err(MaliError::Defect(Defect::IrIndex));
                };
                let guess = super::nir_fs::guessed_trips(m, m.nodes_of(body));
                let counted = super::nir_fs::ir_folds(m, head.val, &w.locals, 0) || guess.is_some();
                /* a loop in a loop: both unrolled, or no reference */
                if !matches!(lp, VsLoop::None) {
                    if matches!(lp, VsLoop::Kept) || !counted || nouter == outer.len() {
                        return Err(MaliError::LoopVertex);
                    }
                    outer[nouter] = lp;
                    nouter += 1;
                }
                lp = if counted {
                    VsLoop::Unroll { end: end as usize, pass: 0, first: vs.n, guess }
                } else {
                    kept_loop = Some(vs_kept_begin(&mut vs, m, body, &mut w.locals)?);
                    VsLoop::Kept
                };
                continue;
            }
            Step::Head(src) => {
                let mut c = lane(w, src, 0);
                if let (VsLoop::Unroll { guess: Some(trips), .. }, false, Some(&IrOp::Alu(AluOp::Ilt, [a, _, _]))) =
                    (lp, matches!(c, Val::Const(_)), m.def(src.val))
                {
                    if let Val::Const(i) = lane(w, a, 0) {
                        c = Val::Const(if (i as i32) < trips as i32 { u32::MAX } else { 0 });
                    }
                }
                match (lp, kept_loop.as_mut()) {
                    /* the counter's test, folded: past the last pass the
                     * loop is left, the rest of that pass dead */
                    (VsLoop::Unroll { end, first, .. }, _) if c == Val::Const(0) => {
                        if vs.n - first > super::nir_fs::LOOP_UNROLL_LIMIT * super::nir_fs::MAX_UNROLL_ITERATIONS {
                            return Err(MaliError::LoopVertex);
                        }
                        lp = VsLoop::None;
                        if nouter > 0 {
                            nouter -= 1;
                            lp = outer[nouter];
                        }
                        k = end + 1;
                    }
                    (VsLoop::Unroll { .. }, _) if matches!(c, Val::Const(_)) => {}
                    (VsLoop::Kept, Some(s)) if s.next == Val::Undef => vs_kept_head(&mut vs, s, c, &w.locals)?,
                    /* a test that does not fold in a pass of an unrolled
                     * loop (`complex_unroll`) has no vertex reference */
                    _ => return Err(MaliError::LoopVertex),
                }
                continue;
            }
            Step::LoopEnd(begin) => {
                match lp {
                    /* `simple_unroll` takes a trip count of at most
                     * `max_unroll_iterations` (bifrost_compile.h:140) */
                    VsLoop::Unroll { ref mut pass, .. } => {
                        *pass += 1;
                        if *pass > super::nir_fs::MAX_UNROLL_ITERATIONS {
                            return Err(MaliError::LoopVertex);
                        }
                        k = begin as usize + 1;
                    }
                    VsLoop::Kept => {
                        let s = kept_loop.take().ok_or(MaliError::Defect(Defect::IrIndex))?;
                        vs_kept_end(&mut vs, &s, &mut w.locals)?;
                        lp = VsLoop::None;
                    }
                    VsLoop::None => return Err(MaliError::Defect(Defect::IrIndex)),
                }
                continue;
            }
            Step::If(c) => {
                cond = lane(w, c, 0);
                pre = Some(VsState { locals: w.locals, position, point_size, varyings });
                continue;
            }
            Step::Else => {
                let p = pre.ok_or(MaliError::Defect(Defect::IrIndex))?;
                then = Some(VsState { locals: w.locals, position, point_size, varyings });
                (w.locals, position, point_size, varyings) = (p.locals, p.position, p.point_size, p.varyings);
                continue;
            }
            Step::EndIf => {
                let (p, t) = pre.zip(then).ok_or(MaliError::Defect(Defect::IrIndex))?;
                let mut e = VsState { locals: w.locals, position, point_size, varyings };
                match cond {
                    /* a folded condition: the side it picks */
                    Val::Const(c) => {
                        if c != 0 {
                            e = t;
                        }
                    }
                    _ => {
                        vs_join(&mut vs, cond, &p, &t, &mut e, &mut phis)?;
                        joined = true;
                    }
                }
                (w.locals, position, point_size, varyings) = (e.locals, e.position, e.point_size, e.varyings);
                continue;
            }
        };
        let op = m.instr(id).ok_or(MaliError::Defect(Defect::IrIndex))?;
        let width = m.comps_of(ValueId(id.0)) as usize;
        let mut out = [Val::Undef; 4];
        /* a uniform array read at an index the walk folded to a constant
         * (an unrolled loop's counter, [`VsLoop::Unroll`]): Mesa's
         * unrolling makes the deref's index a constant before
         * `nir_lower_uniforms_to_ubo`, so the read is the constant-index one
         * (deqp_uniarray_loop_float_vs, byte-equal to float_const_vs; the
         * fragment walk's rule, nir_fs.rs `walk_nodes`); the index clamped
         * as the IR reads it */
        let folded;
        let op = match *op {
            IrOp::LoadUniformIdx { base_dw, stride_dw, len, index, comps } if len > 0 => match lane(w, index, 0) {
                Val::Const(k) => {
                    let k = (k as i32).clamp(0, i32::from(len) - 1) as u16;
                    folded = IrOp::LoadUniform { dword_off: base_dw + k * stride_dw, comps };
                    &folded
                }
                _ => op,
            },
            _ => op,
        };
        match *op {
            IrOp::ConstF32(bits) => out = bits.map(Val::Const),
            /* a 32-bit integer constant is its bits (NIR's `load_const`
             * has no type); vid.vert's `% 4` and `== k`, inst.vert's `== 3` */
            IrOp::ConstI32(bits) => out = bits.map(|b| Val::Const(b as u32)),
            /* a boolean literal is NIR's 0 / ~0 (`nir_const_value` of a
             * 1-bit `true` once `pan_nir_lower_bool_to_bitsize` widens it;
             * nir_fs.rs folds it the same): the rsg programs' `true` and
             * `bool(1)` fold through [`fold_int`] and the `bcsel` arm
             * (deqp_intval_rsg_conv_vs; plan #377 lower-intval-2) */
            IrOp::ConstBool(bits) => out = bits.map(|b| Val::Const(if b { u32::MAX } else { 0 })),
            IrOp::LoadInput { slot: IoSlot::Attrib(loc), comps } => {
                let n = vs.load(Kind::Input { loc, comps })?;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, c as u8);
                }
            }
            /* gl_VertexID and gl_InstanceID: the preloaded r60 and r61
             * (builtin_43, builtin_47.expect.json: `LSHIFT_AND.i32 r0, r60^,
             * ..`, `ICMP_OR.u32.eq.m1 r4, r61^, ..`) */
            IrOp::LoadInput { slot: slot @ (IoSlot::VertexId | IoSlot::InstanceId), .. } => {
                let reg = if slot == IoSlot::VertexId { super::ra::VERTEX_ID } else { super::ra::INSTANCE_ID };
                out[0] = Val::Lane(vs.load(Kind::Preload { reg })?, 0);
            }
            IrOp::LoadInput { .. } => return Err(MaliError::SpecialInput),
            IrOp::LoadUniform { dword_off, comps } => {
                let (u, total) = uniform_of(m, dword_off)?;
                if u.gl_type as u32 == GL_INT
                    && !index_only(m, ValueId(id.0))
                    && !int_compare_only(m, ValueId(id.0))
                    && !int_float_only(m, ValueId(id.0))
                {
                    return Err(MaliError::UniformType);
                }
                /* the load Mesa makes: the whole vector, or one column, or
                 * one element of a vec4 array */
                let (base, size) = if let Some((_, rows, stride)) = matrix_cols(u.gl_type) {
                    (u.dword_off + (dword_off - u.dword_off) / stride * stride, rows)
                } else if let Some(load) = element_load(u, dword_off) {
                    load
                } else {
                    (u.dword_off, total)
                };
                let first = dword_off - base;
                if first + comps as u16 > size {
                    return Err(MaliError::Defect(Defect::IrIndex));
                }
                if fp16_uniform(u, fp16) && array_element(u).is_some() {
                    /* an fp16 array element: each word its own load (its
                     * offset is no constant to `nir_lower_uniforms_to_ubo`,
                     * so the alignment is 4 and the load is cut a word a
                     * piece, select.rs `UboLayout::load_of`), a node of two
                     * lanes a word, each lane widened by its own `f2f32`
                     * where an fp32 operation reads it (deqp_uniarray_vec4_const_vs:
                     * `FADD.v2f16 r0, u6.w1, r0` of the zw words) */
                    for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                        let k = first as usize + c;
                        let word = (k / 2 * 2) as u16;
                        let n = vs.load16(Kind::Uniform { dword: base + word, comps: (size - word).min(2) as u8 })?;
                        *o = vs.alu(Alu::F2f32, Val::Lane(n, (k % 2) as u8), Val::Undef, Val::Undef)?;
                    }
                } else if fp16_uniform(u, fp16) {
                    /* a mediump float uniform is fp16 in the block
                     * (lower_precision.cpp:1023-1060 with
                     * `fp16_const_buffers`), each lane read widened by its
                     * own `f2f32` (`F16_TO_F32 r4, u4.w1.h1`,
                     * vsfp16_19_hello.expect.json; docs/notes/mali-precision.md) */
                    let n = vs.load16(Kind::Uniform { dword: base, comps: size as u8 })?;
                    for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                        let l = Val::Lane(n, (first as usize + c) as u8);
                        *o = vs.alu(Alu::F2f32, l, Val::Undef, Val::Undef)?;
                    }
                } else {
                    let n = vs.load(Kind::Uniform { dword: base, comps: size as u8 })?;
                    for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                        *o = Val::Lane(n, (first as usize + c) as u8);
                    }
                }
            }
            /* a float array's element at an int uniform: Mesa's 32-bit
             * `load_ubo(0, base + (index << 2))`, `LSHIFT_OR.i32`,
             * `IADD_IMM.i32` and `LD_PKA.i32` (uniarray_vs.expect.json's
             * varying program, docs/notes/mali-uniform-arrays.md) */
            /* a mediump vec2, vec3 or vec4 array's fp16 element at an int
             * uniform ([`uniform_idx16`]): each lane widened by its own
             * `f2f32` where an fp32 operation reads it, as a constant
             * index's element (deqp_uniarray_dyn_vec{2,3,4}_vs) */
            IrOp::LoadUniformIdx { base_dw, index, comps, .. }
                if uniform_of(m, base_dw).is_ok_and(|(u, _)| u.array_len > 1 && fp16_uniform(u, fp16)) =>
            {
                let n = uniform_idx16(&mut vs, m, base_dw, comps, lane(w, index, 0))?;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = vs.alu(Alu::F2f32, Val::Lane(n, c as u8), Val::Undef, Val::Undef)?;
                }
            }
            IrOp::LoadUniformIdx { base_dw, index, comps, .. } => {
                let n = uniform_idx(&mut vs, m, base_dw, comps, lane(w, index, 0))?;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, c as u8);
                }
            }
            IrOp::LoadLocal { local, index: None, comps } => {
                let s = w.locals.get(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))?;
                out[..comps as usize].copy_from_slice(&s[..comps as usize]);
            }
            IrOp::StoreLocal { local, index: None, src, write_mask } => {
                let mut lanes = [Val::Undef; 4];
                for (i, t) in lanes.iter_mut().enumerate() {
                    *t = lane(w, src, i);
                }
                /* a mediump local stores an integer's float narrowed
                 * (lower_precision: `float(res)` of the operator suite's
                 * `mediump vec4 color`, [`vs_narrow_stored`]); a boolean's
                 * float (`b2f`, no precision) and a constant (fp32 bits in
                 * the walk, fp16 where a 16-bit operation reads it) stay as
                 * they are; a highp local widens a 16-bit lane */
                let medium = m.local_prec_of(local) == Prec::Medium;
                for (i, t) in lanes.iter_mut().enumerate() {
                    if write_mask & (1 << i) == 0 {
                        continue;
                    }
                    *t = match (medium, vs_alu_of(&vs, *t)) {
                        (true, Some((Alu::I2f | Alu::U2f, _))) if !vs_is_f16(&vs, *t) => vs_narrow_stored(&mut vs, *t)?,
                        (false, _) => vs_widen(&mut vs, *t)?,
                        _ => *t,
                    };
                }
                let s = w.locals.get_mut(local.0 as usize).ok_or(MaliError::Defect(Defect::IrIndex))?;
                for (i, t) in lanes.iter().enumerate() {
                    if write_mask & (1 << i) != 0 {
                        s[i] = *t;
                    }
                }
            }
            IrOp::LoadLocal { .. } | IrOp::StoreLocal { .. } => return Err(MaliError::IndexedLocal),
            IrOp::StoreOutput { slot, src } => {
                let mut lanes = [Val::Undef; 4];
                for (i, t) in lanes.iter_mut().enumerate() {
                    *t = lane(w, src, i);
                }
                /* a mediump vector stored to an output is widened whole
                 * (`f2f32` of the vector, lower_precision): with a computed
                 * lane among them, Mesa's link sees every lane of it
                 * computed (`nir_scalar_is_const` does not see through the
                 * conversion, and `nir_lower_alu_width` cuts it only after
                 * the link), so a constant lane is neither propagated into
                 * the fragment shader nor made flat: the vertex shader
                 * stores it, as the fp16 value it holds (`kept`;
                 * deqp_intval_opvs_add_int_highp's `IADD_IMM.i32 r2, 0x0,
                 * #0x3F19A000`, the 0.6 rounded to fp16). A vector whose
                 * computed lanes read no attribute is convergent whole, and
                 * the link's rule for it is the flat one below
                 * (deqp_misc_spill_*, deqp_unitype_vsmat3_lowp: uniforms
                 * alone beside constants; Mesa keeps vsmat3_lowp's 1.0 in
                 * its flat `.v4` too, the fp16 sums' `f2f32` hiding it,
                 * which the link's refusal of a kept lane from no
                 * attribute stands before). Each 16-bit lane is its own
                 * `f2f32` (`F16_TO_F32 r1, r0.h0`), in lane order at the
                 * store. */
                let medium = m.prec_of(src.val) == Prec::Medium
                    || matches!(m.def(src.val), Some(IrOp::LoadLocal { local, .. }) if m.local_prec_of(*local) == Prec::Medium);
                /* or of fp16 uniforms alone (deqp_uniarray_*_const_vs's
                 * `vec4(res, 0.0, 1.0)` of a mediump array's sum: the
                 * constants stored in the flat `.v4` beside it, `IADD_IMM.i32
                 * r4, 0x0, #0x0`, `kept_flat` below) */
                let attr = lanes.iter().any(|l| matches!(l, Val::Lane(n, _) if vs.reads_attribute(*n)));
                let usum = lanes.iter().any(|l| matches!(l, Val::Lane(n, _) if uniform_sum16(&vs, *n)));
                let computed = attr || usum;
                for (i, t) in lanes.iter_mut().enumerate() {
                    match (*t, slot) {
                        (Val::Const(c), IoSlot::Varying(loc)) if medium && computed => {
                            *t = Val::Const(as_f16(c)?);
                            kept |= 1 << (loc as usize * 4 + i);
                            if !attr {
                                kept_uniform |= 1 << (loc as usize * 4 + i);
                            }
                        }
                        _ => *t = vs_widen(&mut vs, *t)?,
                    }
                }
                match slot {
                    IoSlot::Position => position = Some(lanes),
                    IoSlot::Varying(loc) => {
                        *varyings.get_mut(loc as usize).ok_or(MaliError::VaryingLocation)? = Some(lanes)
                    }
                    IoSlot::PointSize => {
                        vs.note(Prod::PointSize);
                        point_size = Some(lanes[0]);
                    }
                    _ => return Err(MaliError::FragOutput),
                }
            }
            /* a product or sum the front end classed mediump over a
             * mediump local's narrowed lanes and constants: fp16, in lane
             * pairs ([`vs_alu16`]) */
            IrOp::Alu(a @ (AluOp::Fmul | AluOp::Fadd), s)
                if m.prec_of(ValueId(id.0)) == Prec::Medium && vs_pairs16(&vs, m, w, &s, width) =>
            {
                vs_alu16(&mut vs, a, m, w, &s, width, &mut out)?;
            }
            /* one over fp16 uniforms and mediump attributes: fp16 too, the
             * uniform's words read whole ([`vs_alu16u`]) */
            IrOp::Alu(a @ (AluOp::Fmul | AluOp::Fadd | AluOp::Ffma | AluOp::Fneg), s)
                if m.prec_of(ValueId(id.0)) == Prec::Medium && vs_uniform16(&vs, m, w, a, &s, width) =>
            {
                vs_alu16u(&mut vs, a, m, w, &s, width, &mut out)?;
            }
            /* `abs` of a mediump local's lanes (mediump attributes'): fp16,
             * one lane at a time ([`vs_fabs16`]) */
            IrOp::Alu(AluOp::Fabs, s) if m.prec_of(ValueId(id.0)) == Prec::Medium && vs_abs16(&vs, m, w, &s, width) => {
                vs_fabs16(&mut vs, m, w, &s, width, &mut out)?;
            }
            /* `fract` of the same lanes: fp16 too ([`vs_ffract16`]) */
            IrOp::Alu(AluOp::Ffract, s)
                if m.prec_of(ValueId(id.0)) == Prec::Medium && vs_abs16(&vs, m, w, &s, width) =>
            {
                vs_ffract16(&mut vs, m, w, &s, width, &mut out)?;
            }
            IrOp::Alu(a, s) => {
                /* a mediump operation of a lowp or mediump sampler's texels
                 * other than the fp16 arms' is fp16 in Mesa too, a form
                 * this fp32 walk does not build (mali-vs-lookups.md §7) */
                if m.prec_of(ValueId(id.0)) == Prec::Medium
                    && (0..super::nir_fs::ir_arity(a)).any(|k| {
                        (0..width.max(1)).any(|i| {
                            matches!(lane(w, s[k], i), Val::Lane(n, _)
                                if matches!(vs.nodes[n as usize].kind, Kind::VsTex { medium: true, .. }))
                        })
                    })
                {
                    return Err(MaliError::TexVertex);
                }
                /* lanes an arm makes for every lane at once, at lane 0 */
                let mut held = [Val::Undef; 4];
                let split = if a == AluOp::Fmul { split_const(w, &s, width) } else { [false; 2] };
                /* the operands' lanes, a 16-bit one widened ([`vs_widen`]) */
                let mut xs = [[Val::Undef; 4]; 3];
                for (k, xk) in xs.iter_mut().enumerate().take(super::nir_fs::ir_arity(a)) {
                    for (i, x) in xk.iter_mut().enumerate().take(width.max(1)) {
                        *x = vs_widen(&mut vs, lane(w, s[k], i))?;
                    }
                }
                for (l, o) in out.iter_mut().enumerate().take(width) {
                    let x = |k: usize| xs[k][l];
                    /* a lane past an operand's: the front end gives a
                     * built-in or a `++` of a narrower local the width of
                     * its result (`abs` of a `vec2` local, four lanes), and
                     * Mesa's operation has the operand's width, so nothing
                     * is computed for the lane (deqp_varylink_op_abs_vec2:
                     * the `abs` of two lanes only). A reduction reads its
                     * operands whole at lane 0 */
                    let reduces = matches!(a, AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4 | AluOp::Bany | AluOp::Ball);
                    if !reduces && (0..super::nir_fs::ir_arity(a)).any(|k| x(k) == Val::Undef) {
                        *o = Val::Undef;
                        continue;
                    }
                    /* a mediump sum or product of two constants: fp16 at
                     * each step, every constant rounded to half and every
                     * result too (lower_precision, then
                     * `nir_opt_constant_folding` at 16 bits; [`fold16`]).
                     * deqp_constop_mat3sub_vs's `in0 - in1` and the sum of
                     * its columns: x is -2^-11 (`0x9000`) where fp32 gives
                     * -2.98e-8, which no fp16 holds. A result fp16 holds
                     * only as a subnormal keeps the fp32 fold */
                    if let (AluOp::Fadd | AluOp::Fmul, Val::Const(p), Val::Const(q)) = (a, x(0), x(1)) {
                        if m.prec_of(ValueId(id.0)) == Prec::Medium {
                            if let Ok(c) = fold16(a, p, q) {
                                *o = Val::Const(c);
                                continue;
                            }
                        }
                    }
                    *o = match a {
                        AluOp::Fadd => vs_fadd(&mut vs, m, &s, x(0), x(1))?,
                        AluOp::Fneg => vs.alu(Alu::Fneg, x(0), Val::Undef, Val::Undef)?,
                        /* `a / b` is `a * rcp(b)` from the front end */
                        AluOp::Frcp => vs.alu(Alu::Frcp, x(0), Val::Undef, Val::Undef)?,
                        /* a lane past the source's: the front end gives
                         * `normalize`'s product of a vec3 and its `rsq` four
                         * lanes (builtin_call.rs), the last undefined */
                        AluOp::Fmul if x(0) == Val::Undef || x(1) == Val::Undef => Val::Undef,
                        /* `smoothstep`'s last product: the front end's `(t *
                         * t) * (3.0 - 2.0 * t)` is Mesa's `t * (t * (3.0 -
                         * 2.0 * t))` (builtin_functions.cpp `_smoothstep`, as
                         * nir_fs.rs; gm2_fma_3..5: `FMA.f32 r2, r1, r2^,
                         * 0x0.neg`, then `FMA.f32 r1, r1^, r2^, 0x0.neg`) */
                        AluOp::Fmul if vs_smoothstep_tail(&vs, x(0), x(1)).is_some() => {
                            let t = vs_smoothstep_tail(&vs, x(0), x(1)).ok_or(MaliError::Defect(Defect::IrIndex))?;
                            let ti = vs.alu(Alu::Fmul, t, x(1), Val::Undef)?;
                            vs.alu(Alu::Fmul, t, ti, Val::Undef)?
                        }
                        /* a constant over a product read once (:2393); not
                         * a lane whose zero only the back end folds
                         * ([`split_const`], [`vs_fmul`]). The other lanes of
                         * such a vector take the rule once the back end has
                         * scalarised them (deqp_varylink_draw_multi's
                         * `vec4(u * color, 1.0) * 0.5`: `u * 0.5` first,
                         * `FMA.f32 r0, u3.w1, 0x38000000.h1, r0^.neg`, then
                         * each lane's `FMA.f32 r, r0, a, 0.5`) */
                        AluOp::Fmul
                            if m.prec_of(ValueId(id.0)) == Prec::Medium
                                && vs_cmp_side(&vs, x(0)).is_some()
                                && vs_cmp_side(&vs, x(1)).is_some() =>
                        {
                            match vs_cmp_and(&mut vs, x(0), x(1))? {
                                Some(v) => v,
                                None => vs_fmul(&mut vs, split, x(0), x(1))?,
                            }
                        }
                        AluOp::Fmul if split == [false; 2] || !(is_zero(x(0)) || is_zero(x(1))) => {
                            match vs_fmul_const_up(&mut vs, m, &s, x(0), x(1))? {
                                Some(v) => v,
                                None => vs_fmul(&mut vs, split, x(0), x(1))?,
                            }
                        }
                        AluOp::Fmul => vs_fmul(&mut vs, split, x(0), x(1))?,
                        /* a matrix product's last term by a `w` of 1.0
                         * (`M * vec4(p, 1.0)`): `fmul(c3, 1.0)` is `c3`
                         * (nir_opt_algebraic.py:292) and the term `fadd(acc,
                         * c3)`, the sum first as GLSL's `add(acc, term)`
                         * (S1 of docs/notes/mali-vs-constant-products.md,
                         * vsfold_one.expect.json's `FADD.f32 r1, r1^,
                         * u9.w0`). A sum of two terms is S2, which moves the
                         * column into the first term, every lane's
                         * `ffma(c0, x, c3)` first, then every lane's
                         * `ffma(c1, y, ..)` (made at lane 0, `held`;
                         * gm2_arithconst_1's `vec4(position, 0.0, 1.0)`,
                         * [`vs_two_terms`]) */
                        AluOp::Ffma
                            if is_mat_term(m, &s) && mat_column(m, &s) == Some(3) && x(1) == Val::Const(F32_ONE) =>
                        {
                            vs.note(Prod::MatProduct);
                            if l == 0 && vs_two_terms(&vs, x(2)).is_some() {
                                let mut terms = [[Val::Undef; 4]; 4];
                                for (i, t) in terms.iter_mut().enumerate().take(width) {
                                    *t = vs_two_terms(&vs, lane(w, s[2], i)).ok_or(MaliError::ArithConstant)?;
                                }
                                let mut first = [Val::Undef; 4];
                                for (i, f) in first.iter_mut().enumerate().take(width) {
                                    *f = vs.alu(Alu::Ffma, terms[i][0], terms[i][1], lane(w, s[0], i))?;
                                }
                                for (i, h) in held.iter_mut().enumerate().take(width) {
                                    *h = vs.alu(Alu::Ffma, terms[i][2], terms[i][3], first[i])?;
                                }
                            }
                            match held[l] {
                                Val::Undef if vs_two_terms(&vs, x(2)).is_some() => {
                                    return Err(MaliError::ArithConstant)
                                }
                                Val::Undef => vs.alu(Alu::Fadd, x(2), x(0), Val::Undef)?,
                                h => h,
                            }
                        }
                        /* any other term by 1.0 or -1.0 (a `w` of -1.0, a
                         * `z` of 1.0): NIR's rules
                         * (:311-314) meet it inside the vector sums of the
                         * product, which no reference draws in this stage */
                        AluOp::Ffma
                            if is_mat_term(m, &s)
                                && [x(0), x(1)].iter().any(|v| matches!(v, Val::Const(F32_ONE | F32_MINUS_ONE))) =>
                        {
                            return Err(MaliError::ArithConstant);
                        }
                        /* a mat3's or mat2's column has no lane past its rows
                         * (the front end's term is four lanes wide) */
                        AluOp::Ffma if is_mat_term(m, &s) && (x(0) == Val::Undef || x(2) == Val::Undef) => Val::Undef,
                        AluOp::Ffma if is_mat_term(m, &s) => {
                            vs.note(Prod::MatProduct);
                            vs.alu(Alu::Ffma, x(0), x(1), x(2))?
                        }
                        /* the front end's `mod(x, y)`: `FMA.f32 r0,
                         * y.neg, floor, x` (gl_test case 221, gm2_fma_6) */
                        AluOp::Ffma if is_fmod(m, &s) => vs_fmod(&mut vs, x(2), x(0), x(1))?,
                        /* `cross(a, b)`: the front end's `ffma(a.yzx, b.zxy,
                         * fneg(fmul(a.zxy, b.yzx)))` is what Mesa's late
                         * fusion makes of GLSL's `a.yzx * b.zxy - a.zxy *
                         * b.yzx`, the first product fused and the `fneg` the
                         * addend's `.neg` (vsfma_cross.expect.json,
                         * docs/notes/mali-vs-fma.md) */
                        AluOp::Ffma if is_cross(m, &s) => vs.alu(Alu::Ffma, x(0), x(1), x(2))?,
                        /* `mix(a, b, t)`: Mesa's `lower_flrp` makes `a + t
                         * * (b - a)`, the front end's `ffma(t, b - a, a)`
                         * (gm2_fma_7: every lane's `FADD.f32 d, b, a.neg`,
                         * then `FMA.f32 r, t, d, a`); `smoothstep`'s `3.0 -
                         * 2.0 * t`, `FMA.f32 r, 0x40000000.neg, t, 3.0`
                         * (gm2_fma_3..5) */
                        AluOp::Ffma if is_mix(m, &s) || is_smoothstep_inner(m, &s) => {
                            vs.alu(Alu::Ffma, x(0), x(1), x(2))?
                        }
                        /* `reflect(I, N)`: `dot(N, I) * -2.0` is
                         * `fneg(fadd(d, d))` (`fmul(a, 2.0)`, then the
                         * `fneg` taken up the product), the `fneg` the
                         * `.neg` of each lane's `FMA.f32 r, d.neg, N, I`
                         * (gm2_fma_8: `FADD.f32 r4, r4^, r4^`); made once
                         * at lane 0 (`held`) */
                        AluOp::Ffma if is_reflect(m, &s) => {
                            if l == 0 {
                                let Some(IrOp::Alu(_, [d, ..])) = m.def(s[0].val) else {
                                    return Err(MaliError::Defect(Defect::IrIndex));
                                };
                                let d = lane(w, *d, 0);
                                let d2 = vs.alu(Alu::Fadd, d, d, Val::Undef)?;
                                held[0] = vs.alu(Alu::Fneg, d2, Val::Undef, Val::Undef)?;
                            }
                            vs.alu(Alu::Ffma, held[0], x(1), x(2))?
                        }
                        /* of three constants, the fold of a built-in on
                         * constants ([`Nir::alu`]'s unfused `a * b + c`):
                         * the front end's `atan` polynomial of a constant
                         * (deqp_constrnd_atan_zero_lanes_vs) */
                        AluOp::Ffma if (0..3).all(|i| matches!(x(i), Val::Const(_))) => {
                            vs.alu(Alu::Ffma, x(0), x(1), x(2))?
                        }
                        AluOp::Ffma => return Err(MaliError::FusedMultiplyAdd),
                        AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4 if l == 0 => {
                            vs.note(Prod::Dot);
                            let n = match a {
                                AluOp::Fdot2 => 2,
                                AluOp::Fdot3 => 3,
                                _ => 4,
                            };
                            let (va, vb) = (|i| lane(w, s[0], i), |i| lane(w, s[1], i));
                            if let Some(c) = const_dot(n, va, vb)? {
                                vs.note_user(Prod::Folded);
                                c
                            } else if let Some(d) = vs_dot_factored(&mut vs, m, w, &s, n)? {
                                d
                            } else {
                                let mut acc = vs.alu(Alu::Fmul, va(n - 1), vb(n - 1), Val::Undef)?;
                                for i in (0..n - 1).rev() {
                                    acc = vs.alu(Alu::Ffma, va(i), vb(i), acc)?;
                                }
                                acc
                            }
                        }
                        AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4 => Val::Undef,
                        /* `imod(a, 2^k)` is `iand(a, 2^k - 1)`
                         * (nir_opt_algebraic.py's power-of-two rule;
                         * builtin_43's `LSHIFT_AND.i32 r0, r60^, 0x0.b00,
                         * u5.w1`, the 3 a FAU word) */
                        AluOp::Imod => match x(1) {
                            Val::Const(b) if (b as i32) > 0 && b.is_power_of_two() => {
                                vs.alu(Alu::Iand, x(0), Val::Const(b - 1), Val::Undef)?
                            }
                            _ => return Err(MaliError::IntValue),
                        },
                        /* `ieq(b, true)` is `b` and `ieq(b, false)` is
                         * `inot(b)` (nir_opt_algebraic.py, as nir_fs.rs's
                         * int_lane): dEQP's `compare_bool(u_var, true)`
                         * (deqp_intval_uniapi_bool_vs) */
                        AluOp::Ieq | AluOp::Ine
                            if (matches!(x(1), Val::Const(0 | u32::MAX)) && vs_is_bool(&vs, x(0)))
                                || (matches!(x(0), Val::Const(0 | u32::MAX)) && vs_is_bool(&vs, x(1))) =>
                        {
                            let (b, c) = if matches!(x(1), Val::Const(_)) { (x(0), x(1)) } else { (x(1), x(0)) };
                            if (c == Val::Const(u32::MAX)) == (a == AluOp::Ieq) {
                                b
                            } else {
                                vs.alu(Alu::Inot, b, Val::Undef, Val::Undef)?
                            }
                        }
                        /* `ieq_pan`, a 0 / ~0 mask; `ior` of two masks is
                         * the compares' `ICMP_OR` (opt.rs `fuse_cmp`) */
                        AluOp::Ieq => vs.alu(Alu::Icmp(Cmpf::Eq), x(0), x(1), Val::Undef)?,
                        AluOp::Ior => vs.alu(Alu::Ior, x(0), x(1), Val::Undef)?,
                        /* `bcsel(a, b, b)` is `b` and `bcsel(a, 1.0, 0.0)`
                         * is `b2f32(a)` (nir_opt_algebraic.py), the
                         * `CSEL.u32.eq c, 0, 0, 1.0` of va_lower_isel */
                        AluOp::Bcsel if x(1) == x(2) => x(1),
                        /* `bcsel(inot(a), b, c)` is `bcsel(a, c, b)`
                         * (nir_opt_algebraic.py; deqp_intval_uniapi_bool_initial_vs's
                         * `u_var == false ? 1.0 : 0.0`: the `CSEL.u32.eq`
                         * with its arms swapped) */
                        AluOp::Bcsel if matches!(vs_alu_of(&vs, x(0)), Some((Alu::Inot, _))) => {
                            let c = vs_alu_of(&vs, x(0)).map(|(_, s)| s[0]).unwrap_or(Val::Undef);
                            if x(2) == Val::Const(F32_ONE) && x(1) == Val::Const(F32_ZERO) {
                                vs.alu(Alu::B2f, c, Val::Undef, Val::Undef)?
                            } else {
                                vs.alu(Alu::Bcsel, c, x(2), x(1))?
                            }
                        }
                        AluOp::Bcsel if x(1) == Val::Const(F32_ONE) && x(2) == Val::Const(F32_ZERO) => {
                            vs.alu(Alu::B2f, x(0), Val::Undef, Val::Undef)?
                        }
                        AluOp::Bcsel => vs.alu(Alu::Bcsel, x(0), x(1), x(2))?,
                        /* the clamps of vsarith_182_fsat and _fsatfolded
                         * (docs/notes/mali-vs-compare-clamp.md section 3) */
                        AluOp::Fmax | AluOp::Fmin => vs_clamp(&mut vs, a, x(0), x(1))?,
                        /* `abs` is the `.abs` modifier on its consumer and
                         * `floor` one `FROUND.f32.rtn` (vsarith_181_fmactrl,
                         * docs/notes/mali-vs-abs-if.md) */
                        AluOp::Fabs => vs.alu(Alu::Fabs, x(0), Val::Undef, Val::Undef)?,
                        AluOp::Ffloor => vs.alu(Alu::Ffloor, x(0), Val::Undef, Val::Undef)?,
                        /* `fract(a)` is `fadd(a, fneg(ffloor(a)))`
                         * (nir_opt_algebraic.py's `lower_ffract`, then
                         * `lower_fsub`), rewritten on the vector before
                         * it is scalarised: every lane's `ffloor` first
                         * (made at lane 0, `held`), then each lane's
                         * `fadd`; `FROUND.f32.rtn` and `FADD.f32 a, r.neg`
                         * (vsarith_fract.expect.json, whose pressure
                         * schedule differs in the lane-by-lane order) */
                        AluOp::Ffract if x(0) == Val::Undef => Val::Undef,
                        AluOp::Ffract => {
                            if l == 0 {
                                /* a lane past the operand's (a local vec2
                                 * copy, the front end's four lanes) has no
                                 * `ffloor` (deqp_posvary_fract_*) */
                                for (i, h) in held.iter_mut().enumerate().take(width) {
                                    let a = lane(w, s[0], i);
                                    *h = if a == Val::Undef {
                                        Val::Undef
                                    } else {
                                        vs.alu(Alu::Ffloor, a, Val::Undef, Val::Undef)?
                                    };
                                }
                            }
                            let n = vs.alu(Alu::Fneg, held[l], Val::Undef, Val::Undef)?;
                            vs.alu(Alu::Fadd, x(0), n, Val::Undef)?
                        }
                        /* `inversesqrt` is one `FRSQ.f32`
                         * (vsarith_vrcp.expect.json); `sqrt` is
                         * `frcp(frsq(a))` (`lower_fsqrt`,
                         * nir_opt_algebraic.py:1908, as nir_fs.rs):
                         * `FRSQ.f32` then `FRCP.f32` (gm2_vsarith_6,
                         * glmark2 function's `fract(sqrt(d))`) */
                        AluOp::Frsq => vs.alu(Alu::Frsq, x(0), Val::Undef, Val::Undef)?,
                        AluOp::Fsqrt if x(0) == Val::Undef => Val::Undef,
                        /* of a constant, `sqrtf` (correctly rounded): the
                         * GLSL folder's (ir_expression_operation.py's
                         * `sqrt`) and NIR's `fsqrt` fold, both before
                         * `lower_fsqrt` (deqp_constrnd_sqrt_zero_vs's
                         * `exp2(sqrt(0.0))`; `frcp(frsq(0.0))` is a
                         * reciprocal of an infinity) */
                        AluOp::Fsqrt if matches!(x(0), Val::Const(_)) => {
                            let Val::Const(c) = x(0) else { return Err(MaliError::Defect(Defect::IrIndex)) };
                            vs.note_user(Prod::Folded);
                            Val::Const(fold::fold_float(AluOp::Fsqrt, c, 0, 0).ok_or(MaliError::ConstantFold)?)
                        }
                        AluOp::Fsqrt => {
                            let r = vs.alu(Alu::Frsq, x(0), Val::Undef, Val::Undef)?;
                            vs.alu(Alu::Frcp, r, Val::Undef, Val::Undef)?
                        }
                        /* a lane past the compared vector's (a vec2 or vec3
                         * compared as the front end's four lanes) */
                        AluOp::Feq | AluOp::Fne | AluOp::Flt | AluOp::Fle
                            if x(0) == Val::Undef || x(1) == Val::Undef =>
                        {
                            Val::Undef
                        }
                        /* a compare that only an `any` / `all` reads is not
                         * kept: [`vs_reduce`] reads its sources, and
                         * [`dce`] drops these */
                        AluOp::Feq => vs.alu(Alu::Fcmp(Cmpf::Eq), x(0), x(1), Val::Undef)?,
                        AluOp::Fne => vs.alu(Alu::Fcmp(Cmpf::Ne), x(0), x(1), Val::Undef)?,
                        /* `bool(x)` is `fneu(x, 0.0)` (glsl_to_nir.cpp's
                         * `f2b`); `float(b)` is `bcsel_pan(b, 1.0, 0.0)`
                         * (bifrost_nir_algebraic.py:127), the `CSEL.u32.eq
                         * c, 0, 0, 1.0` of va_lower_isel per lane
                         * (deqp_intval_opvs_less_ivec4's `vec4(bvec4)`);
                         * of a constant both fold ([`fold_int`]) */
                        AluOp::F2b | AluOp::B2f if x(0) == Val::Undef => Val::Undef,
                        AluOp::F2b | AluOp::B2f => match fold_int(a, x(0), x(1)) {
                            Some(v) => v,
                            None if a == AluOp::F2b => {
                                vs.alu(Alu::Fcmp(Cmpf::Ne), x(0), Val::Const(F32_ZERO), Val::Undef)?
                            }
                            None => match vs_alu_of(&vs, x(0)) {
                                /* `b2f(inot(a))` is `bcsel_pan(a, 0.0, 1.0)`
                                 * (bifrost_nir_algebraic.py:129) */
                                Some((Alu::Inot, [c, ..])) => {
                                    vs.alu(Alu::Bcsel, c, Val::Const(F32_ZERO), Val::Const(F32_ONE))?
                                }
                                _ => vs.alu(Alu::B2f, x(0), Val::Undef, Val::Undef)?,
                            },
                        },
                        AluOp::Flt => vs.alu(Alu::Fcmp(Cmpf::Lt), x(0), x(1), Val::Undef)?,
                        /* GLSL's `<=` is `fge` with its sources swapped
                         * (glsl_to_nir.cpp:2488), as nir_fs.rs */
                        AluOp::Fle => vs.alu(Alu::Fcmp(Cmpf::Ge), x(1), x(0), Val::Undef)?,
                        AluOp::Bany | AluOp::Ball if l == 0 => vs_reduce(&mut vs, m, w, a, s[0])?,
                        AluOp::Bany | AluOp::Ball => Val::Undef,
                        /* the 32-bit conversions and shifts of
                         * intbool_199_vsc and intbool_89_flatlast
                         * (docs/notes/mali-int-and-size.md, vs-int-convert):
                         * `F32_TO_S32.rtz`, `F32_TO_U32.rtz`, `S32_TO_F32`,
                         * `U32_TO_F32`, `FROUND.f32.rtz`, `LSHIFT_OR.i32`,
                         * `ARSHIFT_OR.i32`, `RSHIFT_OR.i32`, `ISUB.s32`;
                         * constants fold as NIR's (`fold_int`). The front end
                         * gives a conversion of a scalar no width (four
                         * lanes): the lanes past its source's stay undefined */
                        AluOp::Ftrunc
                        | AluOp::F2i
                        | AluOp::F2u
                        | AluOp::I2f
                        | AluOp::U2f
                        | AluOp::Ishl
                        | AluOp::Ishr
                        | AluOp::Ushr
                        | AluOp::Ineg
                        | AluOp::Iadd
                            if x(0) == Val::Undef =>
                        {
                            Val::Undef
                        }
                        AluOp::Ftrunc => vs.alu(Alu::Ftrunc, x(0), Val::Undef, Val::Undef)?,
                        AluOp::F2i
                        | AluOp::F2u
                        | AluOp::I2f
                        | AluOp::U2f
                        | AluOp::Ishl
                        | AluOp::Ishr
                        | AluOp::Ushr
                        | AluOp::Ineg
                        | AluOp::Iadd => match fold_int(a, x(0), x(1)) {
                            Some(v) => v,
                            None => {
                                let alu_of = |v: Val| match v {
                                    Val::Lane(n, 0) => match vs.nodes[n as usize] {
                                        NirNode { kind: Kind::Alu(op), src, width: 1, .. } => Some((op, src)),
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                let (p, q) = (x(0), x(1));
                                let (op, s0, s1) = match a {
                                    AluOp::F2i => (Alu::F2i, p, Val::Undef),
                                    AluOp::F2u => (Alu::F2u, p, Val::Undef),
                                    /* `i2f32(f2i32(a))` is `ftrunc(a)`
                                     * (nir_opt_algebraic.py:1250) */
                                    AluOp::I2f => match alu_of(p) {
                                        Some((Alu::F2i, [t, ..])) => (Alu::Ftrunc, t, Val::Undef),
                                        _ => (Alu::I2f, p, Val::Undef),
                                    },
                                    AluOp::U2f => (Alu::U2f, p, Val::Undef),
                                    AluOp::Ishl => (Alu::Ishl, p, q),
                                    AluOp::Ishr => (Alu::Ishr, p, q),
                                    AluOp::Ushr => (Alu::Ushr, p, q),
                                    AluOp::Ineg => (Alu::Ineg, p, Val::Undef),
                                    /* GLSL's `a - b` is `iadd(a, ineg(b))`,
                                     * which is `isub(a, b)`
                                     * (nir_opt_algebraic.py); the `ineg`
                                     * is left to `nir_opt_dce` */
                                    _ => match (alu_of(p), alu_of(q)) {
                                        (_, Some((Alu::Ineg, [b, ..]))) => (Alu::Isub, p, b),
                                        (Some((Alu::Ineg, [b, ..])), _) => (Alu::Isub, q, b),
                                        /* any other sum: `IADD.s32`, or
                                         * `IADD_IMM.i32` with a constant
                                         * (opt.rs; deqp_intval_opvs_add_int_highp
                                         * and _post_increment_int, plan #377
                                         * lower-intval-2) */
                                        _ => (Alu::Iadd, p, q),
                                    },
                                };
                                vs.alu(op, s0, s1, Val::Undef)?
                            }
                        },
                        /* one of constants folds as NIR's ([`fold_int`]):
                         * the `&&` of two compares of constants
                         * (deqp_constop_struct_param_out_vs's `s.red ==
                         * vec3(1.0, 0.0, 0.0) && s.blue == ..`) */
                        AluOp::Iand | AluOp::Ixor | AluOp::Inot | AluOp::Bnot
                            if matches!(fold_int(a, x(0), x(1)), Some(Val::Const(_)))
                                && matches!(x(0), Val::Const(_))
                                && (matches!(a, AluOp::Inot | AluOp::Bnot) || matches!(x(1), Val::Const(_))) =>
                        {
                            fold_int(a, x(0), x(1)).ok_or(MaliError::Defect(Defect::IrIndex))?
                        }
                        /* a lane past the operands' (a scalar's conversion
                         * is four lanes wide, as above) */
                        AluOp::Imul
                        | AluOp::Idiv
                        | AluOp::Iand
                        | AluOp::Ixor
                        | AluOp::Ilt
                        | AluOp::Ile
                        | AluOp::Ine
                        | AluOp::Ult
                        | AluOp::Ule
                        | AluOp::Imin
                        | AluOp::Imax
                            if x(0) == Val::Undef || x(1) == Val::Undef =>
                        {
                            Val::Undef
                        }
                        /* `/` of two run-time integers: the `nir_lower_idiv`
                         * sequence ([`vs_idiv`]) */
                        AluOp::Idiv => vs_idiv(&mut vs, x(0), x(1))?,
                        AluOp::B2i | AluOp::I2b | AluOp::Inot | AluOp::Bnot if x(0) == Val::Undef => Val::Undef,
                        /* the 32-bit integer operations of the dEQP
                         * operator suite's `*_vertex` cases, the compares
                         * and bool logic (plan #377 lower-intval-2,
                         * docs/notes/mali-deqp-intval.md section 8): an
                         * integer stays 32 bits in every precision
                         * (`caps->int16` is false), a boolean is a 0 / ~0
                         * mask of its compare's width. Constants fold as
                         * NIR's ([`fold_int`]). */
                        AluOp::Imul
                        | AluOp::Iand
                        | AluOp::Ixor
                        | AluOp::Ilt
                        | AluOp::Ile
                        | AluOp::Ine
                        | AluOp::Ult
                        | AluOp::Ule
                        | AluOp::Imin
                        | AluOp::Imax
                        | AluOp::B2i
                        | AluOp::I2b
                        | AluOp::Inot
                        | AluOp::Bnot => match fold_int(a, x(0), x(1)) {
                            Some(v) => v,
                            None => {
                                let (p, q) = (x(0), x(1));
                                match a {
                                    /* `IMUL.i32` (deqp_intval_opvs_mul_ivec3_mediump) */
                                    AluOp::Imul => vs.alu(Alu::Imul, p, q, Val::Undef)?,
                                    /* `LSHIFT_AND.i32`, `LSHIFT_XOR.i32` */
                                    AluOp::Iand => vs.alu(Alu::Iand, p, q, Val::Undef)?,
                                    AluOp::Ixor => vs.alu(Alu::Ixor, p, q, Val::Undef)?,
                                    /* `ICMP_OR.s32.lt.m1` (deqp_intval_opvs_less_ivec4);
                                     * GLSL's `<=` is `ige` with its sources
                                     * swapped (glsl_to_nir.cpp:2488) */
                                    AluOp::Ilt => vs.alu(Alu::Icmp(Cmpf::Lt), p, q, Val::Undef)?,
                                    AluOp::Ile => vs.alu(Alu::Icmp(Cmpf::Ge), q, p, Val::Undef)?,
                                    AluOp::Ine => vs.alu(Alu::Icmp(Cmpf::Ne), p, q, Val::Undef)?,
                                    AluOp::Ult => vs.alu(Alu::Ucmp(Cmpf::Lt), p, q, Val::Undef)?,
                                    AluOp::Ule => vs.alu(Alu::Ucmp(Cmpf::Ge), q, p, Val::Undef)?,
                                    /* `CSEL.s32.lt` / `.gt` (bifrost_compile.c:3061-3070) */
                                    AluOp::Imin => vs.alu(Alu::Imin, p, q, Val::Undef)?,
                                    AluOp::Imax => vs.alu(Alu::Imax, p, q, Val::Undef)?,
                                    /* `MUX.i32 0, 1, b` */
                                    AluOp::B2i => vs.alu(Alu::B2i, p, Val::Undef, Val::Undef)?,
                                    /* `i2b(a)` is `ine(a, 0)`: of a bool
                                     * uniform's word (0 / ~0 in the FAU,
                                     * `UniformBooleanTrue`) it is the word
                                     * itself (deqp_intval_uniapi_bool_vs's
                                     * `CSEL.u32.eq r0, u5.w1, 0x0, ..`); of
                                     * another integer the compare */
                                    AluOp::I2b if vs_is_uniform_word(&vs, p) => p,
                                    AluOp::I2b => vs.alu(Alu::Icmp(Cmpf::Ne), p, Val::Const(0), Val::Undef)?,
                                    /* `!` of a run-time boolean; the walk's
                                     * users fold it (`bcsel(inot(a), b, c)`
                                     * is `bcsel(a, c, b)` below) */
                                    _ => vs.alu(Alu::Inot, p, Val::Undef, Val::Undef)?,
                                }
                            }
                        },
                        /* `%` with a run-time divisor and the unsigned
                         * forms have no reference (ES 1.00 has neither) */
                        AluOp::Udiv | AluOp::Umod | AluOp::Umin | AluOp::Umax => return Err(MaliError::IntValue),
                        /* another operation of constant lanes: the folds
                         * `fold::fold_float` makes exactly (`sign`, an
                         * `exp2` of an integer, ...), which are the bits
                         * any libm gives; the others keep the refusal */
                        _ if (0..super::nir_fs::ir_arity(a)).all(|i| matches!(x(i), Val::Const(_))) => {
                            let c = |i: usize| match x(i) {
                                Val::Const(c) => c,
                                _ => 0,
                            };
                            let r = fold::fold_float(a, c(0), c(1), c(2)).ok_or(MaliError::Arithmetic)?;
                            vs.note_user(Prod::Folded);
                            Val::Const(r)
                        }
                        _ => return Err(MaliError::Arithmetic),
                    };
                }
                /* a dot product (and a reduction) is one value, whatever
                 * width the IR gives it */
                if matches!(a, AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4 | AluOp::Bany | AluOp::Ball) {
                    out = [out[0]; 4];
                }
            }
            IrOp::Tex {
                dim: dim @ (TexDim::D2 | TexDim::Cube),
                shadow: false,
                proj: false,
                ret: ScalarType::F32,
                sampler,
                coord,
                lod: TexLod::Lod(level),
                offset: None,
            } => {
                let cube = dim == TexDim::Cube;
                let (s, t) = if cube {
                    /* [`Nir::cube_desc`]. A constant lane of the direction
                     * stays an operand: the intrinsics fold no constant,
                     * and `va_lower_constants` makes it an immediate
                     * (case114_v.expect.json's `CUBEFACE1 r2, 0x3F800000,
                     * r1, r0`), every lane of a direction of constants too
                     * (vslookup_101_cubelod.expect.json's `CUBEFACE1 r0,
                     * 0x3F800000, 0x0, 0x0`) */
                    let (x, y, z) = (lane(w, coord, 0), lane(w, coord, 1), lane(w, coord, 2));
                    vs.cube_desc(x, y, z)?
                } else {
                    /* a constant lane: its FAU word moved to the staging
                     * register, whether the other lane is a constant too
                     * (unitype_100.expect.json's `textureLod(vtex,
                     * vec2(0.25), 0.0)`, `MOV.i32 r2, u6.w0` and `MOV.i32
                     * r3, u6.w0`) or an attribute's lane used in place
                     * (case149_vstexconst{,_s}.expect.json, `MOV.i32 r1,
                     * u5.w1` / `MOV.i32 r0, u5.w1`) */
                    (lane(w, coord, 0), lane(w, coord, 1))
                };
                let level = lane(w, level, 0);
                /* the texels' width (mali-vs-lookups.md §7): a look-up of
                 * a lowp or mediump sampler is fp16 in Mesa's NIR
                 * (lower_precision's builtin), and `pan_nir_fuse_io_cvt`
                 * makes it fp32 again only when every reader converts it
                 * up (`f2f32`). It stays fp16 when a mediump product or
                 * sum reads it, the reader of [`vs_alu16u`]
                 * (gm2_texvert_3's `texture2D(s, uv).xyz * 2.0 - 1.0`);
                 * after `nir_opt_cse` that holds for each copy of one
                 * look-up (gm2_texvert_4) */
                let medium = m.prec_of(ValueId(id.0)) == Prec::Medium;
                let half = medium && vs_tex_read16(m, id);
                /* `nir_scalar_is_zero`: the bits 0 */
                let (kind, src) = if level == Val::Const(F32_ZERO) {
                    (Kind::VsTex { explicit: false, cube, sampler, index: 0, medium }, [s, t, Val::Undef])
                } else {
                    let x = vs.sfixed_8_8(level)?;
                    (Kind::VsTex { explicit: true, cube, sampler, index: 0, medium }, [s, t, x])
                };
                /* a second look-up of one sampler at one coordinate and
                 * level is the first's (`nir_opt_cse`; gm2_texvert_4: one
                 * TEX_SINGLE, both varyings read it). One built fp32 that
                 * a later copy wants fp16 has no form here */
                let seen = vs.order[..vs.len]
                    .iter()
                    .copied()
                    .find(|&k| vs.nodes[k as usize].kind == kind && vs.nodes[k as usize].src == src);
                let n = match seen {
                    Some(k) if half && vs.nodes[k as usize].bits != 16 => return Err(MaliError::TexVertex),
                    Some(k) => k,
                    None => vs.push_sized(kind, src, if half { 16 } else { 32 })?,
                };
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            IrOp::Tex { .. } | IrOp::TexSize { .. } => return Err(MaliError::TexVertex),
            /* fragment only (the front end refuses it in a vertex shader) */
            IrOp::Discard => return Err(MaliError::DiscardForm),
        }
        let origin = |origins: &[[u16; 4]], s: ir::Src, i: usize| {
            origins.get(s.val.0 as usize).map_or(0, |o| o[s.swizzle[i] as usize & 3])
        };
        let mut made = [0u16; 4];
        match *op {
            IrOp::LoadLocal { local, index: None, .. } => {
                made = local_origins.get(local.0 as usize).copied().unwrap_or([0; 4]);
            }
            IrOp::StoreLocal { local, index: None, src, write_mask } => {
                if let Some(t) = local_origins.get_mut(local.0 as usize) {
                    for (i, t) in t.iter_mut().enumerate() {
                        if write_mask & (1 << i) != 0 {
                            *t = origin(origins, src, i);
                        }
                    }
                }
            }
            IrOp::StoreOutput { slot: IoSlot::Varying(loc), src } => {
                if let Some(t) = varying_origins.get_mut(loc as usize) {
                    for (i, t) in t.iter_mut().enumerate() {
                        *t = origin(origins, src, i);
                    }
                }
            }
            IrOp::Alu(a, s) => made = vec_origins(w, id.0, a, &s, width, &out, |s, i| origin(origins, s, i)),
            _ => {}
        }
        if let Some(o) = origins.get_mut(id.0 as usize) {
            *o = made;
        }
        if let Some(v) = w.vals.get_mut(id.0 as usize) {
            *v = out;
        }
    }
    /* a lane that is a component of a vector operation is as divergent as
     * the operation (`nir_vertex_divergence_analysis` marks a definition,
     * not its components): beside a lane that reads an attribute, a lane of
     * uniforms alone or a constant is interpolated like it, stored as it is
     * (deqp_varylink_va_multi3's `u_colorScale * color`, whose `z` is the
     * uniform: a `MOV` in the smooth `.v3`; deqp_varylink_draw_multi's
     * `.. * 0.5 + 0.5`, whose `w` is 1.0: stored in the smooth `.v4`).
     * `divergent`: those lanes; `apart`: every lane of a vector operation */
    let (mut divergent, mut apart) = (0u64, 0u64);
    for (l, tags) in varying_origins.iter().enumerate() {
        for (c, &t) in tags.iter().enumerate() {
            if t == 0 {
                continue;
            }
            let of = w.vals.get((t as usize - 1) / 4).copied().unwrap_or([Val::Undef; 4]);
            /* a vector operation of constants alone is a constant to the
             * link: Mesa's GLSL front end folds it before NIR
             * (`ir_constant_expression.cpp`; deqp_constprod_fma_matmul_const_vs's
             * `in0 * in1` of two `const mat2`, whose columns' lanes the
             * fragment shader takes as constants) */
            if of.iter().all(|v| matches!(v, Val::Const(_) | Val::Undef)) {
                continue;
            }
            apart |= 1 << (l * 4 + c);
            if of.iter().any(|v| matches!(*v, Val::Lane(n, _) if !vs.convergent(n))) {
                divergent |= 1 << (l * 4 + c);
            }
        }
    }
    /* a constant lane of a vector operation the link does not fold (it folds
     * an operation of constants alone, and the back end scalarises after the
     * link) is no constant to the link: the vertex shader stores it (`kept`),
     * flat with the operation's other lanes when no lane of it reads an
     * attribute (`kept_flat`; deqp_varylink_draw_single's `vec4(u * color,
     * 1.0) * 0.5 + 0.5`: four flat lanes, the 1.0 an `IADD_IMM.i32`). A
     * shader with a late fold ([`vs_fmul`]) keeps its own rule for these
     * lanes, in both of its walks ([`build`]: the constants the numbering
     * walk does not have are taken back, and a lane the attribute numbering
     * depends on is refused) */
    let mut kept_flat = kept_uniform;
    let late = vs.late_fold || numbering;
    for (l, lanes) in varyings.iter().enumerate().filter(|_| !late) {
        for (c, v) in lanes.iter().flatten().enumerate() {
            let bit = 1u64 << (l * 4 + c);
            if matches!(v, Val::Const(_)) && apart & bit != 0 && (phis | kept) & bit == 0 {
                kept |= bit;
                if divergent & bit == 0 {
                    kept_flat |= bit;
                }
            }
        }
    }
    /* an invariant output keeps its arithmetic exact in Mesa (no
     * contraction): refused, unless every invariant output is a copy (an
     * attribute, a uniform or a constant, through locals), which has no
     * arithmetic to keep exact. Measured: deqp_misc_invariant_position,
     * `invariant gl_Position; gl_Position = a_pos;`, whose position
     * program is the non-invariant one's, the viewport transform fused
     * (`FMA.f32 r0, r0^, u0.w0, u0.w1`). The test is the specification's,
     * wider than Mesa's ([`super::mesa_invariant`]): a shader under
     * `#pragma STDGL invariant(all)` with no declared output is refused
     * too (unless its outputs are copies), though Mesa compiles it as
     * without the pragma. Lowered: a shader whose arithmetic all goes to
     * outputs declared invariant (every other output a copy, no pragma)
     * is built exact, with no late fusion ([`fuse`]): an exact `fmul`
     * stays `FMA.f32 a, b, -0` and its `fadd` a separate `FADD.f32`, the
     * viewport transform still fused, as made
     * (deqp_misc2_invariant_posonly). Exactness is per value
     * ([`Nir::mark_exact`], before [`fuse`]): an invariant output beside a
     * computed non-invariant one keeps exact the terms they share
     * (`nir_opt_cse` carries `exact` into the one left,
     * deqp_misc2_invariant_cse), the rest of the other output fused. The
     * pragma's stays refused */
    let exact = m.outputs.iter().any(|o| o.is_invariant()) && !output_copies(m, true);
    let pragma = m.outputs.iter().any(|o| o.invariant == crate::shaderc::online::ir::Invariance::Pragma);
    if exact && pragma {
        return Err(MaliError::Invariant);
    }
    /* the locations the output lanes read, whichever the fragment shader
     * reads ([`build`]) */
    if numbering {
        for (l, lanes) in varyings.iter().enumerate() {
            for (c, &v) in lanes.iter().flatten().enumerate() {
                let read = vs.inputs_of(&[v])?;
                if let Some(x) = vs.lane_inputs.get_mut(4 * l + c) {
                    *x = read;
                }
            }
        }
    } else if vs.late_fold {
        let mut roots = [Val::Undef; 4 * MAX_VARYINGS + 5];
        let lanes = varyings.iter().flatten().flatten().chain(position.iter().flatten()).chain(point_size.iter());
        for (r, &v) in roots.iter_mut().zip(lanes) {
            *r = v;
        }
        vs.output_inputs = vs.inputs_of(&roots)?;
    }
    /* the constant lanes of the shader's own stores, which Mesa's link
     * propagates into the fragment shader before it moves code
     * (nir_opt_varyings.c:2880-2900, `propagate_uniform_expressions`: a
     * constant is a uniform expression): [`super::link::VaryingConsts`] */
    for o in m.outputs.iter().filter(|o| o.kind == IoKind::Varying) {
        /* a varying declared and never written: each lane an undef, which
         * the link takes into the fragment shader's loads and stores no
         * slot for ([`super::link::VaryingConsts::push_undef`]) */
        let Some(lanes) = varyings.get(o.location as usize).copied().flatten() else {
            for c in 0..o.components.min(4) {
                vs.consts.push_undef(o.location, c);
            }
            continue;
        };
        for (c, l) in lanes.iter().enumerate().take(o.components as usize) {
            let phi = phis & (1 << (o.location as usize * 4 + c)) != 0;
            match *l {
                /* a constant both sides of an `if` store is a phi when the
                 * link runs: stored flat ([`vs_join`]) */
                Val::Const(_) if phi => vs.consts.push_flat(o.location, c as u8),
                /* a constant lane of a mediump vector the link sees computed:
                 * stored by this stage, read interpolated (`kept`) */
                Val::Const(_) if kept_flat & (1 << (o.location as usize * 4 + c)) != 0 => {
                    vs.consts.push_flat(o.location, c as u8)
                }
                Val::Const(_) if kept & (1 << (o.location as usize * 4 + c)) != 0 => {}
                Val::Const(bits) => vs.consts.push(o.location, c as u8, bits),
                /* a lane of a vector operation that reads an attribute in
                 * another lane: interpolated (`divergent`) */
                Val::Lane(..) if divergent & (1 << (o.location as usize * 4 + c)) != 0 => {}
                /* a lane of uniforms alone, which the link promotes to
                 * flat ([`super::link::VaryingConsts::push_flat`]) */
                Val::Lane(n, _) if vs.convergent(n) => vs.consts.push_flat(o.location, c as u8),
                _ => {}
            }
        }
    }
    /* the interpolated lanes equal to an earlier one, which Mesa's link
     * merges (`deduplicate_outputs`): [`super::link::VaryingConsts::push_equal`] */
    let decls: [Option<&ir::IoDecl<'_>>; MAX_VARYINGS] =
        core::array::from_fn(|i| m.outputs.iter().filter(|o| o.kind == IoKind::Varying).nth(i));
    /* a lane's value: the vertex shader's outputs are temporaries stored
     * at its end (`nir_lower_io_passes`, nir_lower_io.c:1434-1453), so
     * deduplicate_outputs compares the values the lanes hold there,
     * wherever a vertex `if` wrote them (b15: equal on both sides; v01,
     * v15, v19: one lane before or after the `if`). A join of two values
     * (a select or a phi) Mesa merges too (b13 stores two lanes), but its
     * vertex words for the join are not this back end's (the third
     * review's b14): those lanes stay apart and the program is refused
     * (`VaryingDuplicate`). The interpolation qualifier is the fragment
     * shader's ([`super::link::FsConsts::class`]'s reader, nir_fs.rs
     * `layout`) */
    let lane_at = |o: &ir::IoDecl<'_>, c: usize| {
        let l = varyings.get(o.location as usize).copied().flatten()?;
        let phi = phis & (1 << (o.location as usize * 4 + c)) != 0;
        match l[c & 3] {
            Val::Lane(n, _) if phi && matches!(vs.nodes[n as usize].kind, Kind::Phi | Kind::Alu(Alu::Bcsel)) => None,
            Val::Lane(n, _) if vs.nodes[n as usize].kind == Kind::Phi => None,
            /* the key is where the link resolves the lane, not divergence
             * (`deduplicate_outputs`' `nir_scalar_resolved`): two lanes of
             * one uniform value merge (varymerge_equal's `vec4(u *
             * vec3(1.0), 1.0)`: one flat lane), two components of one vector
             * operation do not, equal or not (deqp_varylink_draw_single:
             * three lanes of `ffma(u, 0.5, 0.5)`, each stored) */
            Val::Lane(..) => Some((varying_origins[o.location as usize & (MAX_VARYINGS - 1)][c & 3], l[c & 3])),
            _ => None,
        }
    };
    let mut equal = [(0u8, 0u8, 0u8, 0u8); 4 * MAX_VARYINGS];
    let mut nequal = 0;
    for (i, o) in decls.iter().enumerate().filter_map(|(i, o)| o.map(|o| (i, o))) {
        for c in 0..(o.components as usize).min(4) {
            let Some(v) = lane_at(o, c) else { continue };
            let earlier = decls[..=i].iter().flatten().find_map(|e| {
                let last = if e.location == o.location { c } else { e.components as usize };
                (0..last.min(4)).find(|&d| lane_at(e, d) == Some(v)).map(|d| (e.location, d as u8))
            });
            if let (Some((l, d)), Some(x)) = (earlier, equal.get_mut(nequal)) {
                *x = (o.location, c as u8, l, d);
                nequal += 1;
            }
        }
    }
    for &(l, c, of_l, of_c) in &equal[..nequal] {
        vs.consts.push_equal(l, c, of_l, of_c);
    }
    /* the fragment shader's moved expressions, cloned at the end of the
     * shader's own code (nir_opt_varyings.c:4398-4401) */
    super::motion::apply(&mut vs, &mut varyings, moved)?;
    let p = position.ok_or(MaliError::NoPosition)?;
    if p.contains(&Val::Undef) {
        return Err(MaliError::PositionLane);
    }
    /* the varyings, laid out as Mesa's link does (nir_opt_varyings): it
     * propagates a constant lane into the fragment shader (sel_v_const: a
     * program's vertex shader is built for a fragment shader that took the
     * constant lanes, mod.rs `compile_fs_for_vs`, so it stores none of
     * them), promotes a lane that reads no attribute to flat (sel_v_uniform,
     * below) and merges two lanes of equal value (sel_v_duplicate, refused:
     * the layout would not be this stage's alone) */
    let decls = m.outputs.iter().filter(|o| o.kind == IoKind::Varying);
    vs.nvar = generic_layout(decls.clone().map(|o| (o.location, o.components, o.gl_type)), &mut vs.varyings)?;
    /* a shader alone that stores a constant lane is built for its own lanes
     * less its constant ones, the layout the link's fragment shader reads
     * once they are folded into it (codegen.rs `compile_vs_without_consts`,
     * which this makes the shader's own compile) */
    let mut own = [(0u8, 0u8); MAX_VARYINGS];
    let mut nown = 0;
    let konst = |o: &&ir::IoDecl<'_>, c: u8| match varyings.get(o.location as usize).copied().flatten() {
        /* never written: an undef the link takes (`push_undef`) */
        None => true,
        Some(l) => {
            matches!(l[c as usize & 3], Val::Const(_))
                && (phis | kept) & (1 << (o.location as usize * 4 + (c as usize & 3))) == 0
        }
    };
    /* a lane equal to an earlier one, which the link's fragment shader reads
     * from that one: a shader alone is built without it too (the three
     * equal colour lanes of gm2_arithconst_3's `diffuse * Albedo.rgb`) */
    let dup = |o: &&ir::IoDecl<'_>, c: u8| equal[..nequal].iter().any(|e| (e.0, e.1) == (o.location, c));
    let reads = match reads {
        None if decls.clone().any(|o| (0..o.components).any(|c| konst(&o, c) || dup(&o, c))) => {
            for o in decls {
                let mask = (0..o.components).filter(|&c| !konst(&o, c) && !dup(&o, c)).fold(0u8, |m, c| m | 1 << c);
                if mask == 0 {
                    continue;
                }
                *own.get_mut(nown).ok_or(MaliError::VaryingLocation)? = (o.location, mask);
                nown += 1;
            }
            vs.own_reads = Some((own, nown));
            Some(&own[..nown])
        }
        r => r,
    };
    compact_layout(&mut vs, reads);
    let mut seen = [(0u16, Val::Undef); 4 * MAX_VARYINGS];
    let mut nseen = 0;
    /* each stored lane's class in scalar slot order (`fs_assign_slot_groups`,
     * nir_opt_varyings.c:5228-5290): interpolated (`smooth`) or convergent
     * (`flat`: uniforms alone, a constant the link sees computed by a phi
     * or a vector operation of uniforms); [`fill`] packs them */
    let mut classes = [(0u8, 0u8, false); 4 * MAX_VARYINGS];
    let mut nclasses = 0;
    /* a convergent lane that is a constant (of a phi, or kept beside a
     * vector operation of uniforms): with an interpolated lane beside it,
     * Mesa's link may propagate the constant rather than fill a free word
     * (no reference draws the pair, mali_draw_oom's
     * `conditionals_attribute_select`), so the pair stays refused */
    let mut const_flat = false;
    for k in 0..vs.nvar {
        let v = vs.varyings[k];
        if vs.masks[k] != (1 << v.comps) - 1 {
            /* a part (or none) of it is stored */
            vs.note(Prod::VaryingCompacted);
        }
        let lanes = match varyings.get(v.location as usize).copied().flatten() {
            Some(l) => l,
            /* never written and so not stored ([`compact_layout`] over
             * the reads the undef lanes left out) */
            None if vs.masks[k] == 0 => continue,
            None => return Err(MaliError::VaryingForm),
        };
        let attr = |l: Val| match l {
            Val::Lane(id, c) => match vs.nodes[id as usize].kind {
                Kind::Input { loc, .. } => Some((id, loc, c)),
                _ => None,
            },
            _ => None,
        };
        let whole = attr(lanes[0]).is_some_and(|(id, loc, _)| {
            let declared = m.inputs.iter().find(|d| d.slot() == IoSlot::Attrib(loc)).map(|d| d.components);
            declared == Some(v.comps)
                && lanes[..v.comps as usize].iter().enumerate().all(|(c, l)| *l == Val::Lane(id, c as u8))
        });
        let stored = (0..v.comps as usize).filter(|&c| vs.masks[k] & (1 << c) != 0);
        for (c, l) in stored.map(|c| (c, lanes[c])) {
            let (mut flat, mut smooth) = (false, false);
            match l {
                Val::Undef => return Err(MaliError::VaryingForm),
                Val::Const(_) if phis & (1 << (v.location as usize * 4 + c)) != 0 => {
                    flat = true;
                    const_flat = true;
                }
                /* a constant lane of a vector operation of uniforms: stored
                 * flat beside them (`kept_flat`) */
                Val::Const(_) if kept_flat & (1 << (v.location as usize * 4 + c)) != 0 => {
                    const_flat = true;
                    vs.prods |= 1 << Prod::VaryingComputed as u32;
                    classes[nclasses] = (v.location, c as u8, true);
                    nclasses += 1;
                    continue;
                }
                /* a lane the back end's fold made a constant, which the
                 * link saw computed ([`build`]) and the fragment shader
                 * reads: stored as it is, each store its own constant
                 * (gm2_arithconst_4), equal or not to another; a constant
                 * lane of a mediump vector the same (`kept`) */
                Val::Const(_) if vs.late_fold || kept & (1 << (v.location as usize * 4 + c)) != 0 => {
                    vs.prods |= 1 << Prod::VaryingComputed as u32;
                    classes[nclasses] = (v.location, c as u8, false);
                    nclasses += 1;
                    continue;
                }
                Val::Const(_) => return Err(MaliError::VaryingNotFromAttribute),
                /* a convergent lane (`nir_vertex_divergence_analysis`: no
                 * attribute behind it) is promoted to flat, its store
                 * below: one behind a texture look-up (a `tex` is no
                 * uniform expression, nir_opt_varyings.c:2846-2878), a
                 * uniform array read at a run-time index (measured:
                 * uniarray_vs.expect.json keeps the `LD_PKA.i32` in the
                 * varying program), or uniforms alone (117, 181, 191, 199:
                 * the link neither propagates nor moves them, the
                 * fragment shader loads them flat, varyconst_*.expect.json) */
                Val::Lane(..) if divergent & (1 << (v.location as usize * 4 + c)) != 0 => smooth = true,
                Val::Lane(n, _) if vs.uniform_only(n) => flat = true,
                /* fp16 arithmetic of fp16 array elements read at a run-time
                 * index (`uniform_idx16`), widened at the store: convergent,
                 * flat beside the constant lanes kept with it
                 * (deqp_uniarray_dyn_vec2_vs's `vec4(res, 0.0, 1.0)`, the
                 * fragment shader's `.v4.src_flat32`) */
                Val::Lane(n, _) if widened_sum16(&vs, n) => flat = true,
                /* a look-up or a run-time indexed read with no attribute
                 * behind it: convergent too (the conversion below) */
                Val::Lane(n, _) if !vs.reads_attribute(n) && vs.convergent(n) => flat = true,
                /* no attribute behind it, and no reference of its form
                 * (a phi of uniforms) */
                Val::Lane(n, _) if !vs.reads_attribute(n) && !vs.reads_tex(n) && !vs.reads_indexed(n) => {
                    return Err(MaliError::VaryingNotFromAttribute)
                }
                Val::Lane(..) => smooth = true,
                Val::Pair(..) => return Err(MaliError::Defect(Defect::IrIndex)),
            }
            debug_assert!(flat != smooth);
            classes[nclasses] = (v.location, c as u8, flat);
            nclasses += 1;
            let key = (varying_origins[v.location as usize & (MAX_VARYINGS - 1)][c], l);
            if seen[..nseen].contains(&key) {
                return Err(MaliError::VaryingDuplicate);
            }
            seen[nseen] = key;
            nseen += 1;
            if !whole {
                let p = if attr(l).is_some() { Prod::VaryingSwizzle } else { Prod::VaryingComputed };
                vs.prods |= 1 << p as u32;
            }
        }
        if whole {
            vs.note(Prod::Varying);
        }
    }
    if const_flat && classes[..nclasses].iter().any(|l| !l.2) {
        return Err(MaliError::VaryingNotFromAttribute);
    }
    /* the convergent lanes that take the interpolated vec4's free words:
     * stored as they are, no conversion */
    let filled = fill(&classes[..nclasses])?;
    /* the viewport transform, before the position's store */
    vs.user = false;
    vs.exact_build = false;
    vs.user_end = vs.n;
    let scale = vs.load(Kind::Sysval(Sysval::ViewportScale))?;
    let offset = vs.load(Kind::Sysval(Sysval::ViewportOffset))?;
    vs.note(if matches!(p[3], Val::Const(_)) { Prod::ViewportConstW } else { Prod::ViewportW });
    /* a w that selects between constants none of which is below zero:
     * Mesa's range analysis takes a zero of either sign as zero, so `1/w`
     * is not negative and the clamp's `fmax` goes, and the w the
     * reciprocal reads is the +0.0 one (deqp_constop_nz_shared_vs's `w`
     * of `a_position * -0.0` or 1.0: `CSEL ... 0x3F800000, 0x0`, then
     * `FRCP`, `FMIN` alone) */
    let nonneg = |v: Val| matches!(v, Val::Const(k) if k == F32_NEG_ZERO || (k >> 31 == 0 && plain(k)));
    let pz = |v: Val| if v == Val::Const(F32_NEG_ZERO) { Val::Const(F32_ZERO) } else { v };
    /* a w of ±0.0: `frcp` is ±inf, which the clamp takes to ±2^15 only
     * when the whole position is a constant vector (the first
     * nir_opt_constant_folding folds the chain:
     * deqp_cfold2_zero_operand_rcp_inf). With a lane not constant
     * (`vec4(p, 0.0, 0.0)`) the w reaches `frcp` as a constant only after
     * copy propagation, and nir_opt_algebraic drops the clamp first: Mesa
     * stores w = +inf (pan_shc), not lowered. The constant vector is the
     * IR's: one store of a constant, not lanes the walk folded */
    let mut stores = m.instrs.iter().filter_map(|op| match op {
        IrOp::StoreOutput { slot: IoSlot::Position, src } => Some(src.val),
        _ => None,
    });
    let const_position = match (stores.next(), stores.next()) {
        (Some(v), None) => matches!(m.def(v), Some(IrOp::ConstF32(_))),
        _ => false,
    };
    if is_zero(p[3]) && !const_position {
        return Err(MaliError::ConstantFold);
    }
    let mut w = p[3];
    let mut clamp_low = true;
    if let Some((Alu::Bcsel, [c, a, b])) = vs_alu_of(&vs, w) {
        if nonneg(a) && nonneg(b) {
            if a == Val::Const(F32_NEG_ZERO) || b == Val::Const(F32_NEG_ZERO) {
                w = vs.alu(Alu::Bcsel, c, pz(a), pz(b))?;
            }
            clamp_low = false;
        }
    }
    let r = vs.alu(Alu::Frcp, w, Val::Undef, Val::Undef)?;
    let r = if clamp_low { vs.alu(Alu::Fmax, r, Val::Const(W_RECIP_MIN), Val::Undef)? } else { r };
    let wr = vs.alu(Alu::Fmin, r, Val::Const(W_RECIP_MAX), Val::Undef)?;
    let mut ndc = [Val::Undef; 3];
    for (c, d) in ndc.iter_mut().enumerate() {
        *d = vs.alu(Alu::Fmul, p[c], wr, Val::Undef)?;
    }
    let mut s = [Val::Undef; 3];
    for (c, d) in s.iter_mut().enumerate() {
        *d = vs.alu(Alu::Fmul, ndc[c], Val::Lane(scale, c as u8), Val::Undef)?;
    }
    let mut screen = [Val::Undef; 4];
    for c in 0..3 {
        screen[c] = vs.alu(Alu::Fadd, s[c], Val::Lane(offset, c as u8), Val::Undef)?;
    }
    screen[3] = wr;
    /* nir_lower_point_size (bifrost_nir.c: a minimum of 1.0) on the fp16
     * point size (pan_nir_collect_varyings: PSIZ is fp16), after the
     * viewport transform: the outputs are stored in slot order */
    let psiz = match point_size {
        /* the front end writes gl_PointSize as one float */
        Some(Val::Undef) => return Err(MaliError::Defect(Defect::IrIndex)),
        Some(x) => {
            let h = vs.alu(Alu::F2f16, x, Val::Undef, Val::Undef)?;
            Some(vs.alu_sized(Alu::Fmax, 16, h, Val::Const(F16_ONE), Val::Undef)?)
        }
        None => None,
    };
    /* pan_nir_lower_vs_outputs: the stores, at the end, the varyings in
     * the layout's order (measured: sel_v_color_uv_written_reversed) */
    let lea = vs.push(Kind::Lea { out: 0 }, [Val::Undef; 3])?;
    for (c, v) in screen.iter().enumerate() {
        store(&mut vs, 0, lea, c as u8, false, *v)?;
    }
    if let Some(x) = psiz {
        let lea = vs.push(Kind::Lea { out: 2 }, [Val::Undef; 3])?;
        store(&mut vs, 2, lea, 0, false, x)?;
    }
    if vs.masks[..vs.nvar].iter().any(|&m| m != 0) {
        /* a convergent varying promoted to flat (nir_opt_varyings.c:
         * 4964-5002, the compaction's `FS_VEC4_TYPE_FLAT` path): since
         * interpolation would have turned an infinity into a NaN, the
         * producer stores `ffma(x, 0.0, x)`, exact (`build_convert_inf_to_
         * nan`, :1111-1117), before the store of each lane, when the
         * fragment shader's use of the input preserves NaNs
         * (`uses_preserve_nans`, :1094-1108: a store, an `if`, a
         * NaN-preserving operation). The pair's fragment shader is not
         * known here; a colour stored or an `if` tested is the shape
         * measured (tools/mali/testdata/cube_vs), so the conversion is
         * made for every convergent lane. Exact: no algebraic rule folds
         * the `ffma` (the node is pushed, not built by [`Nir::alu`]). */
        /* one conversion of a value, whichever lanes store it
         * (`nir_opt_cse`; deqp_varylink_draw_single: one `FMA.f32 r0, r0^,
         * 0x0, r0^`, stored three times) */
        let mut converted = [(Val::Undef, Val::Undef); 4 * MAX_VARYINGS];
        let mut nconverted = 0;
        for k in 0..vs.nvar {
            let v = vs.varyings[k];
            let Some(lanes) = varyings.get_mut(v.location as usize).and_then(|l| l.as_mut()) else { continue };
            for (c, l) in lanes.iter_mut().enumerate().take(v.comps as usize) {
                let Val::Lane(n, _) = *l else { continue };
                if vs.masks[k] & (1 << c) == 0 || vs.reads_attribute(n) {
                    continue;
                }
                /* interpolated with its vector's other lanes (`divergent`),
                 * or a convergent lane in an interpolated vec4 ([`fill`]) */
                if (divergent | filled) & (1 << (v.location as usize * 4 + c)) != 0 {
                    continue;
                }
                if let Some(&(_, y)) = converted[..nconverted].iter().find(|x| x.0 == *l) {
                    *l = y;
                    continue;
                }
                /* a select of two constants that is an `if`'s phi
                 * ([`vs_join`]; `phis`): nir_opt_if's `opt_split_alu_of_phi`
                 * moves the `ffma` into the `if`'s sides, where it folds on
                 * the constants: a new select of the same constants in the
                 * `ffma`'s place (vsarith_188_predif: the 0.25 / 0.75 lane's
                 * `CSEL` after the other lanes' `FMA`s), the first one left
                 * to [`dce`]. GLSL's own `c ? 1.0 : 0.0` is a `bcsel` from
                 * the start and keeps its exact `ffma`
                 * (deqp_intval_uniapi_bool_initial_vs: `CSEL.u32.eq` then
                 * `FMA.f32 r0, r0^, 0x0, r0^`). */
                let phi = phis & (1 << (v.location as usize * 4 + c)) != 0;
                if let (true, Some((Alu::Bcsel, src @ [_, Val::Const(_), Val::Const(_)]))) = (phi, vs_alu_of(&vs, *l)) {
                    let f = NirNode { kind: Kind::Alu(Alu::Bcsel), src, bits: 32, width: 1 };
                    *l = Val::Lane(vs.push_node(f)?, 0);
                    continue;
                }
                vs.note(Prod::VaryingConvergent);
                let x = *l;
                let f = NirNode { kind: Kind::Alu(Alu::Ffma), src: [x, Val::Const(F32_ZERO), x], bits: 32, width: 1 };
                *l = Val::Lane(vs.push_node(f)?, 0);
                if let Some(e) = converted.get_mut(nconverted) {
                    *e = (x, *l);
                    nconverted += 1;
                }
            }
        }
        let lea = vs.push(Kind::Lea { out: 1 }, [Val::Undef; 3])?;
        for k in 0..vs.nvar {
            let v = vs.varyings[k];
            let lanes = varyings[v.location as usize].ok_or(MaliError::Defect(Defect::IrIndex))?;
            let mask = vs.masks[k];
            let stored = (0..v.comps as usize).filter(|&c| mask & (1 << c) != 0);
            for (j, c) in stored.enumerate() {
                /* the output's store is per 16-byte slot: a slot's first
                 * word past offset 0 is its `iadd` (constant second) */
                let word = (v.offset / 4) as u8 + j as u8;
                store(&mut vs, 1, lea, word, word.is_multiple_of(4), lanes[c])?;
            }
        }
    }
    /* the values of the invariant outputs, exact ([`Nir::mark_exact`]) */
    if exact {
        let mut roots = [Val::Undef; 4 * MAX_VARYINGS + 5];
        let mut nroots = 0;
        let mut root = |v: Val| {
            if let Some(r) = roots.get_mut(nroots) {
                *r = v;
                nroots += 1;
            }
        };
        for o in m.outputs.iter().filter(|o| o.is_invariant()) {
            match o.kind {
                IoKind::Position => p.iter().for_each(|&v| root(v)),
                IoKind::PointSize => point_size.iter().for_each(|&v| root(v)),
                IoKind::Varying => {
                    varyings.get(o.location as usize).copied().flatten().iter().flatten().for_each(|&v| root(v))
                }
                _ => {}
            }
        }
        vs.mark_exact(&roots[..nroots]);
    }
    fuse(&mut vs);
    late_rules(&mut vs);
    dce(&mut vs, None);
    /* each live look-up's descriptor index: its sampler's rank by binding
     * among the samplers the live look-ups read (`textures_used` after
     * DCE; a look-up no output reads binds nothing), the fragment's rule
     * (nir_fs.rs `layout`; vslookup_149_rank.expect.json: `u_s2` then
     * `u_s0` read are handles 0x20011801 and 0x20001800) */
    let mut read = 0u64;
    for k in 0..vs.len {
        if let Kind::VsTex { sampler, .. } = vs.nodes[vs.order[k] as usize].kind {
            read |= 1u64.checked_shl(u32::from(sampler)).ok_or(MaliError::Defect(Defect::IrIndex))?;
        }
    }
    for k in 0..vs.len {
        if let Kind::VsTex { sampler, ref mut index, .. } = vs.nodes[vs.order[k] as usize].kind {
            *index = (read & ((1u64 << sampler) - 1)).count_ones() as u8;
        }
    }
    shrink(&mut vs, m)?;
    opt_move(&mut vs, scratch)?;
    /* each compare stands before its first user (`nir_move_comparisons`,
     * bifrost_nir.c:375-381, run on every stage): a flattened `if`'s
     * before its first select (vsarith_188_predif's `FCMP_OR.f32.lt.m1`
     * after both sides' arithmetic), `vec4(lessThan(a, b))`'s each before
     * its lane's `CSEL.u32.eq` (deqp_intval_opvs_less_ivec4) */
    let _ = joined;
    /* `move_all` has `nir_move_copies` too: a `vec2` of fp16 halves right
     * before the operation that reads it (deqp_intval_opvs_mul_ivec3_mediump:
     * each `MKVEC.v2i16` before its `FMA.v2f16`) */
    move_compares(&mut vs, scratch, true)?;
    Ok(vs)
}

/// `fs_assign_slot_groups` (nir_opt_varyings.c:5228-5290, documented at
/// :419-425) over the stored lanes in scalar slot order, `(location, lane,
/// convergent)`: the interpolated lanes take the first scalar slots, then the
/// convergent ones fill the words left free in the last interpolated vec4
/// (interpolated, stored as they are: deqp_posvary_tex_uniform_w's uniform
/// alpha), and those left over are flat in vec4s of their own after them
/// (`FMA x, 0, x`, `.src_flat32`; deqp_posvary_flat_sep_kau_bs26's `e` at
/// word 4 after a full vec4). With no interpolated lane all are flat
/// (varyconst_*). The lanes in another order (a convergent lane before an
/// interpolated one, deqp_posvary_flat_sep_vec4_bs51) would move, which the
/// per-varying layout cannot hold: refused. The
/// convergent lanes left interpolated, as `location * 4 + lane` bits;
/// [`super::nir_fs`]'s `layout` applies the same rule to the same lanes.
pub(super) fn fill(lanes: &[(u8, u8, bool)]) -> Result<u64, MaliError> {
    let interp = lanes.iter().filter(|l| !l.2).count();
    if interp == 0 {
        return Ok(0);
    }
    if lanes[..interp].iter().any(|l| l.2) {
        return Err(MaliError::VaryingNotFromAttribute);
    }
    let free = (4 - interp % 4) % 4;
    Ok(lanes[interp..].iter().take(free).fold(0u64, |m, l| m | 1 << (l.0 as u32 * 4 + l.1 as u32)))
}

/// Mesa's link keeps only the components the fragment shader reads
/// (`compact_varyings`, the fragment side's nir_fs.rs `layout`): each
/// component read, in layout order, takes the next scalar slot. Without
/// `reads` every component is stored where the layout puts it.
fn compact_layout(vs: &mut Nir<'_>, reads: Option<&[(u8, u8)]>) {
    let Some(reads) = reads else {
        for k in 0..vs.nvar {
            vs.masks[k] = (1 << vs.varyings[k].comps) - 1;
        }
        return;
    };
    let mut s = 0u16;
    for k in 0..vs.nvar {
        let v = &mut vs.varyings[k];
        let full = (1u8 << v.comps) - 1;
        let mask = reads.iter().find(|r| r.0 == v.location).map_or(0, |r| r.1 & full);
        vs.masks[k] = mask;
        v.offset = 4 * s;
        s += mask.count_ones() as u16;
    }
}

/// One word's store, after its address: `first` says the word is its
/// output's first (whose `iadd` keeps the constant second).
fn store(vs: &mut Nir<'_>, out: u8, lea: u16, word: u8, first: bool, data: Val) -> Result<(), MaliError> {
    let addr = if word == 0 {
        Val::Undef
    } else {
        Val::Lane(vs.push(Kind::Addr { out, word, swapped: first }, [Val::Lane(lea, 0), Val::Undef, Val::Undef])?, 0)
    };
    vs.push(Kind::Store { out, word }, [data, addr, Val::Undef])?;
    Ok(())
}

/// `bifrost_nir_lower_algebraic_late` (bifrost_nir_algebraic.py:31), after
/// the fusion (nir_opt_algebraic_late, bifrost_nir.c:338-358): a product
/// with 2.0 left is `fadd(a, a)`.
pub(super) fn late_rules(vs: &mut Nir<'_>) {
    for k in 0..vs.len {
        let x = vs.order[k] as usize;
        let nd = vs.nodes[x];
        let two = nd.src[..2].iter().position(|v| *v == Val::Const(F32_TWO));
        if let (Kind::Alu(Alu::Fmul), Some(k)) = (nd.kind, two) {
            let a = nd.src[1 - k];
            vs.nodes[x] = NirNode { kind: Kind::Alu(Alu::Fadd), src: [a, a, Val::Undef], ..nd };
            if x < vs.user_end {
                vs.note(Prod::FmulTwo);
            }
        }
    }
}

/// `bcsel(a, fadd(b, c), fadd(b, d))`, one of the sums used once, is
/// `fadd(b, bcsel(a, c, d))` (nir_opt_algebraic.py:4139-4143, a late rule:
/// "These kinds of sequences can occur after nir_opt_peephole_select").
/// Each `fadd` is matched both ways round, the first sum's order before
/// the second's (nir_search's commutative combinations: the second
/// expression's swap is the higher bit). The new select stands before the
/// `fadd`, which takes the old select's place. Measured:
/// fbload_predlive_fs_coord.stderr's `s = b - a` / `s = a + b`, `%18 =
/// bcsel_pan %17, %13 (fneg a), %3 (a)` then `%19 = fadd %5 (b), %18`.
pub(super) fn bcsel_fadd(vs: &mut Nir<'_>) -> Result<(), MaliError> {
    let mut k = 0;
    while k < vs.len {
        let x = vs.order[k] as usize;
        let nd = vs.nodes[x];
        k += 1;
        let (Kind::Alu(Alu::Bcsel), 32, 1) = (nd.kind, nd.bits, nd.width) else { continue };
        let sum = |vs: &Nir<'_>, v: Val| match v {
            Val::Lane(n, 0) => match vs.nodes[n as usize] {
                NirNode { kind: Kind::Alu(Alu::Fadd), src, bits: 32, width: 1 } => Some((n, [src[0], src[1]])),
                _ => None,
            },
            _ => None,
        };
        let (Some((t, ts)), Some((e, es))) = (sum(vs, nd.src[1]), sum(vs, nd.src[2])) else { continue };
        if vs.uses(t).count() != 1 && vs.uses(e).count() != 1 {
            continue;
        }
        let found = (0..4u8).find_map(|comb| {
            let (tb, tc) = if comb & 1 == 0 { (ts[0], ts[1]) } else { (ts[1], ts[0]) };
            let (eb, ed) = if comb & 2 == 0 { (es[0], es[1]) } else { (es[1], es[0]) };
            (tb == eb).then_some((tb, tc, ed))
        });
        let Some((b, c, d)) = found else { continue };
        let sel = NirNode { kind: Kind::Alu(Alu::Bcsel), src: [nd.src[0], c, d], ..nd };
        let n = vs.insert_before(k - 1, sel)?;
        vs.nodes[x] = NirNode { kind: Kind::Alu(Alu::Fadd), src: [b, Val::Lane(n, 0), Val::Undef], ..nd };
        k += 1;
    }
    Ok(())
}

/// [`bcsel_fadd`] on the one-lane 16-bit nodes of a mediump shader's
/// flattened ladder (nir_fs.rs `guarded16`), as bifrost's late loop runs it
/// (bifrost_nir.c:335-344): each `nir_opt_algebraic_late` call walks the
/// instructions last first (nir_search.c:879-906) and tries a fired rule's
/// users again after the walk (`nir_algebraic_update_automaton`, :617-638,
/// the new instructions themselves not); a fired rule's new select stands
/// before the `fadd` that takes the old select's place (:687-714); the
/// first rule (the then side's sum used once) before the second, each
/// trying the commutative combinations in order. `is_used_once` counts
/// every use still in the program, a dead one's too, until the `nir_opt_dce`
/// after the call; the calls repeat while one makes progress. So a ladder
/// of `acc += u` sums peels one `fadd` a call down to the first that is
/// no sum (cflow_71_deadloop: `%10 = bcsel_pan %9, %3.y, %7` of `2 * u_k.y`
/// and `3 * u_k.y`, then `%11 = fadd %1.y, %10`, docs/notes/mali-control-flow.md).
pub(super) fn bcsel_fadd16(vs: &mut Nir<'_>, scratch: &Arena<'_>) -> Result<(), MaliError> {
    let sum = |vs: &Nir<'_>, v: Val| match v {
        Val::Lane(n, 0) => match vs.nodes[n as usize] {
            NirNode { kind: Kind::Alu(Alu::Fadd), src, bits: 16, width: 1 } => Some((n, [src[0], src[1]])),
            _ => None,
        },
        _ => None,
    };
    let work = scratch.alloc_filled(2 * MAX_NODES, 0u16).map_err(|_| MaliError::OutOfScratch)?;
    loop {
        let mut progress = false;
        let mut nw = 0;
        for k in (0..vs.len).rev() {
            work[nw] = vs.order[k];
            nw += 1;
        }
        let mut next = 0;
        while next < nw {
            let x = work[next] as usize;
            next += 1;
            let nd = vs.nodes[x];
            let (Kind::Alu(Alu::Bcsel), 16, 1) = (nd.kind, nd.bits, nd.width) else { continue };
            let Some(k) = vs.order[..vs.len].iter().position(|&id| id as usize == x) else { continue };
            let (Some((t, ts)), Some((e, es))) = (sum(vs, nd.src[1]), sum(vs, nd.src[2])) else { continue };
            let once = [vs.uses(t).count() == 1, vs.uses(e).count() == 1];
            let found = (0..2).filter(|&r| once[r]).find_map(|_| {
                (0..4u8).find_map(|comb| {
                    let (tb, tc) = if comb & 1 == 0 { (ts[0], ts[1]) } else { (ts[1], ts[0]) };
                    let (eb, ed) = if comb & 2 == 0 { (es[0], es[1]) } else { (es[1], es[0]) };
                    (tb == eb).then_some((tb, tc, ed))
                })
            });
            let Some((b, c, d)) = found else { continue };
            let sel = NirNode { kind: Kind::Alu(Alu::Bcsel), src: [nd.src[0], c, d], ..nd };
            let n = vs.insert_before(k, sel)?;
            vs.nodes[x] = NirNode { kind: Kind::Alu(Alu::Fadd), src: [b, Val::Lane(n, 0), Val::Undef], ..nd };
            progress = true;
            for (id, _) in vs.uses(x as u16) {
                if nw == work.len() {
                    return Err(MaliError::TooManyValues);
                }
                work[nw] = id;
                nw += 1;
            }
        }
        if !progress {
            return Ok(());
        }
        dce(vs, None);
    }
}

/// The late fusion, last instruction first (nir_search.c's worklist pops
/// the last instruction first).
pub(super) fn fuse(vs: &mut Nir<'_>) {
    for k in (0..vs.len).rev() {
        let x = vs.order[k] as usize;
        /* an exact `fadd` is not fused; the viewport transform, made after
         * the shader's own nodes, is not exact ([`Nir::exact`]) */
        if vs.nodes[x].kind != Kind::Alu(Alu::Fadd) || (vs.exact[x] && x < vs.user_end) {
            continue;
        }
        /* the rules in their list order (nir_opt_algebraic.py:3816-3820),
         * each trying the fadd's sources in order: `fadd(fmul(a, b), c)`,
         * then `fadd(fneg(fmul(a, b)), c)` -> `ffma(fneg(a), b, c)`. A
         * 16-bit vector of two lanes reads its product whole
         * ([`Val::Pair`] of both halves, nir_fs.rs `revector_coords`) */
        let whole = |v: Val| match v {
            Val::Lane(f, 0) | Val::Pair(f, Swz::H01) => Some(f),
            _ => None,
        };
        let product = |vs: &Nir<'_>, v: Val| match whole(v) {
            Some(f) if vs.nodes[f as usize].kind == Kind::Alu(Alu::Fmul) && vs.live_in_order(f) => {
                vs.only_used_by_fadd(f).then_some(f)
            }
            _ => None,
        };
        let mut fused = false;
        for s in 0..2 {
            let Some(f) = product(vs, vs.nodes[x].src[s]) else { continue };
            let other = vs.nodes[x].src[1 - s];
            let fm = vs.nodes[f as usize];
            vs.nodes[x] = NirNode { kind: Kind::Alu(Alu::Ffma), src: [fm.src[0], fm.src[1], other], ..fm };
            if x < vs.user_end {
                vs.note(if matches!(other, Val::Const(_)) { Prod::FusedConstAddend } else { Prod::Fused });
            }
            fused = true;
            break;
        }
        if fused {
            continue;
        }
        for s in 0..2 {
            let Some(g) = whole(vs.nodes[x].src[s]) else { continue };
            let gn = vs.nodes[g as usize];
            if gn.kind != Kind::Alu(Alu::Fneg) || !vs.live_in_order(g) || !vs.only_used_by_fadd(g) {
                continue;
            }
            let Some(f) = product(vs, gn.src[0]) else { continue };
            let other = vs.nodes[x].src[1 - s];
            let fm = vs.nodes[f as usize];
            /* `fneg` of a constant folds (`nir_opt_constant_folding` in the
             * late loop): femtovg filter_image's `fpos - 3.0 * dir`, the
             * constant -3.0 an `IADD_IMM #0xC0400000` beside a uniform; a
             * 16-bit node's constant is its fp16 halves by now (nir_fs.rs
             * `vectorize16`): the sign of each (cflow_148_texbranch's `0.5 -
             * 2.0 * u.w`, `FMA.v2f16 .., 0x40000000.neg.h11, ..` once the
             * constant lowering finds -2.0 as the table's 2.0 negated) */
            if let Val::Const(c) = fm.src[0] {
                let sign = match (fm.bits, fm.width) {
                    (32, _) => 1 << 31,
                    (_, 2) => 0x8000_8000,
                    _ => 0x8000,
                };
                vs.nodes[x] =
                    NirNode { kind: Kind::Alu(Alu::Ffma), src: [Val::Const(c ^ sign), fm.src[1], other], ..fm };
                if x < vs.user_end {
                    vs.note(Prod::FusedNegated);
                }
                break;
            }
            /* the new fneg stands where the fadd does */
            let Ok(n) = vs.insert_before(
                k,
                NirNode { kind: Kind::Alu(Alu::Fneg), src: [fm.src[0], Val::Undef, Val::Undef], ..fm },
            ) else {
                break;
            };
            vs.nodes[x] = NirNode { kind: Kind::Alu(Alu::Ffma), src: [Val::Lane(n, 0), fm.src[1], other], ..fm };
            if x < vs.user_end {
                vs.note(Prod::FusedNegated);
            }
            break;
        }
    }
}

/// The late reassociation of a sum into a negated fused dot product
/// (nir_opt_algebraic.py:4004-4005, "Things that look like DPH"; the
/// fragment stage only: Mesa's condition leaves out the vertex stage), after
/// [`fuse`], on fp32 lanes: `fadd(fneg(ffma(a, b, ffma(c, d, fmul(e, f)))),
/// g)` is `ffma(fneg(a), b, ffma(fneg(c), d, ffma(fneg(e), f, g)))`, a
/// commutative match on the fadd; the outer `ffma` and the `fmul` used once,
/// `g` not a constant, `e` and `f` neither a constant nor an `fsign`. The
/// terrain noise's `m - vec4(dot(x0, x0), ..)` (gm2_locals_noise,
/// docs/notes/mali-local-arrays.md section 7). The three rules before it in
/// Mesa's list (the sums without the fneg, :3998-4003) match no reference,
/// glmark2 or corpus shader and are not taken; none of them matches an fneg,
/// so this one's matches are Mesa's whatever the order.
pub(super) fn dot_addend(vs: &mut Nir<'_>) -> Result<(), MaliError> {
    let mut k = 0;
    while k < vs.len {
        let x = vs.order[k] as usize;
        let nd = vs.nodes[x];
        let matched = match nd {
            NirNode { kind: Kind::Alu(Alu::Fadd), bits: 32, width: 1, src } => (0..2).find_map(|s| {
                let g = src[1 - s];
                let chain = vs.negated_dot(src[s])?;
                (!matches!(g, Val::Const(_))).then_some((chain, g))
            }),
            _ => None,
        };
        let Some((chain, g)) = matched else {
            k += 1;
            continue;
        };
        /* the new chain, innermost first, before the fadd; each first factor
         * negated (a constant's sign flipped, as [`fuse`] does) */
        let mut acc = g;
        let mut at = k;
        for (j, &f) in chain.iter().rev().enumerate() {
            let fd = vs.nodes[f as usize];
            let a = match fd.src[0] {
                Val::Const(c) => Val::Const(c ^ F32_NEG_ZERO),
                a => {
                    let neg =
                        NirNode { kind: Kind::Alu(Alu::Fneg), src: [a, Val::Undef, Val::Undef], bits: 32, width: 1 };
                    let n = vs.insert_before(at, neg)?;
                    at += 1;
                    Val::Lane(n, 0)
                }
            };
            let node = NirNode { kind: Kind::Alu(Alu::Ffma), src: [a, fd.src[1], acc], bits: 32, width: 1 };
            if j + 1 == chain.len() {
                vs.nodes[x] = node;
            } else {
                acc = Val::Lane(vs.insert_before(at, node)?, 0);
                at += 1;
            }
        }
        if x < vs.user_end {
            vs.note(Prod::FusedNegated);
        }
        k = at + 1;
    }
    Ok(())
}

impl Nir<'_> {
    /// The `ffma`, `ffma` and `fmul` of [`dot_addend`]'s `fneg(ffma(a, b,
    /// ffma(c, d, fmul(e, f))))`, outermost first, when `p` is one.
    fn negated_dot(&self, p: Val) -> Option<[u16; 3]> {
        let fp32 = |v: Val, op: Alu| match v {
            Val::Lane(n, 0) => match self.nodes[n as usize] {
                NirNode { kind, bits: 32, width: 1, .. } if kind == Kind::Alu(op) && self.live_in_order(n) => Some(n),
                _ => None,
            },
            _ => None,
        };
        let once = |n: u16| self.uses(n).count() == 1;
        let neg = fp32(p, Alu::Fneg)?;
        let outer = fp32(self.nodes[neg as usize].src[0], Alu::Ffma).filter(|&n| once(n))?;
        let inner = fp32(self.nodes[outer as usize].src[2], Alu::Ffma)?;
        let mul = fp32(self.nodes[inner as usize].src[2], Alu::Fmul).filter(|&n| once(n))?;
        /* `is_not_const_and_not_fsign`: no `fsign` reaches this graph (the
         * GLSL front end's `sign` is a select) */
        if self.nodes[mul as usize].src[..2].iter().any(|s| matches!(s, Val::Const(_))) {
            return None;
        }
        Some([outer, inner, mul])
    }

    /// `is_only_used_by_fadd` (nir_search_helpers.h:594-613): every use an
    /// fadd, or an fneg whose own uses are.
    pub(super) fn only_used_by_fadd(&self, id: u16) -> bool {
        self.uses(id).all(|(u, _)| match self.nodes[u as usize].kind {
            Kind::Alu(Alu::Fadd) => true,
            Kind::Alu(Alu::Fneg) => self.only_used_by_fadd(u),
            _ => false,
        })
    }

    /// A node inserted into the order before position `k`; its id.
    pub(super) fn insert_before(&mut self, k: usize, nd: NirNode) -> Result<u16, MaliError> {
        if self.n >= MAX_NODES {
            return Err(MaliError::TooManyValues);
        }
        let id = self.n as u16;
        self.nodes[self.n] = nd;
        self.block[self.n] = self.block[self.order[k] as usize];
        self.n += 1;
        self.order.copy_within(k..self.len, k + 1);
        self.order[k] = id;
        self.len += 1;
        Ok(id)
    }

    fn live_in_order(&self, id: u16) -> bool {
        self.order[..self.len].contains(&id)
    }
}

/// `nir_opt_dce`: what no store needs leaves the order. With `only` the
/// stores of one IDVS variant are the roots (bifrost_compile.c:4374-4399):
/// 0 the position's and the point size's, 1 the varyings'; `live` says
/// what stays.
pub fn dce(vs: &mut Nir<'_>, only: Option<u8>) {
    for l in vs.live.iter_mut() {
        *l = false;
    }
    /* an `if`'s condition is read by the `if` (`nir_if::condition` is a
     * use), so it lives as long as the region does */
    for r in vs.ifs[..vs.nifs].iter().filter(|r| r.live) {
        if let Val::Lane(c, _) = r.cond {
            vs.live[c as usize] = true;
        }
    }
    /* in reverse order, again while a loop header's phi made a node of the
     * body (later in the order) live */
    let mut progress = true;
    while progress {
        progress = false;
        for k in (0..vs.len).rev() {
            let id = vs.order[k] as usize;
            let nd = vs.nodes[id];
            let root = match nd.kind {
                Kind::Store { out, .. } | Kind::Lea { out } | Kind::Addr { out, .. } => {
                    only.is_none_or(|o| o == out || (o == 0 && out == 2))
                }
                /* a side effect (`nir_instr_is_dead`: an intrinsic that
                 * cannot be eliminated) */
                Kind::Discard => true,
                _ => false,
            };
            if !(root || vs.live[id]) {
                continue;
            }
            vs.live[id] = true;
            for v in nd.src {
                if let Val::Lane(n, _) | Val::Pair(n, _) = v {
                    if nd.kind == Kind::Phi && !vs.live[n as usize] {
                        progress |= vs.order[k + 1..vs.len].contains(&n);
                    }
                    vs.live[n as usize] = true;
                }
            }
        }
    }
    if only.is_none() {
        let mut j = 0;
        for k in 0..vs.len {
            if vs.live[vs.order[k] as usize] {
                vs.order[j] = vs.order[k];
                j += 1;
            }
        }
        vs.len = j;
    }
}

/// [`dce`] of IDVS variant `out` of a vertex shader with kept loops
/// ([`vs_kept_begin`]): whether the variant keeps them. A loop none of the
/// variant's stores needs (no node of its body, header phi included, is
/// live without the head test's `if`) is dead whole, its `if` with it
/// (`nir_opt_dead_cf`): the variant is one block (gm2_intloop_uniform_vs's
/// varying program, where the loop feeds only the position). A variant
/// that needs one loop and not another has no reference.
pub fn variant_dce(vs: &mut Nir<'_>, out: u8) -> Result<bool, MaliError> {
    if vs.nloops == 0 {
        dce(vs, Some(out));
        return Ok(false);
    }
    let saved = vs.ifs;
    for r in vs.ifs[..vs.nifs].iter_mut() {
        r.live = false;
    }
    dce(vs, Some(out));
    vs.ifs = saved;
    let mut kept = 0;
    for r in &vs.loops[..vs.nloops] {
        let inside = |b: u8| b >= r.header && b <= r.latch;
        if vs.order[..vs.len].iter().any(|&id| vs.live[id as usize] && inside(vs.block[id as usize])) {
            kept += 1;
        }
    }
    if kept == 0 {
        return Ok(false);
    }
    if kept != vs.nloops {
        return Err(MaliError::LoopVertex);
    }
    dce(vs, Some(out));
    Ok(true)
}

/// `nir_opt_shrink_vectors` for the loads: the lanes up to the highest one
/// read.
fn shrink(vs: &mut Nir<'_>, m: &Module<'_>) -> Result<(), MaliError> {
    for k in 0..vs.len {
        let id = vs.order[k];
        let mut top = 0u8;
        let input = matches!(vs.nodes[id as usize].kind, Kind::Input { .. });
        for (u, s) in vs.uses(id) {
            let un = vs.nodes[u as usize];
            match un.src[s] {
                /* an attribute lane narrowed alone is lane `c` of the
                 * `f2fmp` of the whole vector's aligned pair, shrunk only
                 * after the load was (deqp_posvary_fract_mediump_vec3:
                 * `a_in0.zxy` narrowed, `FADD.f32 r1.h0, r2^` of z, the
                 * load `.v4`) */
                Val::Lane(_, c) if input && un.kind == Kind::Alu(Alu::F2f16) && un.bits == 16 && un.width == 1 => {
                    top = top.max((c | 1) + 1)
                }
                Val::Lane(_, c) => top = top.max(c + 1),
                /* a 16-bit node's first word ([`vs_pair16`]) */
                Val::Pair(_, Swz::H00) => top = top.max(1),
                Val::Pair(..) => top = top.max(2),
                _ => {}
            }
        }
        let nd = &mut vs.nodes[id as usize];
        match nd.kind {
            Kind::Input { loc, comps } => {
                nd.kind = Kind::Input { loc, comps: top.min(comps) };
                vs.prods |= 1 << Prod::AttrLane as u32;
            }
            Kind::Uniform { dword, comps } => {
                let (u, total) = uniform_of(m, dword)?;
                let p = match total {
                    _ if matrix_cols(u.gl_type).is_some() => Prod::UniformMat4Column,
                    _ if u.array_len > 1 => Prod::UniformVec4,
                    1 => Prod::UniformFloat,
                    2 => Prod::UniformVec2,
                    3 => Prod::UniformVec3,
                    4 => Prod::UniformVec4,
                    _ => Prod::UniformMat4Column,
                };
                vs.prods |= 1 << p as u32;
                if top < comps {
                    vs.prods |= 1 << Prod::UniformShrunk as u32;
                }
                vs.nodes[id as usize].kind = Kind::Uniform { dword, comps: top };
            }
            _ => {}
        }
    }
    Ok(())
}

/// `nir_opt_move_block` (nir_opt_move.c:46-118) over the one block: walked
/// backwards, each movable instruction is inserted before its first user;
/// an instruction already just before that user stays and keeps its own
/// index, so a later one that shares the user goes after it.
fn opt_move(vs: &mut Nir<'_>, scratch: &Arena<'_>) -> Result<(), MaliError> {
    move_before_users(vs, scratch, Kind::movable)
}

/// [`opt_move`] of the comparisons (`nir_move_comparisons`,
/// bifrost_nir.c:375-381; nir_opt_sink.c:94-95) over a mediump shader's
/// flattened ladder, one block (nir_fs.rs `guarded16`): each compare stands
/// right before the first select that reads it, so the passes' compares
/// come innermost first (cflow_71_deadloop.expect.json: `FCMP_OR.v2f16.ge`
/// of pass 7 before its `MUX.v2i16`, the constants pushed in that order,
/// `u3.w0` = (7.0, 7.0), `u3.w1` = (6.0, 6.0), `u4.w0` = (4.0, 5.0)).
/// An integer compare moves the same way (`nir_alu_instr_is_comparison`,
/// nir.c:3292-3314: `ilt`, `ult`, `ige`, `uge`, `ieq`, `ine` beside the
/// float ones): deqp_intval_reslib_bvec_fs's `bvec3 == bvec3`, each
/// `ICMP_OR.v2u16` right before the `LSHIFT_AND.v2i16` that reads it, and
/// the `FCMP_OR.v2f16` of the pair before the first of those.
/// A `b2i32` is a copy to the pass (nir_opt_sink.c:72, `nir_move_copies`)
/// and moves the same way (deqp_intval_reslib_mixed_fs's `CSEL.u32.eq` right
/// before the compare that reads it); so is a `vec2` of fp16 halves, moved
/// with `copies` (deqp_intval_opfs_sub_assign_ivec2_lowp's `MKVEC.v2i16`
/// right before the `FMA.v2f16` that reads it).
///
/// Mesa runs the pass twice on a fragment shader: at the link
/// (bifrost_nir.c:889-890, on the linker's NIR) and at the compile (:380-381,
/// after the bool lowering). Two compares that share a first user swap at
/// each run (the later one is already in place and keeps its index, the
/// earlier lands between), so the final order is the original one
/// (deqp_intval_opfs_logical_xor's two `FCMP_OR.v2f16` before the
/// `LSHIFT_XOR`, measured on Mesa 26.2.1's per-pass NIR with
/// `NIR_DEBUG=print_fs`); a caller that models both runs calls this twice.
pub(super) fn move_compares(vs: &mut Nir<'_>, scratch: &Arena<'_>, copies: bool) -> Result<(), MaliError> {
    /* `uge_pan` / `ult_pan` are comparisons too (nir.c:3292-3314):
     * `nir_lower_idiv`'s two `ICMP_OR.u32.ge` each right before the `CSEL`
     * that reads it (deqp_intval_opvs_div_ivec2_highp) */
    if copies {
        move_before_users(vs, scratch, |k| {
            matches!(k, Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_) | Alu::Ucmp(_) | Alu::B2i | Alu::Vec2))
        })
    } else {
        move_before_users(vs, scratch, |k| {
            matches!(k, Kind::Alu(Alu::Fcmp(_) | Alu::Icmp(_) | Alu::Ucmp(_) | Alu::B2i))
        })
    }
}

/// `nir_opt_move_block` of the instructions `movable` takes ([`opt_move`]).
fn move_before_users(vs: &mut Nir<'_>, scratch: &Arena<'_>, movable: fn(Kind) -> bool) -> Result<(), MaliError> {
    let index = scratch.alloc_filled(MAX_NODES, 0u32).map_err(|_| MaliError::OutOfScratch)?;
    let mut next = 1u32;
    let mut pos = vs.len;
    while pos > 0 {
        pos -= 1;
        let id = vs.order[pos];
        index[id as usize] = next;
        next += 1;
        if !movable(vs.nodes[id as usize].kind) {
            continue;
        }
        /* the first user: the user with the largest index */
        let mut first: Option<usize> = None;
        for (u, _) in vs.uses(id) {
            let at = vs.order[..vs.len].iter().position(|&x| x == u).ok_or(MaliError::Defect(Defect::IrIndex))?;
            if first.is_none_or(|f| index[u as usize] > index[vs.order[f] as usize]) {
                first = Some(at);
            }
        }
        let Some(mut f) = first else { continue };
        let fi = index[vs.order[f] as usize];
        while f > 0 && index[vs.order[f - 1] as usize] == fi && vs.order[f - 1] != id {
            f -= 1;
        }
        if f > 0 && vs.order[f - 1] == id {
            /* already before its user: it keeps its own index
             * (nir_opt_move.c:91-92, `continue` before the index is taken),
             * so an earlier instruction with the same user lands between it
             * and the user */
            continue;
        }
        /* remove from `pos`, insert before `f` (f > pos) */
        vs.order.copy_within(pos + 1..f, pos);
        vs.order[f - 1] = id;
        index[id as usize] = fi;
        if matches!(vs.nodes[id as usize].kind, Kind::Input { .. }) {
            let k = f - 1;
            if k > 0 && matches!(vs.nodes[vs.order[k - 1] as usize].kind, Kind::Input { .. }) {
                vs.note(Prod::SharedFirstUse);
            }
        }
    }
    Ok(())
}
