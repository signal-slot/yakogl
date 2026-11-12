// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `bi_emit_alu` (bifrost_compile.c:2621-3300): one NIR ALU operation, by
//! its op and bit size, to the BIR Mesa selects for it. The selection is
//! plain: an fneg is `FABSNEG` and an FADD with a constant an FADD, and the
//! passes after it fold them (opt.rs), as in Mesa.
//!
//! Each row is one `case` of `bi_emit_alu`; the populations of tests.rs
//! draw every row ([`ROWS`], `the_selection_rows_are_the_references`).

use super::bir::{clamp, Cmpf, Dest, Idx, Ins, Op, Prog, ResultType, Round, Src, Swz};
use super::nir::Alu;
use super::select::alu as ins;
use super::MaliError;

/// `bi_negzero()`: the zero with `.neg`, the addend of `fmul`'s FMA.
const NEG_ZERO: Src = Src { neg: true, ..Src::of(Idx::Const(0)) };
/// `bi_zero()`: the `_OR` compares' third source.
const ZERO: Src = Src::of(Idx::Const(0));
/// `bi_imm_f32(1.0)`.
const ONE: Src = Src::of(Idx::Const(0x3f80_0000));
/// `bi_imm_f16(1.0)`: `bi_half(bi_imm_u32(0x3c00), false)`, the 16-bit
/// constant read as its low half.
const ONE_F16: Src = Src { swz: Swz::H00, ..Src::of(Idx::Const(0x3c00)) };

/// A NIR op at a bit size and width, as [`emit`] takes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub op: Alu,
    pub bits: u8,
    pub width: u8,
}

const fn row(op: Alu, bits: u8, width: u8) -> Row {
    Row { op, bits, width }
}

