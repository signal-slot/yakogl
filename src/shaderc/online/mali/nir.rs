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
    GL_BOOL, GL_FLOAT, GL_FLOAT_MAT4, GL_FLOAT_VEC2, GL_FLOAT_VEC3, GL_FLOAT_VEC4, GL_INT, GL_INT_VEC2, GL_INT_VEC3,
    GL_INT_VEC4, GL_UNSIGNED_INT, GL_UNSIGNED_INT_VEC2, GL_UNSIGNED_INT_VEC3, GL_UNSIGNED_INT_VEC4,
};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir::{
    self, AluOp, IoKind, IoSlot, Module, Node as IrNode, Op as IrOp, ScalarType, TexDim, TexLod, ValueId,
};

/// The instructions one vertex shader may hold here (the fragment walk's
/// too). The largest measured, femtovg's fill_color with 256 offset arms
/// (case 194's offarm256), takes 611 (docs/notes/mali-int-and-size.md).
pub const MAX_NODES: usize = 768;
/// The IR values the walk keeps lanes for (the fragment walk's too).
/// offarm256 takes 1658, about six per arm.
pub const MAX_VALUES: usize = 2048;
/// The local variables the walk keeps lanes for (the fragment walk's too).
pub const MAX_LOCALS: usize = 32;
/// The `if`s one shader may hold ([`IfRegion`]): gl_test's databreak (case
/// 87) has nine, the eight guarded passes of its unrolled loop and the
/// `continue` of one of them.
pub const MAX_IFS: usize = 16;

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
            | Alu::B2i => 1,
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
    /// The first eight bytes of the texture descriptor of `sampler`'s
    /// look-up (nir_fs.rs; `pan_nir_load_va_desc(b, 4, 16, handle, 0)` of
    /// `pan_nir_load_va_tex_size`, pan_nir.h:106-140: a `load_ssbo` of the
    /// descriptor table, Mesa's `LD_PKA.i64`): two 32-bit lanes, the
    /// words, whose halves `textureSize` reads.
    TexDesc {
        sampler: u8,
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
    /// (pan_nir_lower_tex.c:802-973).
    VsTex {
        explicit: bool,
        cube: bool,
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
    };
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
    /// A vertex shader's look-up ([`Kind::VsTex`]): the sampler binding
    /// whose texture and sampler it reads at descriptor index
    /// [`super::nir_fs::TEX_INDEX`].
    pub sampler: Option<u8>,
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
}

