<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): lowering the dEQP arithmetic built-in refusals to Mesa's words (design)

**Status: design, no code.** Plan #387 item `arith-design`; the implementation items are
`arith-lower-1` (step 1 below) and the later ones this note outlines. It reads the 144
Mesa 26.2.1 references `tools/mali/testdata/deqp_arith_*` and the compiler
(`src/shaderc/online/mali/`, read only) at `3497799a`, and changes neither. Every case
count is the host measurement of [`mali-deqp-arith.md`](mali-deqp-arith.md) (1,627 cases
whose first refusal is `MaliError::Arithmetic`, `FsOperandForm` or `Rebalance`; its
section 2.1 lists every shape with its precision split and its reference; section 18 of
`mali-deqp-families.md` has the ranking they come from). Where this note says "first
refusal" the same caveat holds: a count is the cases a step *moves*, an upper bound of those
it finishes (a random program finishes only when every op in it is lowered).

Line numbers are those of `3497799a`. Plan 377's `lower-constop` is changing the same files,
so an implementer finds each site by the **arm** named here (a `ir::Op::Alu(..)` pattern,
a function) and not by the number.

The sequences of section 1 are the `text` of each reference's `vs.varying` (vertex shapes)
or `fs.fragment` (fragment shapes) region, with the loads, the `STORE`s and the
`ATEST`/`BLEND` tail left out; a colour's `*0.5+0.5` envelope is described once, below.
Related notes: [`mali-alu-ops.md`](mali-alu-ops.md), [`mali-vs-alu-ops.md`](mali-vs-alu-ops.md),
[`mali-fs-arith.md`](mali-fs-arith.md), [`mali-transcendentals.md`](mali-transcendentals.md),
[`mali-algebraic-folds.md`](mali-algebraic-folds.md).

## 1. What Mesa emits, per shape

**The envelope.** dEQP's operator shaders write `color = clamp(op(x) * scale + bias, 0, 1)`
style colours: a result is narrowed with `FADD.f32 r.h0, r, 0x0.neg` (or `V2F32_TO_V2F16`
for two lanes), widened with `U16_TO_U32 r, r.h0`, and scaled by a `FMA.v2f16 r, r, scale,
bias` (`0x38000000.h11` = 0.5, `0x34000000.h11` = 0.25, `u2.w0` a FAU word). The vertex
shapes carry the same tail followed by `F16_TO_F32` x2 and `IADD_IMM.i32` of the two
constant colour lanes. Below, only the op's own instructions are listed. A *fp32 kernel*
is an op Mesa has only at 32 bits (`bi_lower_bit_size`): a mediump operand is `F16_TO_F32`d
or, when it is a varying, simply read from its fp32 load (the `f2f32(f2fmp(a))` pair
cancels at the link), and the result is narrowed.