/// The rows [`emit`] selects, each with the `bi_emit_alu` case it is.
pub const ROWS: [(Row, &str); 93] = [
    (row(Alu::Fadd, 32, 1), "nir_op_fadd: FADD.f32"),
    (row(Alu::Fmul, 32, 1), "nir_op_fmul: FMA.f32 a, b, -0.0"),
    (row(Alu::Ffma, 32, 1), "nir_op_ffma: FMA.f32"),
    (row(Alu::Frcp, 32, 1), "nir_op_frcp: FRCP.f32"),
    (row(Alu::Fmax, 32, 1), "nir_op_fmax: FMAX.f32"),
    (row(Alu::Fmin, 32, 1), "nir_op_fmin: FMIN.f32"),
    (row(Alu::Fneg, 32, 1), "nir_op_fneg: FABSNEG.f32 a.neg"),
    (row(Alu::F2f16, 16, 1), "nir_op_f2f16 of a scalar: FADD.f32 a, -0.0 into the low half"),
    (row(Alu::Fmax, 16, 1), "nir_op_fmax: FMAX.v2f16"),
    (row(Alu::F2f16, 16, 2), "nir_op_f2f16 of two lanes: V2F32_TO_V2F16"),
    (row(Alu::Fadd, 16, 2), "nir_op_fadd: FADD.v2f16"),
    (row(Alu::Fmul, 16, 2), "nir_op_fmul: FMA.v2f16 a, b, -0.0"),
    (row(Alu::Ffma, 16, 2), "nir_op_ffma: FMA.v2f16"),
    (row(Alu::Fneg, 16, 2), "nir_op_fneg: FABSNEG.v2f16 a.neg"),
    (row(Alu::Fabs, 16, 2), "nir_op_fabs: FABSNEG.v2f16 a.abs"),
    (row(Alu::Vec2, 16, 2), "nir_op_vec2 of two 16-bit halves: MKVEC.v2i16"),
    /* the S4 rows (#31), one shim reference each (tools/mali/testdata/s4_*) */
    (row(Alu::Fabs, 32, 1), "nir_op_fabs: FABSNEG.f32 a.abs"),
    (row(Alu::Fclamp(clamp::ZERO_ONE), 32, 1), "nir_op_fsat: FCLAMP.f32 .clamp_0_1"),
    (row(Alu::Frsq, 32, 1), "nir_op_frsq: FRSQ.f32"),
    (row(Alu::Ffloor, 32, 1), "nir_op_ffloor: FROUND.f32.rtn"),
    (row(Alu::Fceil, 32, 1), "nir_op_fceil: FROUND.f32.rtp"),
    (row(Alu::Fcmp(Cmpf::Lt), 32, 1), "nir_op_flt_pan: FCMP.f32.lt.m1"),
    (row(Alu::Fcmp(Cmpf::Ge), 32, 1), "nir_op_fge_pan: FCMP.f32.ge.m1"),
    (row(Alu::Fcmp(Cmpf::Eq), 32, 1), "nir_op_feq_pan: FCMP.f32.eq.m1"),
    (row(Alu::Icmp(Cmpf::Lt), 32, 1), "nir_op_ilt_pan: ICMP.s32.lt.m1"),
    (row(Alu::B2f, 32, 1), "nir_op_b2f32 as bcsel_pan(c, 1.0, 0.0): MUX.i32.int_zero 0.0, 1.0, c"),
    (row(Alu::B2f, 16, 1), "nir_op_b2f16 as bcsel_pan(i2i16(c), 1.0, 0.0): MUX.v2i16.int_zero 0, 1.0, c.h00"),
    (row(Alu::Bcsel, 32, 1), "nir_op_bcsel_pan: MUX.i32.int_zero else, then, c"),
    (row(Alu::F2i, 32, 1), "nir_op_f2i32: F32_TO_S32.rtz"),
    (row(Alu::I2f, 32, 1), "nir_op_i2f32: S32_TO_F32"),
    (row(Alu::Iadd, 32, 1), "nir_op_iadd: IADD.s32"),
    /* gl_FragCoord's x and y (#31 S4, tools/mali/testdata/fvg_fill_gradient_*) */
    (row(Alu::U2f32, 32, 1), "nir_op_u2f32 of a 16-bit source: U16_TO_F32"),
    /* femtovg's scissor `max(d, 0.0)` (#31 S5, fvg_fill_color_nodiscard) */
    (row(Alu::Fclamp(clamp::ZERO_INF), 32, 1), "nir_op_fclamp_pos: FCLAMP.f32 .clamp_0_inf"),
    /* a vertex look-up's 8.8 level (#32, tools/mali/testdata/tex_vs_lod) */
    (row(Alu::Fclamp(clamp::M1_1), 32, 1), "nir_op_fsat_signed: FCLAMP.f32 .clamp_m1_1"),
    (row(Alu::F2i, 16, 1), "nir_op_f2i16 of a 32-bit source: F32_TO_S32.rtz"),
    (row(Alu::U2u32, 32, 1), "nir_op_u2u32 of a 16-bit source: U16_TO_U32"),
    /* femtovg's `texType == 1`, `glyphTextureType == 1` (#31 S5,
     * fvg_texture_copy_unclipped, fvg_fill_color_glyph_nodiscard) */
    (row(Alu::Icmp(Cmpf::Eq), 32, 1), "nir_op_ieq_pan: ICMP.i32.eq.m1, ICMP_OR.u32 after va_lower_isel"),
    /* a uniform array element read at a run-time index (#32 (b),
     * tools/mali/testdata/uniidx_d, uniidx_i): the byte offset */
    (row(Alu::Ishl, 32, 1), "nir_op_ishl: LSHIFT_OR.i32 a, #0, b.b0"),
    /* a vertex cube-map look-up's coordinate (#32 (c), tools/mali/testdata/
     * cube_vs: `build_cube_coord2_face` and `build_cube_desc`,
     * pan_nir_lower_tex.c:68-128, the intrinsics bifrost_compile.c:2109-2138) */
    (row(Alu::Cubeface1, 32, 1), "nir_intrinsic_cubeface_pan, lane 0: CUBEFACE1 x, y, z"),
    (row(Alu::Cubeface2, 32, 1), "nir_intrinsic_cubeface_pan, lane 1: CUBEFACE2_V9 x, y, z"),
    (row(Alu::CubeSsel, 32, 1), "nir_intrinsic_cube_ssel_pan: CUBE_SSEL z, x, face"),
    (row(Alu::CubeTsel, 32, 1), "nir_intrinsic_cube_tsel_pan: CUBE_TSEL y, z, face"),
    (row(Alu::BitfieldSelect, 32, 1), "nir_op_bitfield_select: MUX.i32.bit b, c, mask"),
    /* atan's sign, `nir_copysign` (#31 S4, fvg_fill_gradient_conic*) */
    (row(Alu::Iand, 32, 1), "nir_op_iand: LSHIFT_AND.i32 a, b, #0"),
    (row(Alu::Ior, 32, 1), "nir_op_ior: LSHIFT_OR.i32 a, b, #0"),
    /* a run-time index's store tree (case 80, other_80_matdyn word 8) */
    (row(Alu::Inot, 32, 1), "nir_op_inot: LSHIFT_OR.i32 #0, a.not, #0"),
    /* the sum of a mediump float array's elements, one lane each (case 108
     * A, tools/mali/testdata/case108_uniidx: `FADD.v2f16 r1, r1^.h00,
     * r2^.h00`) */
    (row(Alu::Fadd, 16, 1), "nir_op_fadd of one 16-bit lane: FADD.v2f16 a.h00, b.h00"),
    /* a mediump computed texture coordinate (case 121, tools/mali/testdata/
     * case121: `FMA.v2f16 r2, r1.h11, 0x38000000.h11, 0x0.neg`, then
     * `F16_TO_F32 r2, r2^.h0`) */
    (row(Alu::Fmul, 16, 1), "nir_op_fmul of one 16-bit lane: FMA.v2f16 a.hN, b.hN, -0.0"),
    (row(Alu::F2f32, 32, 1), "nir_op_f2f32 of a 16-bit source: F16_TO_F32"),
    /* a mediump discard's compare (tools/mali/testdata/alu_branchlane,
     * alu_branchquad: `DISCARD.f32.lt 0x38000000.h1, u2.w0.h1` once fused) */
    (row(Alu::Fcmp(Cmpf::Lt), 16, 1), "nir_op_flt_pan of one 16-bit lane: FCMP.v2f16.lt.m1 a.hN, b.hN"),
    /* a signed divide by a constant power of two, `imul(isign(a), ushr(iabs(a),
     * log2 b))` with `isign` lowered (gl_test's isamp, tools/mali/testdata/
     * fbload_isamp: `ivec2(gl_FragCoord.xy) / 32`) */
    (row(Alu::Imax, 32, 1), "nir_op_imax: CSEL.s32.gt a, b, a, b"),
    (row(Alu::Imin, 32, 1), "nir_op_imin: CSEL.s32.lt a, b, a, b"),
    (row(Alu::Iabs, 32, 1), "nir_op_iabs: IABS.s32"),
    (row(Alu::Ushr, 32, 1), "nir_op_ushr: RSHIFT_OR.i32 a, #0, b.b0"),
    (row(Alu::Imul, 32, 1), "nir_op_imul: IMUL.i32"),
    /* a highp ES 1.00 shader's mediump arithmetic on gl_FragCoord
     * (tools/mali/testdata/fbload_predif_fs_coord, prec_82) */
    (row(Alu::I2i32, 32, 1), "nir_op_i2i32 of a 16-bit mask: S16_TO_S32"),
    (row(Alu::Bcsel, 16, 1), "nir_op_bcsel_pan of one 16-bit lane: MUX.v2i16.int_zero else, then, c.h00"),
    /* a flattened `if`'s select of two uniform lanes (tools/mali/testdata/
     * cflow_68_armselect: `MUX.v2i16 r1, u2.w0, u2.w1, r0.h00`) */
    (row(Alu::Bcsel, 16, 2), "nir_op_bcsel_pan of two 16-bit lanes: MUX.v2i16.int_zero else, then, c.h00"),
    (row(Alu::Ffloor, 16, 2), "nir_op_ffloor of two 16-bit lanes: FROUND.v2f16.rtn"),
    /* a colour lane paired with a constant (tools/mali/testdata/texmore_2:
     * `FMA.v2f16 r1, r3^.h00, 0x38000000.h11, r1^.h00`) */
    (row(Alu::Ffma, 16, 1), "nir_op_ffma of one 16-bit lane: FMA.v2f16 a.hN, b.hN, c.hN"),
    /* a uniform's component negated beside a constant (case 75, tools/mali/
     * testdata/varyconst_75_immneg: `FADD.v2f16 r1, 0x3C000000.h11,
     * u2.w1.neg.h00`, the fneg a modifier of its reader) */
    (row(Alu::Fneg, 16, 1), "nir_op_fneg of one 16-bit lane: FABSNEG.v2f16 a.hN.neg"),
    /* a mediump value's integer (case 72, tools/mali/testdata/intbig_case72:
     * `F16_TO_S32.rtz r0, u2.w0.h0`) */
    (row(Alu::F2i32, 32, 1), "nir_op_f2i32 of a 16-bit source: F16_TO_S32.rtz"),
    /* an int uniform's shift, byte and sign (tools/mali/testdata/intuni_ishr:
     * `ARSHIFT_OR.i32 r0, u2.w0, r0^.b00, 0x0`, `U8_TO_F32 r1, r0.b0`,
     * `U8_TO_F32 r0, r0^.b1`) */
    (row(Alu::Ishr, 32, 1), "nir_op_ishr: ARSHIFT_OR.i32 a, #0, b.b0"),
    (row(Alu::U8f32(0), 32, 1), "nir_op_u2f32 of extract_u8(a, 0): U8_TO_F32 a.b0"),
    (row(Alu::U8f32(1), 32, 1), "nir_op_u2f32 of extract_u8(a, 1): U8_TO_F32 a.b1"),
    /* a mediump `<=` of integers (tools/mali/testdata/intloop_int_compare:
     * `ICMP_OR.s32.ge.m1 r3, r0, r1, 0x0`) */
    (row(Alu::Icmp(Cmpf::Ge), 32, 1), "nir_op_ige_pan: ICMP.s32.ge.m1"),
    /* integer division, `nir_lower_idiv` (case 79, tools/mali/testdata/
     * intbig_case79: `U32_TO_F32`, `FRCP.f32`, `F32_TO_U32.rtz`, the 16-bit
     * halves' products, `ICMP_OR.u32.ge`, `LSHIFT_XOR.i32`), and the
     * absolute value of one fp16 lane (`FADD.v2f16 r2, r4^.abs.h00`) */
    (row(Alu::Ixor, 32, 1), "nir_op_ixor: LSHIFT_XOR.i32 a, b, #0"),
    (row(Alu::Isub, 32, 1), "nir_op_isub: ISUB.s32"),
    (row(Alu::Ineg, 32, 1), "nir_op_ineg: ISUB.s32 0, a"),
    (row(Alu::U2f, 32, 1), "nir_op_u2f32 of a 32-bit source: U32_TO_F32"),
    (row(Alu::F2u, 32, 1), "nir_op_f2u32: F32_TO_U32.rtz"),
    (row(Alu::Extract16(0), 32, 1), "nir_op_extract_u16(a, 0): U16_TO_U32 a.h0"),
    (row(Alu::Extract16(1), 32, 1), "nir_op_extract_u16(a, 1): U16_TO_U32 a.h1"),
    (row(Alu::B2i, 32, 1), "nir_op_b2i32: MUX.i32.int_zero 0, 1, c"),
    (row(Alu::Ucmp(Cmpf::Ge), 32, 1), "nir_op_uge_pan: ICMP.u32.ge.m1"),
    (row(Alu::Fabs, 16, 1), "nir_op_fabs of one 16-bit lane: FABSNEG.v2f16 a.hN.abs"),
    /* `textureSize`'s null test of the descriptor's first half (case 42,
     * tools/mali/testdata/intbig_case42: `ICMP_OR.v2u16.eq.m1 r1, r2^.h00,
     * 0x0, 0x0`) */
    (row(Alu::Icmp(Cmpf::Eq), 16, 1), "nir_op_ieq_pan of one 16-bit lane: ICMP_OR.v2u16.eq.m1 a.h00, b"),
    /* the same test kept as a branch (`inot(ieq)` is `ine`; texmore_46's
     * array, texmore_48: `ICMP_OR.v2u16.ne.m1 r0, r0^.h00, 0x0, 0x0` then
     * `BRANCHZ.eq`) */
    (row(Alu::Icmp(Cmpf::Ne), 16, 1), "nir_op_ine_pan of one 16-bit lane: ICMP_OR.v2u16.ne.m1 a.h00, b"),
    /* a kept loop's `float(n)` of `int n = int(x)` (intloop_loop_exit) */
    (row(Alu::Ftrunc, 32, 1), "nir_op_ftrunc: FROUND.f32.rtz"),
    /* a mediump unrolled loop's ladder (tools/mali/testdata/cflow_71_deadloop:
     * `FROUND.v2f16.rtp r1, r0.h00`, `FMIN.v2f16 r1, r1^.h00, 0x48000000.h11`,
     * `FCMP_OR.v2f16.ge.m1 r3, u3.w0, r1.h00, 0x0` of one pass, `.lt` of two) */
    (row(Alu::Fceil, 16, 1), "nir_op_fceil of one 16-bit lane: FROUND.v2f16.rtp a.hN"),
    (row(Alu::Fmin, 16, 1), "nir_op_fmin of one 16-bit lane: FMIN.v2f16 a.hN, b.hN"),
    (row(Alu::Fcmp(Cmpf::Ge), 16, 1), "nir_op_fge_pan of one 16-bit lane: FCMP.v2f16.ge.m1 a.hN, b.hN"),
    (row(Alu::Fcmp(Cmpf::Lt), 16, 2), "nir_op_flt_pan of two 16-bit lanes: FCMP.v2f16.lt.m1"),
    /* a mediump `clamp(x, 0.0, 1.0)` (tools/mali/testdata/cflow_142_unibranch:
     * `FADD.v2f16.clamp_0_1 r2, r2^, r7^` of a pair, `r4^.h00, r6^.h11` of a
     * lane, the FCLAMP folded into its producer) */
    (row(Alu::Fclamp(clamp::ZERO_ONE), 16, 1), "nir_op_fsat of one 16-bit lane: FCLAMP.v2f16 .clamp_0_1"),
    (row(Alu::Fclamp(clamp::ZERO_ONE), 16, 2), "nir_op_fsat of two 16-bit lanes: FCLAMP.v2f16 .clamp_0_1"),
    /* a texel's copy for the phi at the join of an `if` Mesa keeps
     * (tools/mali/testdata/cflow_148_texbranch: `U16_TO_U32 r1, r1^.h0`
     * after each look-up under an `if`, the MKVEC lowered by opt.rs) */
    (row(Alu::Mov, 16, 1), "nir_op_mov of one 16-bit lane: MKVEC.v2i16 a.hN, #0 (bi_make_vec_to)"),
    /* `any` / `all` of mediump compares (tools/mali/testdata/intbool_76_boolany:
     * `LSHIFT_OR.v2i16 r3, r2.h11, 0x0.b00, r2`, `LSHIFT_AND.v2i16 r2,
     * r2^.h00, 0x0.b00, r4^` with `IADD.v2u16 r4, r1.h11, 0x0`) */
    (row(Alu::Ior, 16, 1), "nir_op_ior of one 16-bit lane: LSHIFT_OR.v2i16 a.hN, b, #0"),
    (row(Alu::Iand, 16, 1), "nir_op_iand of one 16-bit lane: LSHIFT_AND.v2i16 a.hN, b, #0"),
    /* a mediump `dFdx` / `dFdy` of one lane (case 138 deriv, tools/mali/
     * testdata/fsarith_138_deriv: `CLPER.i32.subgroup4.xor r4, r0,
     * 0x1000000.b33`, `FADD.v2f16 r0, r4^, r0^.neg`, `LSHIFT_XOR.v2i16 r0,
     * lane_id.w0.h00, 0xF0E0D0C.b33, r0^`; Y with `0x3020100.b22`, the mask
     * `LSHIFT_AND.i32 r4, lane_id.w0, 0x0.b00, r4^` of the pushed 2 and
     * `0xF0E0D0C.b22`; docs/notes/mali-derivatives.md 1) */
    (row(Alu::Fddx, 16, 1), "ddx_fine of one 16-bit lane: CLPER.i32.xor a, #1; FADD.v2f16 n, a.neg; LSHIFT_XOR.v2i16 lane_id.h00, #15, d"),
    (row(Alu::Fddy, 16, 1), "ddy_fine of one 16-bit lane: CLPER.i32.xor a, #2; FADD.v2f16 n, a.neg; LSHIFT_XOR.v2i16 (lane_id & 2).h00, #14, d"),
    /* a mediump `normalize` and `length` (tools/mali/testdata/
     * fsvec_normalize_mediump, fsvec_length_mediump: `FRSQ.f16 r2,
     * r2^.h00`; `FRSQ.f16 r0, r0^.h00` then `FRCP.f16 r0, r0^.h00`) */
    (row(Alu::Frsq, 16, 1), "nir_op_frsq of one 16-bit lane: FRSQ.f16 a.hN"),
    (row(Alu::Frcp, 16, 1), "nir_op_frcp of one 16-bit lane: FRCP.f16 a.hN"),
    /* a mediump `fract` (tools/mali/testdata/fsvec_fract_mediump:
     * `FROUND.v2f16.rtn r1, r0.h00`) */
    (row(Alu::Ffloor, 16, 1), "nir_op_ffloor of one 16-bit lane: FROUND.v2f16.rtn a.hN"),
];