const F32_ZERO: u32 = 0;
const F32_NEG_ZERO: u32 = 0x8000_0000;
const F32_ONE: u32 = 0x3f80_0000;
const F32_MINUS_ONE: u32 = 0xbf80_0000;
const F32_TWO: u32 = 0x4000_0000;
/// `MAX_W_RECIP_ABS` (nir_lower_viewport_transform.c:33): 2^15.
const W_RECIP_MIN: u32 = 0xc700_0000;
const W_RECIP_MAX: u32 = 0x4700_0000;
/// 1.0 / 16.0 and 16.0 * 256.0, `build_sfixed_8_8`'s factors.
const LOD_DIV16: u32 = 0x3d80_0000;
const LOD_8_8: u32 = 0x4580_0000;
/// 0.5: `build_cube_coord2_face`'s `0.5 / max{x, y, z}` and its addend.
const F32_HALF: u32 = 0x3f00_0000;
/// `BITFIELD_MASK(29)`: the S coordinate's bits of the cube descriptor,
/// the face index above them (`build_cube_desc`).
pub(super) const CUBE_S_MASK: u32 = 0x1fff_ffff;
/// 1.0 in fp16: `nir_lower_point_size`'s minimum (bifrost_nir.c passes
/// 1.0 and no maximum).
const F16_ONE: u32 = 0x3c00;

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
        /* a vec4 array, read element by element at constant indices
         * (gl_test cases 149 and 150's shape, uniarray_vsconst) */
        if u.gl_type as u32 == GL_FLOAT_VEC4 && u.dwords == 4 * u16::from(u.array_len) {
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
            sampler: None,
            consts: super::link::VaryingConsts::NONE,
            nonuniform_const: false,
            nonuniform: scratch.alloc_filled(MAX_NODES, false).map_err(oom)?,
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
            if !r.live {
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
                        if !uniforms && matches!(uk, Kind::Uniform { .. }) && !self.lane_read_above(u, c, b) {
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
                if !r.live || r.then_first != r.then_last || r.else_first != r.else_last {
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
                for k in 0..self.len {
                    let id = self.order[k] as usize;
                    if self.block[id] == r.after && self.nodes[id].kind == Kind::Phi {
                        let nd = self.nodes[id];
                        self.nodes[id] =
                            NirNode { kind: Kind::Alu(Alu::Bcsel), src: [r.cond, nd.src[0], nd.src[1]], ..nd };
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
                if let Val::Lane(y, _) = v {
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

    fn push(&mut self, kind: Kind, src: [Val; 3]) -> Result<u16, MaliError> {
        self.push_sized(kind, src, 32)
    }

    fn push_sized(&mut self, kind: Kind, src: [Val; 3], bits: u8) -> Result<u16, MaliError> {
        self.push_node(NirNode { kind, src, bits, width: 1 })
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
            let f = |k: usize| f32::from_bits(consts[k].unwrap_or(0));
            if consts[..op.arity()].iter().any(|c| !plain(c.unwrap_or(0))) {
                return Err(MaliError::ConstantFold);
            }
            let r = match op {
                Alu::Fadd => f(0) + f(1),
                Alu::Fmul => f(0) * f(1),
                /* a fused rounding: no_std has no fma, and no shape here
                 * makes one */
                Alu::Ffma => return Err(MaliError::ConstantFold),
                Alu::Frcp => 1.0 / f(0),
                Alu::Fmax => f(0).max(f(1)),
                Alu::Fmin => f(0).min(f(1)),
                /* the front end folds a constant expression: a constant
                 * operand here comes from a shape no reference draws */
                Alu::Fclamp(_)
                | Alu::Frsq
                | Alu::Ffloor
                | Alu::Fceil
                | Alu::Fcmp(_)
                | Alu::Icmp(_)
                | Alu::B2f
                | Alu::Bcsel
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
                | Alu::Ucmp(_) => return Err(MaliError::ConstantFold),
                Alu::F2f16
                | Alu::Fneg
                | Alu::Fabs
                | Alu::Vec2
                | Alu::Cubeface1
                | Alu::Cubeface2
                | Alu::CubeSsel
                | Alu::CubeTsel => return Err(MaliError::Defect(Defect::IrIndex)),
            };
            if !plain(r.to_bits()) {
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
            Alu::Fmul if uniform && (is_zero(a) || is_zero(b)) => {
                self.note_user(Prod::Identity);
                return Ok(Val::Const(F32_ZERO));
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
             * commutative match); the shader's own products */
            Alu::Fmul if self.user && self.is_alu(a, Alu::Fneg) && self.is_alu(b, Alu::Fneg) => {
                return Err(MaliError::ArithConstant);
            }
            Alu::Fmul if self.user && (self.is_alu(a, Alu::Fneg) || self.is_alu(b, Alu::Fneg)) => {
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
            let arity = match nd.kind {
                Kind::Alu(a) => a.arity(),
                Kind::Store { .. } | Kind::UniformIdx { .. } | Kind::Discard => 1,
                /* a fragment look-up's computed coordinate (nir_fs.rs
                 * `TexCoord::Computed`; undefined for the other forms) */
                Kind::Phi | Kind::VsTex { explicit: false, .. } => 2,
                /* a cube's third staging source (nir_fs.rs) */
                Kind::Tex | Kind::VsTex { explicit: true, .. } => 3,
                _ => 0,
            };
            (0..arity).filter(move |&s| matches!(nd.src[s], Val::Lane(n, _) if n == id)).map(move |s| (u, s))
        })
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

/// A front-end `ffma` is a matrix product's term (`mat_times_column` under
/// `gpu-mali`): the column a uniform load of a mat4, the scale a splat lane.
pub(super) fn is_mat_term(m: &Module<'_>, srcs: &[ir::Src; 3]) -> bool {
    let col = match m.def(srcs[0].val) {
        Some(IrOp::LoadUniform { dword_off, comps: 4 }) => *dword_off,
        _ => return false,
    };
    let is_mat = uniform_of(m, col).is_ok_and(|(u, _)| u.gl_type as u32 == GL_FLOAT_MAT4);
    let splat = srcs[1].swizzle.iter().all(|&s| s == srcs[1].swizzle[0]);
    let acc = matches!(m.def(srcs[2].val), Some(IrOp::Alu(AluOp::Fmul | AluOp::Ffma, _)));
    is_mat && splat && acc
}

/// Build the program of a vertex shader. `reads`: the fragment shader's
/// reads of the program (location, component mask), when the vertex shader
/// is built for one: then only those components are stored, compacted as
/// Mesa's link does ([`compact_layout`]); `moved`: the fragment shader's
/// expressions the link moves here (motion.rs), computed before the
/// stores and stored in their lanes.
pub fn build<'s>(
    m: &Module<'_>,
    scratch: &Arena<'s>,
    reads: Option<&[(u8, u8)]>,
    moved: &[super::motion::Moved],
) -> Result<Nir<'s>, MaliError> {
    if m.instrs.len() > MAX_VALUES {
        return Err(MaliError::TooManyValues);
    }
    if m.locals.len() > MAX_LOCALS {
        return Err(MaliError::TooManyLocals);
    }
    let mut vs = Nir::new(scratch)?;
    let vals = scratch.alloc_filled(MAX_VALUES, [Val::Undef; 4]).map_err(|_| MaliError::OutOfScratch)?;
    let w = &mut Walk { vals, locals: [[Val::Undef; 4]; MAX_LOCALS] };
    let mut position: Option<[Val; 4]> = None;
    let mut point_size: Option<Val> = None;
    let mut varyings: [Option<[Val; 4]>; MAX_VARYINGS] = [None; MAX_VARYINGS];
    let nodes = m.nodes_of(m.body);
    for (k, nd) in nodes.iter().enumerate() {
        let id = match *nd {
            IrNode::Instr(id) => id,
            IrNode::Return if k + 1 == nodes.len() => continue,
            _ => return Err(MaliError::ControlFlow),
        };
        let op = m.instr(id).ok_or(MaliError::Defect(Defect::IrIndex))?;
        let width = m.comps_of(ValueId(id.0)) as usize;
        let mut out = [Val::Undef; 4];
        match *op {
            IrOp::ConstF32(bits) => out = bits.map(Val::Const),
            /* a 32-bit integer constant is its bits (NIR's `load_const`
             * has no type); vid.vert's `% 4` and `== k`, inst.vert's `== 3` */
            IrOp::ConstI32(bits) => out = bits.map(|b| Val::Const(b as u32)),
            IrOp::ConstBool(_) => return Err(MaliError::IntValue),
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
                if u.gl_type as u32 == GL_INT && !index_only(m, ValueId(id.0)) {
                    return Err(MaliError::UniformType);
                }
                /* the load Mesa makes: the whole vector, or one column, or
                 * one element of a vec4 array */
                let (base, size) = if total == 16 || u.array_len > 1 {
                    (u.dword_off + (dword_off - u.dword_off) / 4 * 4, 4)
                } else {
                    (u.dword_off, total)
                };
                let first = dword_off - base;
                if first + comps as u16 > size {
                    return Err(MaliError::Defect(Defect::IrIndex));
                }
                let n = vs.load(Kind::Uniform { dword: base, comps: size as u8 })?;
                for (c, o) in out.iter_mut().enumerate().take(comps as usize) {
                    *o = Val::Lane(n, (first as usize + c) as u8);
                }
            }
            /* a float array's element at an int uniform: Mesa's 32-bit
             * `load_ubo(0, base + (index << 2))`, `LSHIFT_OR.i32`,
             * `IADD_IMM.i32` and `LD_PKA.i32` (uniarray_vs.expect.json's
             * varying program, docs/notes/mali-uniform-arrays.md) */
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
            IrOp::Alu(a, s) => {
                for (l, o) in out.iter_mut().enumerate().take(width) {
                    let x = |k: usize| lane(w, s[k], l);
                    *o = match a {
                        AluOp::Fadd => vs.alu(Alu::Fadd, x(0), x(1), Val::Undef)?,
                        AluOp::Fneg => vs.alu(Alu::Fneg, x(0), Val::Undef, Val::Undef)?,
                        /* `a / b` is `a * rcp(b)` from the front end */
                        AluOp::Frcp => vs.alu(Alu::Frcp, x(0), Val::Undef, Val::Undef)?,
                        AluOp::Fmul => vs.alu(Alu::Fmul, x(0), x(1), Val::Undef)?,
                        /* a matrix product's term by a constant 1.0 or -1.0:
                         * NIR's rules (:311-314) meet it inside the vector
                         * sums of the product, which no reference draws in
                         * this stage */
                        AluOp::Ffma
                            if is_mat_term(m, &s)
                                && [x(0), x(1)].iter().any(|v| matches!(v, Val::Const(F32_ONE | F32_MINUS_ONE))) =>
                        {
                            return Err(MaliError::ArithConstant);
                        }
                        AluOp::Ffma if is_mat_term(m, &s) => {
                            vs.note(Prod::MatProduct);
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
                            let mut acc = vs.alu(Alu::Fmul, va(n - 1), vb(n - 1), Val::Undef)?;
                            for i in (0..n - 1).rev() {
                                acc = vs.alu(Alu::Ffma, va(i), vb(i), acc)?;
                            }
                            acc
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
                        /* `ieq_pan`, a 0 / ~0 mask; `ior` of two masks is
                         * the compares' `ICMP_OR` (opt.rs `fuse_cmp`) */
                        AluOp::Ieq => vs.alu(Alu::Icmp(Cmpf::Eq), x(0), x(1), Val::Undef)?,
                        AluOp::Ior => vs.alu(Alu::Ior, x(0), x(1), Val::Undef)?,
                        /* `bcsel(a, b, b)` is `b` and `bcsel(a, 1.0, 0.0)`
                         * is `b2f32(a)` (nir_opt_algebraic.py), the
                         * `CSEL.u32.eq c, 0, 0, 1.0` of va_lower_isel */
                        AluOp::Bcsel if x(1) == x(2) => x(1),
                        AluOp::Bcsel if x(1) == Val::Const(F32_ONE) && x(2) == Val::Const(F32_ZERO) => {
                            vs.alu(Alu::B2f, x(0), Val::Undef, Val::Undef)?
                        }
                        AluOp::Bcsel => vs.alu(Alu::Bcsel, x(0), x(1), x(2))?,
                        /* the other integer operations and conversions
                         * (intbool) */
                        AluOp::Iadd
                        | AluOp::Imul
                        | AluOp::Idiv
                        | AluOp::Ineg
                        | AluOp::Ishl
                        | AluOp::Ishr
                        | AluOp::Iand
                        | AluOp::Ixor
                        | AluOp::Ilt
                        | AluOp::Ile
                        | AluOp::Ine
                        | AluOp::B2i
                        | AluOp::F2i
                        | AluOp::I2f
                        | AluOp::I2b
                        | AluOp::Udiv
                        | AluOp::Umod
                        | AluOp::Ushr
                        | AluOp::Ult
                        | AluOp::Ule
                        | AluOp::Imin
                        | AluOp::Imax
                        | AluOp::Umin
                        | AluOp::Umax
                        | AluOp::Inot
                        | AluOp::U2f
                        | AluOp::F2u => return Err(MaliError::IntValue),
                        _ => return Err(MaliError::Arithmetic),
                    };
                }
                /* a dot product is one value, whatever width the IR gives it */
                if matches!(a, AluOp::Fdot2 | AluOp::Fdot3 | AluOp::Fdot4) {
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
            } if vs.sampler.is_none() => {
                vs.sampler = Some(sampler);
                let cube = dim == TexDim::Cube;
                let (s, t) = if cube {
                    /* [`Nir::cube_desc`]. A constant lane of the direction
                     * stays an operand: the intrinsics fold no constant,
                     * and `va_lower_constants` makes it an immediate
                     * (case114_v.expect.json's `CUBEFACE1 r2, 0x3F800000,
                     * r1, r0`). A direction of constants alone is a shape
                     * no reference draws. */
                    let (x, y, z) = (lane(w, coord, 0), lane(w, coord, 1), lane(w, coord, 2));
                    if [x, y, z].iter().all(|v| matches!(v, Val::Const(_))) {
                        return Err(MaliError::TexVertexConstant);
                    }
                    vs.cube_desc(x, y, z)?
                } else {
                    let (s, t) = (lane(w, coord, 0), lane(w, coord, 1));
                    /* a coordinate of constants: each staging word its FAU
                     * word moved to a register (unitype_100.expect.json's
                     * `textureLod(vtex, vec2(0.25), 0.0)`, `MOV.i32 r2,
                     * u6.w0` and `MOV.i32 r3, u6.w0`). One constant lane
                     * beside a computed one has no reference (case 149) */
                    if matches!(s, Val::Const(_)) != matches!(t, Val::Const(_)) {
                        return Err(MaliError::TexVertexConstant);
                    }
                    (s, t)
                };
                let level = lane(w, level, 0);
                /* `nir_scalar_is_zero`: the bits 0 */
                let n = if level == Val::Const(F32_ZERO) {
                    vs.push(Kind::VsTex { explicit: false, cube }, [s, t, Val::Undef])?
                } else {
                    let x = vs.sfixed_8_8(level)?;
                    vs.push(Kind::VsTex { explicit: true, cube }, [s, t, x])?
                };
                for (c, o) in out.iter_mut().enumerate() {
                    *o = Val::Lane(n, c as u8);
                }
            }
            IrOp::Tex { .. } | IrOp::TexSize { .. } => return Err(MaliError::TexVertex),
            /* fragment only (the front end refuses it in a vertex shader) */
            IrOp::Discard => return Err(MaliError::DiscardForm),
        }
        if let Some(v) = w.vals.get_mut(id.0 as usize) {
            *v = out;
        }
    }
    /* an invariant output keeps its arithmetic exact in Mesa (no
     * contraction, the viewport transform's included) */
    if m.outputs.iter().any(|o| o.invariant) {
        return Err(MaliError::Invariant);
    }
    /* the constant lanes of the shader's own stores, which Mesa's link
     * propagates into the fragment shader before it moves code
     * (nir_opt_varyings.c:2880-2900, `propagate_uniform_expressions`: a
     * constant is a uniform expression): [`super::link::VaryingConsts`] */
    for o in m.outputs.iter().filter(|o| o.kind == IoKind::Varying) {
        let Some(lanes) = varyings.get(o.location as usize).copied().flatten() else { continue };
        for (c, l) in lanes.iter().enumerate().take(o.components as usize) {
            match *l {
                Val::Const(bits) => vs.consts.push(o.location, c as u8, bits),
                /* a lane of uniforms alone, which the link promotes to
                 * flat ([`super::link::VaryingConsts::push_flat`]) */
                Val::Lane(n, _) if vs.convergent(n) => vs.consts.push_flat(o.location, c as u8),
                _ => {}
            }
        }
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
    vs.nvar = generic_layout(decls.clone().map(|o| (o.location, o.components)), &mut vs.varyings)?;
    /* a shader alone that stores a constant lane is built for its own lanes
     * less its constant ones, the layout the link's fragment shader reads
     * once they are folded into it (codegen.rs `compile_vs_without_consts`,
     * which this makes the shader's own compile) */
    let mut own = [(0u8, 0u8); MAX_VARYINGS];
    let mut nown = 0;
    let konst = |o: &&ir::IoDecl<'_>, c: u8| {
        varyings.get(o.location as usize).copied().flatten().is_some_and(|l| matches!(l[c as usize & 3], Val::Const(_)))
    };
    let reads = match reads {
        None if decls.clone().any(|o| (0..o.components).any(|c| konst(&o, c))) => {
            for o in decls {
                let mask = (0..o.components).filter(|&c| !konst(&o, c)).fold(0u8, |m, c| m | 1 << c);
                if mask == 0 {
                    continue;
                }
                *own.get_mut(nown).ok_or(MaliError::VaryingLocation)? = (o.location, mask);
                nown += 1;
            }
            Some(&own[..nown])
        }
        r => r,
    };
    compact_layout(&mut vs, reads);
    let mut seen = [Val::Undef; 4 * MAX_VARYINGS];
    let mut nseen = 0;
    /* (a lane of uniforms alone stored, an interpolated lane stored): the
     * link packs flat lanes in slots of their own, a layout no reference
     * draws beside interpolated ones */
    let (mut flat, mut smooth) = (false, false);
    for k in 0..vs.nvar {
        let v = vs.varyings[k];
        if vs.masks[k] != (1 << v.comps) - 1 {
            /* a part (or none) of it is stored */
            vs.note(Prod::VaryingCompacted);
        }
        let lanes = varyings.get(v.location as usize).copied().flatten().ok_or(MaliError::VaryingForm)?;
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
        for l in stored.map(|c| lanes[c]) {
            match l {
                Val::Undef => return Err(MaliError::VaryingForm),
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
                Val::Lane(n, _) if vs.uniform_only(n) => flat = true,
                /* no attribute behind it, and no reference of its form
                 * (a phi of uniforms) */
                Val::Lane(n, _) if !vs.reads_attribute(n) && !vs.reads_tex(n) && !vs.reads_indexed(n) => {
                    return Err(MaliError::VaryingNotFromAttribute)
                }
                Val::Lane(..) => smooth = true,
                Val::Pair(..) => return Err(MaliError::Defect(Defect::IrIndex)),
            }
            if seen[..nseen].contains(&l) {
                return Err(MaliError::VaryingDuplicate);
            }
            seen[nseen] = l;
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
    if flat && smooth {
        return Err(MaliError::VaryingNotFromAttribute);
    }
    /* the viewport transform, before the position's store */
    vs.user = false;
    vs.user_end = vs.n;
    let scale = vs.load(Kind::Sysval(Sysval::ViewportScale))?;
    let offset = vs.load(Kind::Sysval(Sysval::ViewportOffset))?;
    vs.note(if matches!(p[3], Val::Const(_)) { Prod::ViewportConstW } else { Prod::ViewportW });
    let r = vs.alu(Alu::Frcp, p[3], Val::Undef, Val::Undef)?;
    let r = vs.alu(Alu::Fmax, r, Val::Const(W_RECIP_MIN), Val::Undef)?;
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
        for k in 0..vs.nvar {
            let v = vs.varyings[k];
            let Some(lanes) = varyings.get_mut(v.location as usize).and_then(|l| l.as_mut()) else { continue };
            for (c, l) in lanes.iter_mut().enumerate().take(v.comps as usize) {
                let Val::Lane(n, _) = *l else { continue };
                if vs.masks[k] & (1 << c) == 0 || vs.reads_attribute(n) {
                    continue;
                }
                vs.note(Prod::VaryingConvergent);
                let x = *l;
                let f = NirNode { kind: Kind::Alu(Alu::Ffma), src: [x, Val::Const(F32_ZERO), x], bits: 32, width: 1 };
                *l = Val::Lane(vs.push_node(f)?, 0);
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
    fuse(&mut vs);
    late_rules(&mut vs);
    dce(&mut vs, None);
    /* a look-up no output reads binds nothing (`textures_used` after DCE) */
    if !vs.order[..vs.len].iter().any(|&id| matches!(vs.nodes[id as usize].kind, Kind::VsTex { .. })) {
        vs.sampler = None;
    }
    shrink(&mut vs, m)?;
    opt_move(&mut vs, scratch)?;
    Ok(vs)
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

/// The late fusion, last instruction first (nir_search.c's worklist pops
/// the last instruction first).
pub(super) fn fuse(vs: &mut Nir<'_>) {
    for k in (0..vs.len).rev() {
        let x = vs.order[k] as usize;
        if vs.nodes[x].kind != Kind::Alu(Alu::Fadd) {
            continue;
        }
        /* the rules in their list order (nir_opt_algebraic.py:3816-3820),
         * each trying the fadd's sources in order: `fadd(fmul(a, b), c)`,
         * then `fadd(fneg(fmul(a, b)), c)` -> `ffma(fneg(a), b, c)` */
        let product = |vs: &Nir<'_>, v: Val| match v {
            Val::Lane(f, 0) if vs.nodes[f as usize].kind == Kind::Alu(Alu::Fmul) && vs.live_in_order(f) => {
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
            let Val::Lane(g, 0) = vs.nodes[x].src[s] else { continue };
            let gn = vs.nodes[g as usize];
            if gn.kind != Kind::Alu(Alu::Fneg) || !vs.live_in_order(g) || !vs.only_used_by_fadd(g) {
                continue;
            }
            let Some(f) = product(vs, gn.src[0]) else { continue };
            let other = vs.nodes[x].src[1 - s];
            let fm = vs.nodes[f as usize];
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

impl Nir<'_> {
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
    fn insert_before(&mut self, k: usize, nd: NirNode) -> Result<u16, MaliError> {
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
    for k in (0..vs.len).rev() {
        let id = vs.order[k] as usize;
        let nd = vs.nodes[id];
        let root = match nd.kind {
            Kind::Store { out, .. } | Kind::Lea { out } | Kind::Addr { out, .. } => {
                only.is_none_or(|o| o == out || (o == 0 && out == 2))
            }
            /* a side effect (`nir_instr_is_dead`: an intrinsic that cannot
             * be eliminated) */
            Kind::Discard => true,
            _ => false,
        };
        if !(root || vs.live[id]) {
            continue;
        }
        vs.live[id] = true;
        for v in nd.src {
            if let Val::Lane(n, _) = v {
                vs.live[n as usize] = true;
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

/// `nir_opt_shrink_vectors` for the loads: the lanes up to the highest one
/// read.
fn shrink(vs: &mut Nir<'_>, m: &Module<'_>) -> Result<(), MaliError> {
    for k in 0..vs.len {
        let id = vs.order[k];
        let mut top = 0u8;
        for (u, s) in vs.uses(id) {
            if let Val::Lane(_, c) = vs.nodes[u as usize].src[s] {
                top = top.max(c + 1);
            }
        }
        let nd = &mut vs.nodes[id as usize];
        match nd.kind {
            Kind::Input { loc, .. } => {
                nd.kind = Kind::Input { loc, comps: top };
                vs.prods |= 1 << Prod::AttrLane as u32;
            }
            Kind::Uniform { dword, comps } => {
                let (u, total) = uniform_of(m, dword)?;
                let p = match total {
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
    let index = scratch.alloc_filled(MAX_NODES, 0u32).map_err(|_| MaliError::OutOfScratch)?;
    let mut next = 1u32;
    let mut pos = vs.len;
    while pos > 0 {
        pos -= 1;
        let id = vs.order[pos];
        index[id as usize] = next;
        next += 1;
        if !vs.nodes[id as usize].kind.movable() {
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