| Shape | Mesa's sequence (highp ops fp32, mediump fp16 pairs unless stated) | Refs |
|---|---|---|
| `a / b`, `a /= b` | hi: `FRCP.f32 b`, `FMA.f32 a, rcp, 0x0.neg`. mid: `FADD.f32 .h0` narrow both, `FRCP.f16`, `FMA.v2f16 a, rcp, 0x0.neg` | `op_binary_div{,_assign}_fs_{highp,mediump}` |
| `mat / float` | one `FRCP` of the scalar, one `FMA` a lane (hi `.f32` then `FADD.f32 .h0` narrow and `MKVEC.v2i16`; mid `FRCP.f16` + `FMA.v2f16`, the negated lane a `FADD.v2f16 rcp.neg, 0x0.neg`) | `matrix_div_fs_*` |
| `mod(x, y)` | `FRCP y`, `FMA x, rcp, -0` (x/y), `FROUND.<f32 / v2f16>.rtn`, `FMA -y, floor, x`: 4 ops at either width | `op_common_mod_fs_*` |
| `smoothstep(e0, e1, x)` | `FADD x - e0`, `FADD e1 - e0` (mid; hi reads the uniform words), `FRCP`, `FMA.<f32/v2f16>.clamp_0_1 t = a * rcp`, `FMA -2.0 * t + 3.0`, `FMA t * that`, `FMA t * that` | `op_common_smoothstep_fs_*` |
| `sin`, `cos` (fs) | at both precisions the fp32 kernel: `FMA.f32 x, 2/pi, bias`, `FADD_IMM.f32 #0xC9400000`, `FMA.f32` (e), `FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32 e,e,-0,0xFFFFFFFF`, `FMA.f32 .neg`, `FMA.f32.clamp_m1_1`, `FADD.f32` (cos: operands of the last two swap) | `op_angle_{sin,cos}_fs_*` |
| `tan` (fs) | the same kernel once for sin, a second `FMA.f32` + `.clamp_m1_1` + `FADD.f32` for cos; **hi**: `FRCP.f32 cos`, `FMA.f32 sin, rcp`; **mid**: both results narrowed (`FADD.f32 .h0`), `FRCP.f16`, `FMA.v2f16` | `op_angle_tan_fs_*` |
| `exp2` | `FMA_RSCALE.f32 x, 1.0, -0, r3` (r3 the log base register: `bi_fexp_32`), `F32_TO_S32`, `FEXP.f32` a lane; the three lanes' narrowing is `FADD.f32 .h0` + `MKVEC.v2i16` (hi) or `V2F32_TO_V2F16` (mid) | `op_exponential_exp2_*` |
| `log2` | `FREXPE.f32.log`, `S32_TO_F32`, `FADD_LSCALE.f32 -1.0`, `FLOGD.f32`, `FMA.f32` | `op_exponential_log2_*` |
| `log` | `log2`'s five, then hi: `FMA.f32 .., 0x3F317218` (ln 2, fp32); **mid**: narrow, then `FMA.v2f16 .., u.w0` (ln 2 as fp16) | `op_exponential_log_*` |
| `pow(x, y)` | `log2`'s five of x, `FMA_RSCALE.f32 y, log, -0, u.w0`, `F32_TO_S32`, `FEXP.f32` | `op_exponential_pow_*` |
| `exp` (vs) | hi: `FMA.f32 x, log2e`, `FMA_RSCALE`, `F32_TO_S32`, `FEXP`; **mid**: `FADD.f32 .h0` narrow, `FMA.v2f16 x, log2e`, `F16_TO_F32`, then the same | `op_exponential_exp_vs_*` |
| `floor`, `ceil` | one `FROUND.f32.rtn` / `.rtp`; mid `FROUND.v2f16.rtn/.rtp` of the narrowed operand | `op_common_{floor,ceil}_*` |
| `sign` | `FCMP_OR.<f32/v2f16>.lt.f1 0, x` and `.lt.f1 x, 0`, `FADD a, b.neg` (the `lower_fsign` pair, [`mali-algebraic-folds.md`](mali-algebraic-folds.md)) | `op_common_sign_*` |
| `min`, `max`, `clamp` | hi `FMIN/FMAX.f32` (clamp: `FMAX` then `FMIN`, no `.clamp_0_1`, the bounds are variables); mid the same at `.v2f16` after narrowing each operand (`FADD.f32 r.h0`) | `op_common_{min,max,clamp}_*` |
| `a != b` (float, fs) | `FCMP_OR.f32.ne.m1`, `MUX.v2i16 0, 1.0, mask` (hi and mid alike: the operands are varyings read whole) | `op_binary_not_equal_fs_*`, `op_float_notequal_fs_*` |
| `c ? a : b` | mid `FCMP_OR.v2f16.lt.m1 0, c`, `MUX.v2i16 b, a, mask`; hi `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32`, `CSEL.u32.eq` (the arms fp32) | `op_selection_select_fs_*`, `other_bcsel_*` |
| `dynamic_select` (indexing) | 40 `IADD_IMM.i32 #const` stores of the array into thread-local storage, then per read `LSHIFT_OR.i32 idx, #shift`, `IADD.u32 thread_local_pointer`, `LOAD.i16.force.unsigned`, then `FADD.v2f16` of the four loads | `indexing_dynamic_select_fs_none` |
| `sqrt`, `inversesqrt` | `FRSQ.f32` (+ `FRCP.f32` for sqrt) at hi; `FRSQ.f16` (+ `FRCP.f16`) at mid, lane pair `.h00` | `random_{fsqrt,frsq}_fs_mediump` (generated) |
| `acos`, `asin` | fp32 at **both** precisions: 3 `FMA.f32` Horner terms of `x.abs`, `FADD 1 - |x|`, `FRSQ.f32` + `FRCP.f32` (sqrt), `FMA .neg`, two `FCMP_OR.f32.lt.f1` (sign), `FADD`, `FMA`; `asin` adds `FMA` x3, `FCMP_OR.f32.lt.m1 |x| < c`, `CSEL.u32.eq` | `op_angle_{acos,asin}_fs_*` |
| `atan`, `atan2` | hi: `FRCP.f32`, `FCMP_OR.f32.ge.m1 1.0 >= |x|`, `CSEL`, 5 `FMA.f32` (even polynomial in x^2), `CSEL`, `FMA`, 2 `LSHIFT_AND.i32` + `LSHIFT_OR.i32` (copysign), `FADD`; mid the same at `.v2f16` with `MUX.v2i16`, `LSHIFT_*.v2i16`; `atan2` adds the quadrant fix (`FCMP_OR.ge/eq.m1`, `CSEL`, `FMIN`, second `FRCP`) | `op_angle_{atan,atan2}_fs_*` |
| `faceforward`, `refract` | `faceforward`: `FMA.f32` (dot), `FADD x.neg`, `FCMP_OR.f32.lt.m1`, `CSEL`; `refract`: 4 `FMA`, `FRSQ`+`FRCP`, 3 `FMA`, `FCMP_OR.f32.lt.m1`, `CSEL .., 0x0`; mid the same in `.v2f16`/`FRSQ.f16`/`FRCP.f16` | `op_geometric_{faceforward,refract}_*` |
| `+= -= *= ++ -- -x abs mix reflect cross` of varyings (mid), matrix `+` | `V2F32_TO_V2F16` / `FADD.f32 .h0` narrowing of the varying words, `FADD.v2f16`, `FMA.v2f16` (`*` is `FMA a, b, 0x0.neg`), `MKVEC.v2i16` re-pairing, source modifiers `.neg` `.abs` `.h00/.h11/.h10` | `op_binary_{add,sub,mul}*`, `op_unary_*`, `op_common_{abs,mix}_*`, `op_geometric_{reflect,cross}_*`, `matrix_storeout_*`, `*_storeoutput_*` |
| `==`, `!=`, `any`, `all` (vs) | ivec form (`highp`/`mediump` shapes): `F32_TO_S32.rtz` per lane, `ICMP_OR.u32.eq.m1`/`ICMP_AND`/`ICMP_OR.u32.ne.m1`, `CSEL.u32.eq`; vec form (`none`): mediump attributes narrowed (`FADD.f32 .h0`, `V2F32_TO_V2F16`), `FCMP_OR.v2f16.lt.m1`, `ICMP_OR.v2u16.eq/ne.m1`, `LSHIFT_AND/OR.v2i16`, `S16_TO_S32`, `CSEL`; `any`/`all` of a compare are `FMAX`/`FMIN.v2f16` of the lanes, then one `FCMP_OR.v2f16.lt.m1` | `op_binary_{equal,not_equal}_vs_*`, `op_bool_{any,all}_vs_none`, `other_{bany,ball}_vs_none` |
| a constant expression | no op: `MUX.v2i16` / `IADD_IMM` of folded constants (`constexpr_*`) | `constexpr_*` |