/// A source of a one-lane 16-bit operation: its lane's half in both
/// (`bi_alu_src_index` replicates the lane read): the high half kept where
/// the caller selects it (a uniform's odd component, `u2.w0.h11`), else
/// the low half.
fn lone(s: Src) -> Src {
    s.swz(if s.swz == Swz::H11 { Swz::H11 } else { Swz::H00 })
}

/// `row`'s bit in a set of [`ROWS`] (a program's rows drawn).
pub fn bit(row: Row) -> u128 {
    ROWS.iter().position(|r| r.0 == row).map_or(0, |i| 1 << i)
}

/// The value of `row` over sources `s` (its arity read), appended to `p`.
pub fn emit(p: &mut Prog<'_>, row: Row, s: &[Src]) -> Result<u16, MaliError> {
    Ok(match (row.op, row.bits, row.width) {
        (Alu::Fadd, 32, 1) => ins(p, Op::FaddF32, &s[..2])?,
        (Alu::Fmul, 32, 1) => ins(p, Op::FmaF32, &[s[0], s[1], NEG_ZERO])?,
        (Alu::Ffma, 32, 1) => ins(p, Op::FmaF32, &s[..3])?,
        (Alu::Frcp, 32, 1) => ins(p, Op::FrcpF32, &s[..1])?,
        (Alu::Fmax, 32, 1) => ins(p, Op::FmaxF32, &s[..2])?,
        (Alu::Fmin, 32, 1) => ins(p, Op::FminF32, &s[..2])?,
        (Alu::Fneg, 32, 1) => ins(p, Op::FabsnegF32, &[Src { neg: !s[0].neg, ..s[0] }])?,
        (Alu::F2f16, 16, 1) => {
            let t = p.temp()?;
            let d = Dest { idx: Idx::Ssa(t, 0), swz: Swz::H00 };
            p.push(super::bir::Ins::new(Op::FaddF32, &[d], &[s[0], NEG_ZERO]))?;
            t
        }
        (Alu::Fmax, 16, 1) => ins(p, Op::FmaxV2f16, &[s[0].swz(Swz::H00), s[1].swz(Swz::H00)])?,
        (Alu::Fadd, 16, 1) => ins(p, Op::FaddV2f16, &[lone(s[0]), lone(s[1])])?,
        (Alu::F2f16, 16, 2) => ins(p, Op::V2f32ToV2f16, &s[..2])?,
        (Alu::Fadd, 16, 2) => ins(p, Op::FaddV2f16, &s[..2])?,
        (Alu::Fmul, 16, 2) => ins(p, Op::FmaV2f16, &[s[0], s[1], NEG_ZERO])?,
        (Alu::Ffma, 16, 2) => ins(p, Op::FmaV2f16, &s[..3])?,
        (Alu::Fneg, 16, 2) => ins(p, Op::FabsnegV2f16, &[Src { neg: !s[0].neg, ..s[0] }])?,
        (Alu::Fneg, 16, 1) => ins(p, Op::FabsnegV2f16, &[Src { neg: !s[0].neg, ..lone(s[0]) }])?,
        (Alu::Fabs, 16, 2) => ins(p, Op::FabsnegV2f16, &[Src { abs: true, ..s[0] }])?,
        (Alu::Vec2, 16, 2) => ins(p, Op::MkvecV2i16, &s[..2])?,
        (Alu::Fabs, 32, 1) => ins(p, Op::FabsnegF32, &[Src { abs: true, ..s[0] }])?,
        (Alu::Fclamp(c), 32, 1) => {
            /* `bi_fclamp_to` then `I->clamp = ...` (bifrost_compile.c:2881-2897) */
            let t = p.temp()?;
            let mut i = Ins::new(Op::FclampF32, &[Dest::ssa(t)], &s[..1]);
            i.clamp = c;
            p.push(i)?;
            t
        }
        (Alu::Fclamp(c @ clamp::ZERO_ONE), 16, 1 | 2) => {
            let t = p.temp()?;
            let mut i = Ins::new(Op::FclampV2f16, &[Dest::ssa(t)], &s[..1]);
            i.clamp = c;
            p.push(i)?;
            t
        }
        (Alu::Frsq, 32, 1) => ins(p, Op::FrsqF32, &s[..1])?,
        (Alu::Ffloor, 32, 1) => ins(p, Op::FroundF32 { round: Round::Rtn }, &s[..1])?,
        (Alu::Fceil, 32, 1) => ins(p, Op::FroundF32 { round: Round::Rtp }, &s[..1])?,
        (Alu::Ftrunc, 32, 1) => ins(p, Op::FroundF32 { round: Round::Rtz }, &s[..1])?,
        /* `bi_fcmp_to(.., BI_RESULT_TYPE_M1)` (:3267-3272), already in the
         * `_OR` form va_lower_isel gives it, the third source zero */
        (Alu::Fcmp(cmpf), 32, 1) => ins(p, Op::FcmpOrF32 { cmpf, result: ResultType::M1 }, &[s[0], s[1], ZERO])?,
        /* `bi_icmp_to(nir_type_int, ..)`: an equality is `ICMP.i32`, hence
         * `ICMP_OR.u32` (bir.rs); an ordering is `ICMP.s32` */
        /* one lane: each source's half in both (the caller's swizzle) */
        (Alu::Fcmp(cmpf), 16, 1 | 2) => ins(p, Op::FcmpOrV2f16 { cmpf, result: ResultType::M1 }, &[s[0], s[1], ZERO])?,
        (Alu::Fceil, 16, 1) => ins(p, Op::FroundV2f16 { round: Round::Rtp }, &[lone(s[0])])?,
        (Alu::Ffloor, 16, 1) => ins(p, Op::FroundV2f16 { round: Round::Rtn }, &[lone(s[0])])?,
        (Alu::Fmin, 16, 1) => ins(p, Op::FminV2f16, &[lone(s[0]), lone(s[1])])?,
        (Alu::Frsq, 16, 1) => ins(p, Op::FrsqF16, &[lone(s[0])])?,
        (Alu::Frcp, 16, 1) => ins(p, Op::FrcpF16, &[lone(s[0])])?,
        (Alu::Icmp(cmpf @ (Cmpf::Eq | Cmpf::Ne)), 32, 1) => {
            ins(p, Op::IcmpOrU32 { cmpf, result: ResultType::M1 }, &[s[0], s[1], ZERO])?
        }
        (Alu::Icmp(cmpf), 32, 1) => ins(p, Op::IcmpOrS32 { cmpf, result: ResultType::M1 }, &[s[0], s[1], ZERO])?,
        /* `bi_mux_to(dst, s2, s1, s0, BI_MUX_INT_ZERO)` (:2967-2970): the
         * else value, the then value, the condition */
        (Alu::B2f, 32, 1) => ins(p, Op::MuxI32, &[ZERO, ONE, s[0]])?,
        (Alu::B2f, 16, 1) => ins(p, Op::MuxV2i16, &[ZERO, ONE_F16, s[0].swz(Swz::H00)])?,
        (Alu::Bcsel, 32, 1) => ins(p, Op::MuxI32, &[s[2], s[1], s[0]])?,
        /* f2i16 of a 32-bit source is the same instruction, its low half
         * the value (:3127-3135) */
        (Alu::F2i, 32 | 16, 1) => ins(p, Op::F32ToS32, &s[..1])?,
        (Alu::I2f, 32, 1) => ins(p, Op::S32ToF32, &s[..1])?,
        (Alu::Iadd, 32, 1) => ins(p, Op::IaddS32, &s[..2])?,
        (Alu::U2f32, 32, 1) => ins(p, Op::U16ToF32, &s[..1])?,
        /* one lane: each source's own half in both (`bi_alu_src_index`
         * replicates the lane read; the caller's swizzle) */
        (Alu::Fmul, 16, 1) => ins(p, Op::FmaV2f16, &[s[0], s[1], NEG_ZERO])?,
        (Alu::Ffma, 16, 1) => ins(p, Op::FmaV2f16, &s[..3])?,
        (Alu::F2f32, 32, 1) => ins(p, Op::F16ToF32, &s[..1])?,
        (Alu::U2u32, 32, 1) => ins(p, Op::U16ToU32, &[s[0].swz(Swz::H00)])?,
        /* `bi_lshift_or_to(b, sz, dst, s0, bi_zero(), bi_byte(s1, 0))` (:3047-3048);
         * the shift's byte lane is what va_lower_constants gives its
         * constant (fau.rs: the table's byte, moved to a register when the
         * instruction's two FAU words are taken, as the references draw) */
        (Alu::Ishl, 32, 1) => ins(p, Op::LshiftOrI32, &[s[0], ZERO, s[1]])?,
        /* `bi_cubeface1_to` / `bi_cubeface2_v9_to(.., x, y, z)`
         * (bifrost_compile.c:2118-2124), `bi_cube_ssel_to` and
         * `bi_cube_tsel_to` with the intrinsic's three sources in order
         * (:2128-2137) */
        (Alu::Cubeface1, 32, 1) => ins(p, Op::Cubeface1, &s[..3])?,
        (Alu::Cubeface2, 32, 1) => ins(p, Op::Cubeface2, &s[..3])?,
        (Alu::CubeSsel, 32, 1) => ins(p, Op::CubeSsel, &s[..3])?,
        (Alu::CubeTsel, 32, 1) => ins(p, Op::CubeTsel, &s[..3])?,
        /* `bi_mux_to(sz, dst, s1, s2, s0, BI_MUX_BIT)` (:3419-3426): the
         * `bitfield_select(mask, b, c)` as `MUX b, c, mask` */
        (Alu::BitfieldSelect, 32, 1) => ins(p, Op::MuxBitI32, &[s[1], s[2], s[0]])?,
        /* `bi_lshift_and_to` / `bi_lshift_or_to(b, sz, dst, s0, s1,
         * bi_imm_u8(0))` (bifrost_compile.c:3375-3381) */
        (Alu::Iand, 32, 1) => ins(p, Op::LshiftAndI32, &[s[0], s[1], ZERO])?,
        (Alu::Ior, 32, 1) => ins(p, Op::LshiftOrI32, &[s[0], s[1], ZERO])?,
        /* `bi_lshift_or_to(b, sz, dst, bi_zero(), bi_not(s0),
         * bi_imm_u8(0))` (bifrost_compile.c:3387-3388): the `.not` is
         * Valhall's `not2`, BIR's second source with `neg` set */
        (Alu::Inot, 32, 1) => ins(p, Op::LshiftOrI32, &[ZERO, Src { neg: true, ..s[0] }, ZERO])?,
        /* `bi_csel_to(.., s0, s1, s0, s1, BI_CMPF_GT / _LT)`
         * (bifrost_compile.c:3061-3071) */
        (Alu::Imax, 32, 1) => ins(p, Op::CselS32 { cmpf: Cmpf::Gt }, &[s[0], s[1], s[0], s[1]])?,
        (Alu::Imin, 32, 1) => ins(p, Op::CselS32 { cmpf: Cmpf::Lt }, &[s[0], s[1], s[0], s[1]])?,
        (Alu::Iabs, 32, 1) => ins(p, Op::IabsS32, &s[..1])?,
        /* `bi_rshift_or_to(b, sz, dst, s0, bi_zero(), bi_byte(s1, 0), false)`
         * (:3050-3052), the shift's byte lane as `Ishl`'s */
        (Alu::Ushr, 32, 1) => ins(p, Op::RshiftOrI32, &[s[0], ZERO, s[1]])?,
        (Alu::Imul, 32, 1) => ins(p, Op::ImulI32, &s[..2])?,
        /* `bi_s16_to_s32_to(b, dst, s0)` (:3205-3211), the mask's half the
         * caller's swizzle */
        (Alu::I2i32, 32, 1) => ins(p, Op::S16ToS32, &s[..1])?,
        /* `bi_mux_to(.., s2, s1, s0, BI_MUX_INT_ZERO)` as the 32-bit row,
         * each source's half in both (the caller's swizzle) */
        (Alu::Bcsel, 16, 1 | 2) => ins(p, Op::MuxV2i16, &[s[2], s[1], s[0]])?,
        (Alu::Ffloor, 16, 2) => ins(p, Op::FroundV2f16 { round: Round::Rtn }, &s[..1])?,
        (Alu::F2i32, 32, 1) => ins(p, Op::F16ToS32, &s[..1])?,
        /* `bi_arshift_or_to(b, sz, dst, s0, bi_zero(), bi_byte(s1, 0))`
         * (:3053-3055). A uniform's word takes no byte lane: the amount is
         * moved to a register first (intuni_ishr.expect.json: `MOV.i32 r0,
         * u2.w1`, then `ARSHIFT_OR.i32 r0, u2.w0, r0^.b00, 0x0`). A
         * constant's byte lane is va_lower_constants', as `Ishl`'s
         * (intbool_199_vsc: `ARSHIFT_OR.i32 r2, r2, 0x1000000.b33, 0x0`) */
        (Alu::Ishr, 32, 1) => {
            let amount = match s[1].idx {
                Idx::Ubo { .. } => Src::ssa(ins(p, Op::MovI32, &s[1..2])?).swz(Swz::B0),
                Idx::Const(_) => s[1],
                _ => s[1].swz(Swz::B0),
            };
            ins(p, Op::ArshiftOrI32, &[s[0], ZERO, amount])?
        }
        (Alu::U8f32(k), 32, 1) => {
            let b = [Swz::B0, Swz::B1, Swz::B2, Swz::B3][k as usize & 3];
            ins(p, Op::U8ToF32, &[s[0].swz(b)])?
        }
        /* `bi_lshift_xor_to(b, sz, dst, s0, s1, bi_imm_u8(0))` */
        (Alu::Ixor, 32, 1) => ins(p, Op::LshiftXorI32, &[s[0], s[1], ZERO])?,
        /* `bi_isub_to(.., s0, s1, false)`; `ineg` with `bi_zero()` first */
        (Alu::Isub, 32, 1) => ins(p, Op::IsubS32, &s[..2])?,
        (Alu::Ineg, 32, 1) => ins(p, Op::IsubS32, &[ZERO, s[0]])?,
        (Alu::U2f, 32, 1) => ins(p, Op::U32ToF32, &s[..1])?,
        (Alu::F2u, 32, 1) => ins(p, Op::F32ToU32, &s[..1])?,
        /* `bi_u16_to_u32_to(b, dst, bi_half(s0, half))` */
        (Alu::Extract16(k), 32, 1) => ins(p, Op::U16ToU32, &[s[0].swz(if k == 0 { Swz::H00 } else { Swz::H11 })])?,
        /* `bi_mux_to(b, sz, dst, bi_imm_u8(0), bi_imm_uintN(1, sz), s0,
         * BI_MUX_INT_ZERO)` */
        (Alu::B2i, 32, 1) => ins(p, Op::MuxI32, &[ZERO, Src::of(Idx::Const(1)), s[0]])?,
        (Alu::Ucmp(cmpf), 32, 1) => ins(p, Op::IcmpOrU32 { cmpf, result: ResultType::M1 }, &[s[0], s[1], ZERO])?,
        (Alu::Fabs, 16, 1) => ins(p, Op::FabsnegV2f16, &[Src { abs: true, ..lone(s[0]) }])?,
        /* `bi_icmp_to(nir_type_uint16, ..)`, an equality's `ICMP.v2u16`
         * made `ICMP_OR` by va_lower_isel (bir.rs) */
        (Alu::Icmp(cmpf @ (Cmpf::Eq | Cmpf::Ne)), 16, 1) => {
            ins(p, Op::IcmpOrV2u16 { cmpf, result: ResultType::M1 }, &[s[0].swz(Swz::H00), s[1], ZERO])?
        }
        /* `bi_make_vec_to` of one 16-bit component (bifrost_compile.c:
         * 836-848): the lane's half (the caller's swizzle) and `bi_imm_u16(0)`,
         * the zero's low half */
        (Alu::Mov, 16, 1) => ins(p, Op::MkvecV2i16, &[s[0], ZERO.swz(Swz::H00)])?,
        /* `bi_lshift_or_to` / `bi_lshift_and_to(b, 16, dst, s0, s1,
         * bi_imm_u8(0))` of one lane, then `bi_lower_swizzle`
         * (bi_lower_swizzle.c `lower_swizzle`): the second source takes no
         * swizzle, so its low half is read whole (the result is a scalar:
         * its high half is not read) and its high half is moved to both
         * first (`IADD.v2u16 x.h11, 0`, CSE'd) */
        (op @ (Alu::Ior | Alu::Iand), 16, 1) => {
            let b = match lone(s[1]) {
                x if x.swz == Swz::H11 => Src::ssa(ins(p, Op::IaddV2u16, &[x, ZERO])?),
                x => x.swz(Swz::H01),
            };
            let o = if op == Alu::Ior { Op::LshiftOrV2i16 } else { Op::LshiftAndV2i16 };
            ins(p, o, &[lone(s[0]), b, ZERO])?
        }
        /* `bi_emit_derivative` of a 16-bit lane, the fine form: the operand
         * `a` one whole register word (the caller widens a half of a pair,
         * `MKVEC.v2i16 a.h0, #0` as `U16_TO_U32`; a lane in its own register
         * is read as it is), the neighbour `CLPER.i32.subgroup4.xor a, axis`
         * (axis 1 for X, 2 for Y; the 8-bit lane source is the table's
         * byte, fau.rs `lut8`), `FADD.v2f16 n, a.neg` (the same in both
         * halves), then the sign fix: lane `L` computed `a(L ^ axis) -
         * a(L)`, so the lanes with the axis bit set are XORed with the sign
         * bit, `LSHIFT_XOR.v2i16 m, #(16 - ffs(axis)), d` with `m` the lane
         * id (X: every bit but bit 0 shifts out) or `lane_id & 2` (Y, one
         * `LSHIFT_AND.i32` for every Y derivative, CSE'd; the 2 a pushed FAU
         * word moved to a register beside the special, fau.rs `repair_fau`).
         * The fix is always emitted: the form Mesa makes when every use
         * ignores the sign bit (`nir_def_all_uses_ignore_sign_bit`) has no
         * reference, so an `abs` of a derivative is refused at the walk
         * (nir_fs.rs `Fs::abs`) */
        (op @ (Alu::Fddx | Alu::Fddy), 16, 1) => {
            let lane_id = Src::of(Idx::Special(super::bir::Special::LaneId, false));
            let a = s[0].swz(Swz::H01);
            let axis = if op == Alu::Fddx { 1 } else { 2 };
            let n = ins(p, Op::Clper, &[a, Src::of(Idx::Const(axis))])?;
            let d = ins(p, Op::FaddV2f16, &[Src::ssa(n), Src { neg: !a.neg, ..a }])?;
            let (m, shift) = if op == Alu::Fddx {
                (lane_id, 15)
            } else {
                (Src::ssa(ins(p, Op::LshiftAndI32, &[lane_id, Src::of(Idx::Const(2)), ZERO])?), 14)
            };
            ins(p, Op::LshiftXorV2i16, &[m.swz(Swz::H00), Src::ssa(d), Src::of(Idx::Const(shift))])?
        }
        _ => return Err(MaliError::Defect(super::Defect::IrIndex)),
    })
}