**Highp and mediump of one shape.** For `sin`, `cos`, `exp2`, `log2`, `pow` the **vertex** hi and mid references are
byte-equal (`deqp_arith_*_vs_mediump` "same as" `_highp`: the vertex stage's fp32 kernels
do not care); the **fragment** ones differ only in registers. Where Mesa differs by precision
the table says so (`tan`, `log`, `exp`, `ceil`, `sign`, `min`, `max`, `clamp` in the vertex
stage; `tan`, `log` in the fragment stage). `lowp` is `mediump` byte for byte (checked in
`mali-deqp-arith.md` section 2).

## 2. The compiler's refusal sites and the missing piece

`mod.rs` raises `MaliError::Arithmetic` from these arms (all `Err(MaliError::Arithmetic)`).

**Fragment walk** (`nir_fs.rs`, `walk_nodes`, the mediump arm; the fp32 domain is `Fs::alu32`):

| Arm | What it accepts today | Missing for the shapes |
|---|---|---|
| `ir::Op::Alu(AluOp::Frcp, s)` (~6098) | `1.0 / constant` only (folded, fp16-exact) | a lane operand: `Alu::Frcp` node of `fp16_operand(lane)` (emit has `FRCP.f16` and `.f32` rows); the quotient is the front end's `fmul(a, frcp(b))`, whose `Fmul` arm already makes the `FMA ... 0x0.neg` |
| `Fsin \| Fcos \| Flog2` (~6247) | a uniform's dword, or a computed fp16 lane for `log2` only | a **varying's lane** (`f.input_lane`) and a computed lane for `sin`/`cos`; the `exp2` arm (~6276) already accepts a varying lane and shows the pattern |
| `Fexp2` (~6276) | varying lane, uniform dword, fp16 lane, `pow_of` | already there; `exp2` fp32 shapes refuse at the *pair tree* instead (OPERAND, section 3 step 4) |
| `Fmax \| Fmin` (~6122) | two fp16 lanes, or a lane and a constant | a lane and a varying/uniform operand not already fp16 (narrow it: `fp16_operand`); `clamp` is two such nodes |
| `Bcsel` (~6140, the `match (c, y, z)`) | constant arms only | computed arms: `Alu::Bcsel` node over a `Fcmp` mask (`mask16` exists) |
| `Fceil` (~6016, before the Fddx arm) | computed lanes only (`f.alu_of(x).is_none()` refuses a varying) | a varying lane: narrow with `fp16_operand`, then `Alu::Fceil` |
| `Frsq \| Fsqrt` (~6336) | computed lane or a varying (`input_lane`) | the refusals left are random programs; see step 3 |
| catch-all `ir::Op::Alu(..) => Arithmetic` (~6376) | | `Fsign`, `Ffloor` of a varying lane, `Fne` (they reach `Fs::alu32`/`node16` only for the colour's own lanes) |
| `Fs::alu32` (3580; arms 3340-3400 `Frcp Fsqrt Frsq Ffloor Fceil Fsign Fmin Fmax Fcmp`) | fp32 forms of `rcp`, `sqrt`, `floor`, `ceil`, `sign`, `min`, `max`, compares exist | **no fp32 `Fsin`/`Fcos`/`Flog2`** and an fp32 `Fexp2` only of a constant exponent through `pow32_of`: the transcendental kernels are only in `transc_rows` / `exp2_rows` (~13318-13372), which wrap the kernel in `F2f32` ... `F2f16` |

The `FsOperandForm` texts (OPERAND) are raised by the pair-tree builder (`nir_fs.rs` ~8472, 8599,
8614, 8674, 9086-9126, 9430, 9457, 9621-9668, 9892, 9954, 12153): it admits a closed set of
products (`ADMITTED`) and refuses a lane alone first in a sum or a uniform first (the text).

**Vertex walk** (`nir.rs`, `walk`): `IrOp::Alu(a, s)` (~4619) makes the fp32 `Alu` nodes of the arms it lists
(`Frcp`, `Fmax|Fmin` through `vs_clamp`, `Fabs`, `Ffloor`, `Ffract`, `Frsq`, `Fsqrt`, `Fcmp`, `Bcsel`,
integer forms) and ends `_ => return Err(MaliError::Arithmetic)` (~5098) for `Fsin Fcos Flog2 Fexp2 Fpow Fceil
Fsign` and `exp`/`log`/`tan` (front-end compositions of these). `vs_clamp` (~3570) refuses two computed
operands and a clamp to `[b, 1]`; `vs_reduce` (~3686-3711) reads only the `vsarith_86_anyall` forms.
`Alu` already names `Fsin Fcos Flog2 Fexp2 Fpow` (`arity`, ~290-320) and the constant folder
refuses them (`ConstantFold`, ~2247), so the enum is ready; what is missing is their **emit
rows** for a 32-bit node (`emit.rs` has rows for `Fpow` at 32 and `flog2`/`fexp2` helpers, ~222-254, 327;
the sin/cos kernel lives only in `nir_fs.rs::transc_rows`).

**Mixed precision.** A vertex `mediump` op reads its attributes at fp16 in Mesa
(`FADD.f32 r.h0` / `V2F32_TO_V2F16`, the references of the `mid` column of section 1) and the vertex walk is
fp32: shapes marked "differs by precision" cannot be byte-equal until the vertex walk has an fp16 attribute
read (the `mixprec` family's vertex side, issue-level work of `intval-vs16` / `vs16-design`, `mali-deqp-mixprec.md`).

## 3. Mechanisms ranked by blocked cases, and the steps

Rank = first-refusal cases a mechanism moves, net of the 325 that belong to the integer/boolean family
(`isok_int_round` 146, `uniform_api.bool_compare` 179) and the 75 constant folds. Operator-suite
cases ("Op") are the likely floor of the cases that finish; the random programs finish only when every op
of the program is in a landed step. Step numbers are this note's and replace section 4's of
`mali-deqp-arith.md` (its shape table is the source of the counts; the mapping is exact: the rows below sum
to 1,627).

| Step | Mechanism | Moves | Op | Refs | Code |
|---|---|---:|---:|---:|---|
| **1a** | FS `a / b`: `Frcp` of a lane + `mod`, `smoothstep`, matrix `/` | 179 | 107 (+54 matrix) | 12 | `nir_fs.rs` `Frcp` arm |
| **1b** | FS fp32 transcendental kernels of a varying/computed lane (`sin cos tan log log2 pow`; `exp2` random) | 187 | 60 | 17 | `nir_fs.rs` `Fsin\|Fcos\|Flog2` arm, `transc_rows` |
| **2** | VS fp32 built-ins: `sin cos tan(hi) exp2 log2 pow`, `exp log ceil sign min max clamp` (hi), the random `fsin fcos fexp2 flog2` | 189 | 89 | 21 | `nir.rs` `walk` op arm, `vs_clamp`, new 32-bit emit rows |
| **3** | FS rounding and compare: `floor ceil sign`, float `!=`, `sqrt`/`inversesqrt` random | 141 | 57 | 14 | `nir_fs.rs` catch-all, `Fceil` arm, `Fs::alu32` |
| **4** | FS pair trees (OPERAND) and `min max clamp` of two lanes | 216 | 132 (+29 matrix) | 26 | pair builder, `Fmax\|Fmin` arm |
| **5** | FS `bcsel` of computed arms and the dynamic-index select | 96 | 28 (+60 indexing) | 6 | `Bcsel` arm; TLS array (`mali_locals`) |
| **6** | VS fp16 forms of mediump ops (`exp log tan ceil sign min max clamp`, vector `== !=`, `any`/`all`) | 159 | 122 | 20 | vertex fp16 read, `vs_reduce`; shared with `vs16-design` |
| **7** | composites: `acos asin atan atan2 faceforward refract` | 48 | 48 | 12 | needs 1a, 3, 4 and `FSIGN`-compose |
| **8** | `do_rebalance_tree` chains | 12 | 0 | 1 | `glsl.rs` `Rebalance` |
| X1 | constant folds (`constexpr.*`, `other.ball` fs) | 75 | 0 | 13 | `lower-constop` (plan 377), not this family |
| X2 | integer/boolean (`isok_int_round`, bool uniform compare) | 325 | 0 | 2 | `intval-*` items |
| | **Total** | **1,627** | | **144** | |

Order: 1a, 1b, 2 (they share the fp32 kernel factored in 1b), then 4, 3, 5, 6, 7, 8. 6 waits for the
vertex fp16 read of `intval-vs16`/`vs16-design`; 7 is last because its shapes use the ops of 1a, 3 and 4.

### 3.1 Every shape to its step

Counts are the first-refusal cases of the shape in the stage named, precision split where the step changes with it
(`highp` and `none` of a vertex shape are fp32). This lists every shape of section 2.1 of
`mali-deqp-arith.md`.

- **1a (179)**: `matrix.div` fs 54; `op.binary.div_assign` fs 42; `op.binary.div` fs 30; `op.common.smoothstep` fs 21; `random.frcp` fs 15; `op.common.mod` fs 14; `other.frcp` fs 3.
- **1b (187)**: `random.flog2` fs 41; `random.fsin` fs 35; `random.fexp2` fs 33; `random.fcos` fs 13; `op.angle.{cos,sin,tan}` fs 12 each; `op.exponential.{log,log2,pow}` fs 8 each; `other.fsin` fs 5.
- **2 (189)**: `random.fexp2` vs 46; `random.flog2` vs 24; `random.fsin` vs 22; `random.fcos` vs 7; `op.angle.cos` vs 12; `op.angle.sin` vs 12; `op.angle.tan` vs 4 (highp); `op.exponential.exp2` vs 8; `op.exponential.log2` vs 8; `op.exponential.pow` vs 8; `op.exponential.exp` vs 4 (highp); `op.exponential.log` vs 4 (highp); `op.common.{ceil,sign}` vs 4 each (highp); `op.common.{min,max,clamp}` vs 7 each (highp); `other.fmin` vs 1.
- **3 (141)**: `random.fsqrt` fs 34; `random.fne` fs 19; `other.ffloor` fs 16; `op.binary.not_equal` fs 12; `op.common.{ceil,floor,sign}` fs 12 each; `random.frsq` fs 10; `op.float.notEqual` fs 9; `random.fsign` fs 3; `other.fne` fs 2.
- **4 (216)**: `random.storeoutput` fs 31; `matrix.storeout` fs 29; `op.common.{clamp,max,min}` fs 21 each; `other.storeoutput` fs 19; `op.unary.incdec` fs 12; `op.binary.{add,add_assign,sub,sub_assign}` fs 8 each; `op.binary.mul` fs 6; `op.binary.mul_assign` fs 4; `op.common.mix` fs 4; `op.exponential.exp2` fs 4; `random.storelocal` fs 3; `op.common.abs` fs 2; `op.geometric.reflect` fs 2; `op.unary.unary_minus` fs 2; `op.geometric.cross` fs 1; `other.fmin` fs 1; `random.imul` fs 1.
- **5 (96)**: `indexing.dynamic_select` fs 60; `op.selection.select` fs 28; `other.bcsel` fs 8.
- **6 (159)**: `random.bany` vs 22; `op.binary.equal` vs 21; `op.binary.not_equal` vs 21; `op.common.{clamp,max,min}` vs 14 each (mediump); `op.angle.tan` vs 8 (mediump); `op.common.{ceil,sign}` vs 8 each (mediump); `op.exponential.{exp,log}` vs 4 each (mediump); `other.fmax` vs 5; `other.ball` vs 4; `other.bany` vs 4; `op.bool.{all,any}` vs 3 each; `random.ball` vs 2.
- **7 (48)**: `op.angle.{acos,asin,atan,atan2}` fs 8 each; `op.geometric.{faceforward,refract}` fs 8 each.
- **8 (12)**: `other.x` vs 4; `random.chain` vs 4; `random.chain` fs 3; `other.x` fs 1.
- **X1 (75)**: `constexpr.flog2` vs 12; `constexpr.{fsin,fexp2}` fs and vs 9 and 8 each; `constexpr.fceil` vs 4; `constexpr.fcos` fs 4, vs 4; `constexpr.fsign` vs 4; `constexpr.{ball,bany}` fs 3, vs 3 each; `other.ball` fs 1.
- **X2 (325)**: `uniform_api.bool_compare` vs 179; `isok_int_round` fs 146.

## 4. Step 1 (`arith-lower-1`): the design

**Scope: 1a then 1b** (366 first-refusal cases, 29 references, 167 operator-suite cases plus the 54 `matrix.div`): the two share the
fp16/fp32 operand rule of the mediump walk (`fp16_operand`, the "narrowed varying" sources), both are
arm-widenings rather than new builders, and 1b's factoring is what step 2 reuses. A two-hour session lands them as two commits (1a first);
if the session ends after 1a the item is still useful. Everything outside 1a and 1b stays ignored.

### 4.1 Step 1a: division

Mesa: section 1 rows `a / b`, `mat / float`, `mod`, `smoothstep`. Words are mostly already expressible:
`FRCP.f16`/`.f32` rows exist (`emit.rs`), `Fmul`'s `FMA ... 0x0.neg` is made by the pair builder, `Ffloor`
has fp16 node forms (`node16`, `nir_fs.rs` ~2918, 2996) and `fsat` folds into the producer FMA (the
`.clamp_0_1` of the `FMA` rows of the smoothstep reference).

Changes:

1. `walk_nodes`, `ir::Op::Alu(AluOp::Frcp, s)`: keep the constant-folding branch, add: a lane that is not a constant becomes
   `Alu::Frcp` of `f.fp16_operand(lane)` (`Val::Undef` is `ColourForm` as the neighbouring arms do), `f.node(Alu::Frcp, ..)`. The
   hi references (`*_fs_highp`) carry an fp32 operand: their `fp16_operand` is "the shader is mixed"; the
   expected form is `Fs::alu32`'s existing `AluOp::Frcp => self.op32(Alu::Frcp ..)`, so the first thing to
   establish (print the refusal site of each of the 12 references before editing, the way
   `mali-deqp-arith.md` section 1 recorded it) is which arm the highp ones reach and route them to `alu32`.
2. `mod`: the front end builds `x - y * floor(x / y)`; Mesa's program is `FRCP`, `FMA x * rcp`, `FROUND.rtn`, `FMA -y * f + x`: the
   second `Fmul`+`Fadd` fuse into a `Ffma` with `.neg` on `y`. Check that the fuse rule in `Fs::op`
   (`Rule::None` vs the fuse rules `mali-algebraic-folds.md` lists) does it; if the fold is missing,
   add it where `Fadd(a, fneg(fmul(b, c)))` is made. Expect word-level mismatches in the register choice (`FMA r0, r1^.neg, r2^, r0^`:
   `neg` on the first source).
3. `smoothstep`: fp16 case `(x - e0) * rcp(e1 - e0)` with `.clamp_0_1` on the first FMA: the `fsat` fold
   into a `Ffma` producer (exists for `FMA.v2f16.clamp_0_1`, fsarith_4); hi reads `u2.w0` uniform words
   (`e0` and `3.0` constants in FAU).
4. `matrix / float`: one `Frcp` of the scalar uniform, `Ffma` per lane; the lane-0 negation in the hi reference
   (`FADD.f32 r0.h0, r0^.neg`) is the `-1.0` column of the identity-like matrix; check `fneg` does not become a
   separate node.
5. `other.frcp`, `random.frcp` (mediump): `1.0 / x` of a varying lane: `Frcp` arm, no multiply
   (`other_frcp_fs_none`: `FRCP.f16`, `FADD.v2f16 .. 0x3C000000.neg.h11`).

Un-ignore (`tests/mali_deqp_arith.rs`, move each from `REFUSED` to `LOWERED` and un-`#[ignore]`):
`deqp_arith_matrix_div_fs_{highp,mediump}`, `op_binary_div_assign_fs_*`, `op_binary_div_fs_*`, `op_common_mod_fs_*`,
`op_common_smoothstep_fs_*`, `other_frcp_fs_none`, `random_frcp_fs_mediump`. (`refusals_pinned` forces both.)
Unblocks: `shaders.operator.binary_operator.{div,div_assign}`, `shaders.matrix.div*`, `shaders.operator.common_functions.{mod,smoothstep}`,
`shaders.random.*` with `frcp`. Host forecast: `deqp-host-measure.sh --filter` on `shaders.operator`, `shaders.matrix`: the 179
cases move to their next refusal; expect most operator-suite cases to compile (107 + 54 matrix), the random ones depend on the other ops.

### 4.2 Step 1b: fp32 transcendentals of a lane

Mesa: rows `sin cos tan log2 log pow exp2` (fs). The kernel is fp32 at both precisions; the fp16 detail is only the
envelope (a varying's lane is read as the fp32 load; a computed fp16 lane is `F16_TO_F32`d; the result narrowed).

Changes:

1. `Fsin|Fcos|Flog2` arm: accept `f.input_lane(x).is_some_and(|(loc, _)| (loc as usize) < MAX_VARYINGS)` (as the `Fexp2` arm does) and a
   computed fp16 lane; keep the uniform dword form (case 77). The node reads the varying's fp32 word, so `transc_rows` needs a
   variant that **skips the `F2f32` row** for a varying operand (the mediump references start `FMA.f32 r0, r2, u2.w0, u2.w1` on the varying's
   register, no `F16_TO_F32`) and still ends with the `F2f16` row.
2. Factor `transc_rows` into a pure 32-bit kernel `transc32(p, a, x32, k) -> Src` (`Fsin/Fcos` sequence, `exp2_rows`,
   `flog2`, `Fpow`) and the existing envelope (`F2f32` .. `F2f16`). Step 2 (vertex) and the highp fragment reuse `transc32`. This is the
   refactor with the largest blast radius of step 1: `fsarith_77_transc` and `mali_fs_arith.rs`'s transcendental cases must stay byte-equal.
3. `tan`: `sin / cos` of one kernel run (Mesa's CSE shares the table lookups): `Alu::Fsin` and `Alu::Fcos` nodes of the same operand
   must **share** `FMA/FADD/FMA(e)/FSIN_TABLE/FCOS_TABLE/FMA_RSCALE`; the references show the shared prefix once, then `FMA .neg` / `.clamp_m1_1`
   / `FADD` for each result. Hi continues `FRCP.f32`, `FMA.f32`; mid narrows both results (`FADD.f32 r4.h0` and `r0.h0`), `FRCP.f16`,
   `FMA.v2f16`. Do `tan` last in this step: if the shared-kernel form is not reached in the session, leave `op_angle_tan_fs_*` (2 refs, 24
   cases incl. vs) ignored.
4. `log` (fs): `log2` then a multiply by ln 2: hi `FMA.f32 .., 0x3F317218, 0x0.neg` before narrowing; mid `FADD.f32 r.h0` narrowing **first** then
   `FMA.v2f16 .., u.w0, 0x0.neg` (the references differ here). `pow(x, y)`: `log2` rows, `FMA_RSCALE.f32 y, l, -0, u.w0`, `exp2_rows`
   (exists: `Fpow` row at `emit.rs` 222 and the `pow_of` rule of the `Fexp2` arm; a `pow` whose exponent is a variable
   reaches it through `pow32_of`, make sure the `AluOp::Fexp2` arm of `Fs::alu32` (~3542) is used for the highp reference).
5. `exp2` of a varying lane (`random_fexp2_fs_mediump`): the arm exists; the refusal in the random programs is the pair-tree
   (`nir_fs.rs` ~9457, `FsOperandForm` for `Fexp2` of an `input_lane`), a step-4 item; those cases move only when step 4 lands.

Un-ignore: `deqp_arith_op_angle_{sin,cos}_fs_*`, `op_exponential_{log,log2,pow}_fs_*`, `other_fsin_fs_none`,
`random_{fsin,fcos,flog2}_fs_mediump` (and `op_angle_tan_fs_*` if reached). `random_fexp2_fs_mediump` stays ignored with its
`OPERAND` text (step 4). Unblocks: `shaders.operator.angle_and_trigonometry.{sin,cos,tan}`, `exponential.{log,log2,pow}`, `shaders.random.*`
(`fsin`, `flog2`, `fcos`), `flush_finish.*` (5 `other.fsin`).

### 4.3 Pinned tests most at risk in step 1

- `tests/mali_fs_arith.rs` (the `fsarith_*` words, 77 `transc`, 218 `shadowcmp`): the `transc32`
  refactor and any change to the `Frcp` arm; `fs_transc_tests.rs` (in-tree) pins `transc_rows` forms.
- `tests/mali_glmark2_corpus.rs`: the glmark2 shaders divide by varyings and use `smoothstep`/`mod`
  (`gm2_fsmisc_4`: `smoothstep(0.25, 0.75, x)` folds to `(x - 0.25) * 2.0`, the constant branch of the `Frcp`
  arm: it must not change), and `sqrt`/`pow`.
- `tests/mali_vs_arith.rs`: untouched by 1a/1b unless `transc32` is shared into the vertex emit (step 2).
- `tests/mali_deqp_mixprec.rs`, `mali_deqp_fclane.rs`, `mali_deqp_intval.rs`, `mali_deqp_misc.rs`: ignored/LOWERED lists
  that move when an arm they pin starts to compile: re-run them; a test that began to compile with other words
  moves to `DIFFERS`, never silently.
- the unit tests of `nir_fs.rs`/`emit.rs`/`tests.rs` that pin refusal texts (`MaliError::Arithmetic` of a `/`): grep
  `Arithmetic` in `src/shaderc/online/mali/tests.rs` and update the ones step 1 turns into a compile.
- Plan 377 (`lower-constop`) touches the same arms (`Frcp`, `Fsin`: the constant folds): merge after it; the
  `Frcp` arm's constant branch is its.

Commands: `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_arith`
(the file's header); also `--test mali_fs_arith --test mali_vs_arith --test mali_glmark2_corpus --test mali_deqp_mixprec`.
`cargo fmt --check` on the files touched; do not run `tools/check.sh`.

## 5. The later steps

**Step 2 (vs-fp32, 189).** `nir.rs` `walk`, the `IrOp::Alu(a, s)` arm: add `Fsin Fcos Flog2 Fexp2 Fpow Fceil Fsign`
and the front end's `exp` (`FMA.f32 x, log2e` then `exp2`), `log` (`log2` then `FMA.f32 .., 0x3F317218`), `tan`
(highp, shared kernel as 1b) as `Alu` nodes; lower them through `transc32` (no envelope), `Fceil` -> `FROUND.f32.rtp`, `Fsign` ->
the `lower_fsign` pair (`Fcmp` x2 + `Fadd`; as `Fs::alu32` ~3389); `vs_clamp` accepts two computed operands as plain
`Fmax`/`Fmin` (`FMAX.f32 a, b` then `FMIN.f32`, the `op_common_clamp_vs_highp` order: bound `r2` first, then `r5`); the reduction `Fmax|Fmin` keeps
its constant rules. The kernels' constants come from the FAU (`u5.w1`, `u6.w1`): `fau.rs` already pools them for the fragment
kernel. Un-ignore the 21 vertex refs of the table (list in 3.1: `op_angle_{sin,cos}_vs_*`, `op_angle_tan_vs_highp`, `op_exponential_{exp2,log2,pow}_vs_*`,
`op_exponential_{exp,log}_vs_highp`, `op_common_{ceil,sign,min,max,clamp}_vs_highp`, `random_{fexp2,flog2,fsin}_vs_none`). The mediump references of `sin`,
`cos`, `exp2`, `log2`, `pow` are byte-equal to the highp ones, so one lowering serves both. Risk: `tests/mali_vs_arith.rs`
(`vsarith_*`: `vs_clamp` rules, `vsarith_182_fsat`), `mali_vs_fma.rs`, the glmark2 corpus (`fract(sqrt(..))`, `pow`): `vs_clamp`'s second-operand rule is the
most pinned; `mali-vs-compare-clamp.md` section 3 and `mali-vs-alu-ops.md` hold its references. Needs 1b's `transc32`.

**Step 4 (fs-pairs + `min max clamp`, 216).** Widen the pair builder's admitted forms (`ADMITTED`, ~9807, and the arms at 9012,
9345, 8391/8472 that raise `FsOperandForm`): a lane alone first in a sum, a uniform first (`op_binary_{add,sub}*`:
`FADD.v2f16 r1, r1^, r0.h00` with a **narrowed varying** `FADD.f32 r.h0, r, 0x0.neg` as one side and `V2F32_TO_V2F16` as the pair), `.neg/.abs`
as source modifiers on `Fneg`/`Fabs` lanes (`op_unary_*`, `op_common_abs`), `mix` as `FADD` + 3 `FMA` (nir_opt_algebraic `flrp`
lowering), `reflect` (`FMA` dot, `FADD x, x`, `FMA .neg`), `cross`, and the `exp2` of a varying inside a pair tree (~9457). `min/max/clamp`
of two lanes: the `Fmax|Fmin` arm narrows each non-fp16 operand with `fp16_operand` (`FADD.f32 r.h0`) and emits `FMAX.v2f16` then `FMIN.v2f16`
(`op_common_clamp_fs_mediump`); the highp references (`FMAX.f32`, `FMIN.f32`) go through `Fs::alu32`. Un-ignore 26 refs. This is the step
with the widest risk: `mali_fs_arith.rs`, `mali_fragment_random.rs`, the whole glmark2 corpus (pair registers and `MKVEC.v2i16` order),
`mali_deqp_mixprec.rs`. Split it in two sessions if the first leaves any of the pair-tree refs in `DIFFERS`: (a) `min max clamp` and the add/sub/mul
family, (b) `mix reflect cross`, matrix `+`, `storeoutput`.

**Step 3 (fs rounding and compare, 141).** Catch-all and `Fceil`: `floor`/`ceil` of a varying lane = `fp16_operand` then `Alu::Ffloor`/`Alu::Fceil` (`FROUND.v2f16.rtn/.rtp`;
hi `FROUND.f32.rtn/.rtp` via `alu32`), `sign` = two `Fcmp` lt `.f1` + `Fadd` (`FCMP_OR.v2f16.lt.f1`; the `.f1` is the `B2f` fold into the compare's
result type: `bir.rs` `B2f` doc), float `!=` = `FCMP_OR.f32.ne.m1` + `MUX.v2i16 0, 1.0, mask` (the operands are the varyings' fp32 words: this one needs the `Fne` arm
of `alu32` and the colour's `B2f16`, as `deqp_misc_cmp_notequal_vec2` does), `sqrt`/`inversesqrt` random programs (their arms exist; refusals are other ops of the program).
Un-ignore 14. Risk: `mali_deqp_misc.rs`, `mali_fs_arith.rs` (`fract` is `floor`'s).

**Step 5 (select, 96).** `Bcsel` arm: computed arms and a computed condition: mid `FCMP_OR.v2f16.lt.m1 0, c` + `MUX.v2i16 b, a, mask`; hi `S16_TO_S32` + `CSEL.u32.eq`
with fp32 arms (two forms in `op_selection_select_fs_{mediump,highp}`; the `none` one nests `MUX` of `MUX`). `indexing.dynamic_select` (60) is a
different mechanism (a 40-entry constant array stored to thread-local memory, then `LOAD.i16`: the `mali_locals.rs` / local-array machinery; the first
refusal reported as `Arithmetic` is the `Bcsel` of the front end's select chain, and the lowering is the array one): do it as its own session and keep it out of the count if
`locals` already has the form. Un-ignore 6 refs.

**Step 6 (vs fp16 forms, 159).** After `vs16-design`/`intval-vs16`: the vertex read of a mediump attribute as `FADD.f32 r.h0` / `V2F32_TO_V2F16` and fp16 pair ops
(`FMAX/FMIN.v2f16`, `FROUND.v2f16`, `FCMP_OR.v2f16.lt.f1`, `FMA.v2f16`), `exp`/`log`/`tan`'s fp16 tails, and `vs_reduce` for `== !=`/`any`/`all` of vectors
(ivec form `F32_TO_S32.rtz` + `ICMP_*.u32`, vec form fp16 `ICMP_OR.v2u16`). 20 refs; the `op_binary_{equal,not_equal}_vs_*` (6 refs) overlap `intval-rest`'s mixed-precision compares:
agree which item owns each before starting.

**Step 7 (composites, 48).** `acos`/`asin` (`FRSQ`+`FRCP` sqrt, two `FCMP.lt.f1` sign, `CSEL`), `atan`/`atan2`, `faceforward`, `refract`: each is the front end's
composition of ops lowered in 1a, 3 and 4 plus `CSEL.u32.eq`/`LSHIFT_*.i32` (copysign); 12 refs, byte-compare each: Mesa's CSE and scheduling order of long
`FMA` chains decide the match, so expect a `DIFFERS` sub-step.

**Step 8 (chains, 12).** `glsl.rs` `do_rebalance_tree` forms of 3+ adds/muls (`Rebalance`, ~1255-1383); `mali-rebalance.md`; the 1 reference
(`other_x_vs_mediump`).

**X1 / X2** are not this family's: constant folds are `lower-constop` (plan 377); the integer/boolean 325 are `intval-*`.

## 6. What this design does not know

- The site of each of the 144 references *at this tip* was not re-measured (no build here; section 2's arm table is from reading the code, the counts from
  `mali-deqp-arith.md`). Step 1's first action is to print each reference's refusal site; a highp reference that refuses somewhere other than the arm named here moves
  between steps (the counts per step are then those of this note's section 3 table, the mapping of reference to step the implementer's).
- Whether Mesa's CSE of the `sin`/`cos` kernel for `tan` and the `FMA` fusion of `mod`/`smoothstep` are already made by `Fs::op` is unchecked.
- A first-refusal count says nothing of the next refusal of the case; `deqp-host-measure.sh` after each step gives the `next_family` column.
