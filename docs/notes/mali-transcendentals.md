<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): sin, cos, log2 and exp2 in a fragment shader (case 77)

**Status: design, implementation in fs-transc** (plan #336, issue #33).
Re-checked at `04ac35d3` (see "Checked at 04ac35d3" at the end: the file:line
references below are the ones of that commit, the fragment-side steps were
rewritten there). This note changes no code. It maps each instruction of Mesa 26.2.1's program for
gl_test case 77 `transc` to the place in `src/shaderc/online/mali/` that has
to produce it, and lists the tests and the risks. The case is refused today
with `MaliError::Arithmetic` (mod.rs `message`, "arithmetic other than +, -,
* and a dot or matrix product ...") from the catch-all
`ir::Op::Alu(..) => return Err(MaliError::Arithmetic)` of the mediump walk
(nir_fs.rs:4216, after the `Bcsel` arm).

## The references

All in `tools/mali/testdata`, made with the drm-shim build of Mesa 26.2.1
(docs/notes/mali-alu-ops.md, "How the references were made"):

| files | program | bytes |
|---|---|---|
| `fsarith_77_transc.{vert,frag,expect.json}` | `es100/transc.frag` with `passthrough.vert` | the fragment shader, 42 words; this note's subject |
| `alu_transc.*` | the same pair | `cmp`-identical to the `fsarith_77_transc` expect.json |
| `fsarith_77_vtransc.*`, `alu_vtransc.*` | `es100/vtransc.frag`, the second program of case 77 | 38 words; see "vtransc" below |

The fragment source (`mediump`, `uniform vec4 u_a`):

```glsl
gl_FragColor = vec4(sin(u_a.x), cos(u_a.y), log2(u_a.z) * 0.25, exp2(u_a.w));
```

The vertex stage is the pass-through the compiler already accepts
(`fsarith_77_transc.expect.json` `vs.position`, nothing to do).

## What Mesa does, in order

`u_a` is a mediump uniform, read as fp16 pairs (`u2.w0` = x,y; `u2.w1` = z,w:
`UboLayout::fs16`). Each of the four built-ins is a NIR op on a 32-bit
source, with 16-bit values around it:

1. `bifrost_nir.c:42-49` (`bi_lower_bit_size`): `fexp2 flog2 fpow fsin fcos`
   with a source that is not 32-bit are lowered to 32 bits, so NIR is
   `f2f16(fsin(f2f32(u_a.x)))`. The `f2f32` is `F16_TO_F32` (the walk's
   `widen_any`), the `f2f16` of a scalar `FADD.f32 r.h0, r, -0.0`
   (emit.rs `(F2f16,16,1)`).
2. `* 0.25` is an fp16 product: the narrowing comes first, then
   `FMA.v2f16 r.h00, 0x34000000.h11, -0.0` (a lone lane of a `v2f16` FMA,
   emit.rs `(Fmul,16,1)`). It is not folded into the log2: `lower_precision`
   put the multiply at 16 bits.
3. `bi_emit_alu` (bifrost_compile.c) expands the 32-bit op:
   `nir_op_fsin`/`fcos` (:2907-2913) call `bi_lower_fsincos_32` (:2552);
   `nir_op_fexp2` (:2915-2923) calls `bi_fexp_32(.., bi_imm_f32(1.0))`
   (:2450); `nir_op_flog2` (:2925-2933) calls `bi_flog2_32` (:2499). The
   `bi_lower_fexp2_32`/`bi_lower_flog2_32` forms (polynomials, `FLOG_TABLE`,
   `FEXP_TABLE`) are only for `BIFROST_NO_FP32_TRANSCENDENTALS`, not set on
   v10: **not needed**.

The expansions, from bifrost_compile.c (constants: `TWO_OVER_PI` =
`2.0f / 3.14159f` = `0x3F22F98C`, `MPI_OVER_TWO` = `-3.14159f / 2.0` =
`0xBFC90FD0`, `SINCOS_BIAS` = `0x49400000`, both checked against the FAU
words of the reference):

```
bi_lower_fsincos_32(dst, s0, cos):                         # :2552
  x_u6  = FMA.f32 s0, TWO_OVER_PI, SINCOS_BIAS
  t     = FADD.f32 x_u6, -SINCOS_BIAS                      # va_fuse_add_imm: FADD_IMM #0xC9400000
  e     = FMA.f32 t, MPI_OVER_TWO, s0
  sinx  = FSIN_TABLE.u6 x_u6 (offset false)
  cosx  = FCOS_TABLE.u6 x_u6 (offset false)
  e2h   = FMA_RSCALE.f32 e, e, -0.0, #-1 (scale)           # e*e*2^-1
  quad  = FMA.f32 -e2h, (cos ? cosx : sinx), -0.0
  I     = FMA.f32.clamp_m1_1 e, (cos ? -sinx : cosx), quad
  dst   = FADD.f32 I, (cos ? cosx : sinx)

bi_fexp_32(dst, s0, log2_base = 1.0):                      # :2450
  scale = FMA_RSCALE.f32 s0, log2_base, -0.0, #24
  fixed = F32_TO_S32 scale          # round = BI_ROUND_NONE, no .rtz
  dst   = FEXP.f32 fixed, scale

bi_flog2_32(dst, s0):                                      # :2499
  e   = FREXPE.f32.log s0
  ef  = S32_TO_F32 e
  add = FADD_LSCALE.f32 -1.0, s0
  dst = FMA.f32 FLOGD.f32(s0), add, ef
```

## Instruction by instruction

`fsarith_77_transc.expect.json`, `fs.fragment` (42 words, work_reg_count 32,
preload `0x3000000000000000`). `u2.w0`/`u2.w1` are the fp16 pairs of `u_a`;
`u3.w0`, `u3.w1`, `u4.w0` are FAU constants 2/pi, the bias and -pi/2 (the
FAU is `[0 x6, 3f22f98c, 49400000, bfc90fd0, 0]`: words 0-5 the pushed
uniform block, 6-8 the promoted constants, `fau.rs` `lower_constants`, :690). `^` is the
packer's last-use. Rows are in program order; the word is the reference's.

### Lane x: `sin(u_a.x)` (words 0-10)

| # | instruction | word | operands, modifiers | produced by |
|---|---|---|---|---|
| 0 | `F16_TO_F32.discard r0, u2.w0.h0` | `6890c000000b0084` | src: fp16 uniform half h0 | existing: `Fs::widen_any` (nir_fs.rs:1671) -> `Alu::F2f32` row `(F2f32,32,1)` -> `Op::F16ToF32`. `.discard` is the fragment's (existing flow pass) |
| 1 | `FMA.f32 r1, r0, u3.w0, u3.w1` | `00b2c10000878600` | `x_u6`; both constants FAU words | new row `(Fsin,32,1)` first instruction: `Op::FmaF32 [s0, Const(0x3F22F98C), Const(0x49400000)]`; FAU promotion by existing `fau.rs` |
| 2 | `FADD_IMM.f32 r2, r1, #0xC9400000` | `0114c2c940000001` | `t`; the constant folded to an immediate | emit `Op::FaddF32 [x_u6, Const(0x49400000).neg]`; `opt.rs::fuse_add_imm` turns it into `FaddImmF32(0xC9400000)` (neg flips bit 31) |
| 3 | `FMA.f32 r0, r2^, u4.w0, r0^` | `00b2c00000408842` | `e = t * -pi/2 + s0` | emit `Op::FmaF32 [t, Const(0xBFC90FD0), s0]` |
| 4 | `FSIN_TABLE.u6 r2, r1` | `009cc20000040001` | src0 `x_u6`; `offset` 0 | new `Op::FsinTableU6` (bir.rs), pack.rs arm; ISA `FSIN_TABLE.u6` (`IsaOp::FsinTableU6`, v10.rs:425) |
| 5 | `FCOS_TABLE.u6 r1, r1^` | `009cc10000050041` | the same source | new `Op::FcosTableU6` (`IsaOp::FcosTableU6`, v10.rs:343). Emitted in Mesa's order: sin table, cos table, both for either `sin` or `cos` |
| 6 | `FMA_RSCALE.f32 r3, r0, r0, 0x0.neg, 0xFFFFFFFF` | `0160c304c1c00000` | `e*e`, addend -0.0, scale -1 (LUT int `-1`), `special` none | new `Op::FmaRscaleF32` (`IsaOp::FmaRscaleF32`, v10.rs:365: `src0..src3`, `special`, `round` 0); pack.rs arm; the scale is a plain integer source (no absneg), found in the constant table as a signed int |
| 7 | `FMA.f32 r3, r3^.neg, r2, 0x0.neg` | `00b2c34400c00243` | `quad = -e2h * sinx + -0.0` | `Op::FmaF32 [e2h with neg, sinx, NEG_ZERO]` (`Src.neg` of the first source) |
| 8 | `FMA.f32.clamp_m1_1 r0, r0^, r1^, r3^` | `00b2c00200434140` | `e * cosx + quad`, `clamp` = `m1_1` (2) | `Op::FmaF32 [e, cosx, quad]` with `Ins::clamp = clamp::M1_1`; the existing `FmaF32` pack arm already writes `clamp` (pack.rs:222). `bir::clamp::M1_1` (= 2) exists |
| 9 | `FADD.f32 r0, r0^, r2^` | `00a4c00000004240` | `+ sinx` | `Op::FaddF32 [I, sinx]` |
| 10 | `FADD.f32 r0.h0, r0^, 0x0.neg` | `00a440100000c040` | the narrowing, dest `.h0` | existing row `(F2f16,16,1)` (emit.rs:214-221), made by the store or by the walk (see "Where it goes") |

### Lane y: `cos(u_a.y)` (words 11-21)

Same as lane x (its rows 0-10 here as 11-21) with `u2.w0.h1` (`F16_TO_F32 r1, u2.w0.h1`, word
`0090c100100b0084`, no `.discard`), `r1`..`r4`, and the cos choices of
`bi_lower_fsincos_32`: the quadratic (lane row 7) reads the *cos* table result
(`FMA.f32 r4, r4^.neg, r2, 0x0.neg`, word `00b2c44400c00244`), lane row 8
reads the *negated sin* (`FMA.f32.clamp_m1_1 r1, r1^, r3^.neg, r4^`, word
`00b2c11200444341`: `neg` on the second source), lane row 9 adds `cosx`
(`FADD.f32 r1, r1^, r2^`, `00a4c10000004241`), and lane row 10 narrows
(`FADD.f32 r1.h0, r1^, 0x0.neg`, `00a441100000c041`). Words 11-21 in order:
`0090c100100b0084 00b2c20000878601 0114c3c940000002 00b2c10000418843
009cc30000040002 009cc20000050042 0160c404c1c00101 00b2c44400c00244
00b2c11200444341 00a4c10000004241 00a441100000c041`. In the sin lane the
FSIN result is `r2` and the FCOS `r1`; in the cos lane FSIN is `r3` and FCOS
`r2`: the allocator's choice, not a rule (see the risks).

### Lane z: `log2(u_a.z) * 0.25` (words 22-29)

| # | instruction | word | produced by |
|---|---|---|---|
| 22 | `F16_TO_F32 r2, u2.w1.h0` | `0090c200000b0085` | existing `widen_any` -> `F16ToF32` |
| 23 | `FREXPE.f32.log r3, r2` | `0099c30002020002` | new `Op::FrexpeF32 { log }` (`IsaOp::FrexpeF32`, v10.rs:405, `special` = `log` (2), `neg_result` 0) |
| 24 | `S32_TO_F32 r3, r3^` | `0090c30000090043` | existing `Op::S32ToF32` (emit `(I2f,32,1)`), source is the FREXPE integer |
| 25 | `FADD_LSCALE.f32 r4, 0x3F800000.neg, r2` | `00a4c440000902d0` | new `Op::FaddLscaleF32` (`IsaOp::FaddLscaleF32`, v10.rs:323: `src0` = -1.0 as LUT 1.0 with `neg0`, `src1` = x, `opcode2` 9) |
| 26 | `FLOGD.f32 r2, r2^` | `009cc20000080042` | new `Op::FlogdF32` (`IsaOp::FlogdF32`, v10.rs:349, SFU, `src0` only) |
| 27 | `FMA.f32 r2, r2^, r4^, r3^` | `00b2c20000434442` | `Op::FmaF32 [flogd, add, ef]` (Mesa: `bi_fma_f32_to(dst, flogd, add, frexpi)`) |
| 28 | `FADD.f32 r2.h0, r2^, 0x0.neg` | `00a442100000c042` | existing `(F2f16,16,1)` |
| 29 | `FMA.v2f16 r2, r2^.h00, 0x34000000.h11, 0x0.neg` | `00b3c2040ec0d942` | existing `(Fmul,16,1)` (emit.rs:276) with the fp16 constant 0.25 (`0x3400`) read in the high half; the generic mediump `Fmul` arm (nir_fs.rs ~4058) |

### Lane w: `exp2(u_a.w)` (words 30-35)

| # | instruction | word | produced by |
|---|---|---|---|
| 30 | `F16_TO_F32 r3, u2.w1.h1` | `0090c300100b0085` | existing `widen_any` |
| 31 | `IADD_IMM.i32 r4, 0x0, #0x18` | `0110c400000018c0` | existing `fau.rs` (:740, in `lower_constants`) `va_mov_imm`: the scale `Const(24)` does not fit the constant table; inserted before its reader |
| 32 | `FMA_RSCALE.f32 r3, r3^, 0x3F800000, 0x0.neg, r4^` | `0160c30444c0d043` | new `Op::FmaRscaleF32 [s0, ONE, NEG_ZERO, Const(24)]` |
| 33 | `F32_TO_S32 r4, r3` | `0090c400000c0003` | new `F32ToS32` with `round` 0 (no `.rtz`; note below) |
| 34 | `FEXP.f32 r3, r4^, r3^` | `00a4c30000084344` | new `Op::FexpF32 [fixed, scale]` |
| 35 | `FADD.f32.wait0126 r3.h0, r3^, 0x0.neg` | `40a443100000c043` | existing `(F2f16,16,1)`; the `.wait0126` is `flow.rs` for the following `ATEST` |

The table rows quote the words of the file; the full list with its
indexes follows and is the authority for the bytes.

Indexes, exactly as in the file (`text[i]` / `words[i]`):

```
 0 6890c000000b0084 F16_TO_F32.discard r0, u2.w0.h0
 1 00b2c10000878600 FMA.f32 r1, r0, u3.w0, u3.w1
 2 0114c2c940000001 FADD_IMM.f32 r2, r1, #0xC9400000
 3 00b2c00000408842 FMA.f32 r0, r2^, u4.w0, r0^
 4 009cc20000040001 FSIN_TABLE.u6 r2, r1
 5 009cc10000050041 FCOS_TABLE.u6 r1, r1^
 6 0160c304c1c00000 FMA_RSCALE.f32 r3, r0, r0, 0x0.neg, 0xFFFFFFFF
 7 00b2c34400c00243 FMA.f32 r3, r3^.neg, r2, 0x0.neg
 8 00b2c00200434140 FMA.f32.clamp_m1_1 r0, r0^, r1^, r3^
 9 00a4c00000004240 FADD.f32 r0, r0^, r2^
10 00a440100000c040 FADD.f32 r0.h0, r0^, 0x0.neg
11 0090c100100b0084 F16_TO_F32 r1, u2.w0.h1
12 00b2c20000878601 FMA.f32 r2, r1, u3.w0, u3.w1
13 0114c3c940000002 FADD_IMM.f32 r3, r2, #0xC9400000
14 00b2c10000418843 FMA.f32 r1, r3^, u4.w0, r1^
15 009cc30000040002 FSIN_TABLE.u6 r3, r2
16 009cc20000050042 FCOS_TABLE.u6 r2, r2^
17 0160c404c1c00101 FMA_RSCALE.f32 r4, r1, r1, 0x0.neg, 0xFFFFFFFF
18 00b2c44400c00244 FMA.f32 r4, r4^.neg, r2, 0x0.neg
19 00b2c11200444341 FMA.f32.clamp_m1_1 r1, r1^, r3^.neg, r4^
20 00a4c10000004241 FADD.f32 r1, r1^, r2^
21 00a441100000c041 FADD.f32 r1.h0, r1^, 0x0.neg
22 0090c200000b0085 F16_TO_F32 r2, u2.w1.h0
23 0099c30002020002 FREXPE.f32.log r3, r2
24 0090c30000090043 S32_TO_F32 r3, r3^
25 00a4c440000902d0 FADD_LSCALE.f32 r4, 0x3F800000.neg, r2
26 009cc20000080042 FLOGD.f32 r2, r2^
27 00b2c20000434442 FMA.f32 r2, r2^, r4^, r3^
28 00a442100000c042 FADD.f32 r2.h0, r2^, 0x0.neg
29 00b3c2040ec0d942 FMA.v2f16 r2, r2^.h00, 0x34000000.h11, 0x0.neg
30 0090c300100b0085 F16_TO_F32 r3, u2.w1.h1
31 0110c400000018c0 IADD_IMM.i32 r4, 0x0, #0x18
32 0160c30444c0d043 FMA_RSCALE.f32 r3, r3^, 0x3F800000, 0x0.neg, r4^
33 0090c400000c0003 F32_TO_S32 r4, r3
34 00a4c30000084344 FEXP.f32 r3, r4^, r3^
35 40a443100000c043 FADD.f32.wait0126 r3.h0, r3^, 0x0.neg
36 087dbc0204ea037c ATEST.wait0 @r60, r60^, r3.h0, atest_datum.w0
37 00a1c00000054140 MKVEC.v2i16 r0, r0^.h0, r1^.h0
38 48a1c10000054342 MKVEC.v2i16.wait r1, r2^.h0, r3^.h0
39 787f4004333c00f0 BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
40 0110f000000000c0 IADD_IMM.i32 r48, 0x0, #0x0
41 002fc1100000f1c0 BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

(`words` and `text` of the expect.json are parallel lists of 42; this block
was checked against the file line by line. Rows 36-41 are the epilogue.)

Notes on rows 22-35:

* 22: x of `u2.w1`'s low half, `F16_TO_F32 r2, u2.w1.h0` (no `.discard`).
* 23-27 is `bi_flog2_32` in Mesa's own call order: `frexpe`, `s32_to_f32`,
  `fadd_lscale`, `flogd`, `fma`. `FLOGD` stands after `FADD_LSCALE` and
  before the `FMA` as called (`bi_fma_f32_to(b, dst, bi_flogd_f32(b, s0),
  add, frexpi)` evaluates `bi_flogd_f32` as an argument, so after `add`).
* 28-29: log2's result is narrowed, then the product is `v2f16`.
* 30: `u_a.w` is `u2.w1.h1`.
* 31: the scale 24 of `bi_fexp_32`. It is a `Const(24)` source of the
  `FMA_RSCALE`; it does not fit the constant table or the FAU and
  `fau.rs` `lower_constants` (:740) puts it in a register with `IADD_IMM.i32 t,
  0x0 (lut), #imm`, **before** the instruction that reads it. No new code.
* 32: `FMA_RSCALE.f32 r3, r3^, 0x3F800000, 0x0.neg, r4^`: `src1` is
  `log2_base` = 1.0 (a constant-table entry), `src2` -0.0, `src3` the
  register holding 24.
* 33: **`F32_TO_S32` with `round` 0 (`.rte`, printed bare)**. The existing
  `Op::F32ToS32` always writes `round = Rtz` (pack.rs:382-388, the
  `.rtz` of every other reference: alu_divloop `0090c100c00c0041`, here
  `0090c400000c0003`). A second op (`F32ToS32Rne`?) or a `round` field on
  the existing op is needed. Mesa: `fixed_pt->round = BI_ROUND_NONE; // XXX`.
* 34: `FEXP.f32 r3, r4^, r3^`: `src0` the fixed-point integer (`r4`), `src1`
  the float `scale` (`r3`) for NaN propagation. New `Op::FexpF32` (`IsaOp::FexpF32`, v10.rs:345,
  `opcode2` 8, `widen1`, `neg1`/`abs1`).
* 35: `.wait0126` is `flow.rs` `insert_nops` for the `ATEST`; existing.
* 36-41: `ATEST`, `BLEND` and the end stub: existing. The two `MKVEC.v2i16`
  are **not** existing for a computed pair: the mediump colour is built by
  `vectorise` (nir_fs.rs:5638, the `Pairs` trees) and emitted at
  nir_fs.rs:6559 (`Leaf::Pack`, `Alu::Vec2` row); a pair whose lanes are two
  unrelated computed values is refused there (see "Checked at 04ac35d3",
  the next refusal). `Word`/`word_of` (nir_fs.rs:6587, :6753) are the
  *highp* colour's, not this shader's.

## Where each piece goes

### nir.rs (the node IR the walk builds)

`Alu` (nir.rs:96) gets `Fsin`, `Fcos`, `Flog2`, `Fexp2` (arity 1, fp32
nodes, opaque like `Frcp`/`Frsq`): add them to `arity` (:252), to the
constant-fold arm of `Nir::alu` (:1862-1933, the `ConstantFold` list: the
front end folds a constant operand), and to every exhaustive `match` on
`Alu` the compiler flags. Mesa expands them at `bi_emit_alu`, after all NIR
optimisation, so the node must stay one op until emission: no algebraic rule
touches it, and none of `Fmul`/`Fadd`/`Ffma`'s rules see the inner `FMA`s
(this is why the expansion is in emit.rs, not in the walk). `Nir::alu` still
CSEs the node (`nir_opt_cse`), as it does `Frsq`.

### nir_fs.rs (the walk)

Steps are in "Checked at 04ac35d3" below ("Fragment-side steps"); the
points that stay valid here:

* The new arm goes in the mediump walk before the catch-all
  `ir::Op::Alu(..) => return Err(MaliError::Arithmetic)` (nir_fs.rs:4216):
  `ir::Op::Alu(AluOp::Fsin | AluOp::Fcos | AluOp::Flog2 | AluOp::Fexp2, s)`.
  Per lane `i` of the result it takes `lane(vals, s[0], i)`, widens it
  (`f.widen_any(x, at)`, :1671: the `F16_TO_F32` of the uniform half, row
  `(F2f32,32,1)`), makes the fp32 node with `Fs::op32` (:1299; it does
  `mark_mixed` and `widen`), and narrows it with `f.node(Alu::F2f16, [r32,
  u, u], at)` (:971; a 16-bit node, so `is_f16` (:1616) is true, which keeps
  the next `* 0.25` at fp16). **Do not** leave the result as an fp32 lane:
  `Fs::fp32_lane` (:1606) is true only for `I2f | U2f | Fmul | Fadd | Ffma`
  nodes of 32 bits, and the `Fmul`/`Fadd` arms (~:4041-4090) would then
  make the product `FMA.f32`, the wrong instruction. Do not add the four to
  `fp32_lane`.
* The `Fmul` by 0.25 is the existing mediump arm: `node(Fmul, [lane16,
  Const(0.25)])`, row `(Fmul,16,1)`, constant `0x34000000.h11`.
* The colour of `vec4(sin, cos, log2 * 0.25, exp2)` pairs lanes of four
  different computed values. That is **not** `word_of`'s `Word::Lanes`
  (highp): it is `vectorise`'s pair of two lanes that match no tree, today
  refused (nir_fs.rs:5790). See the steps below for the leaf it needs.
* The highp path (`Fs::lane32`, `_ => Err(Arithmetic)` at nir_fs.rs:2142)
  gets the same four ops later for fmods (alu_fmods, mali-alu-ops.md),
  through the same `op32(Alu::Fsin, ..)` call; not for this item (no highp
  reference draws a bare sin; intuni_sharedint does but waits on other
  work). Its `Frsq` arm is nir_fs.rs:2047.

### emit.rs (the rows, `bi_emit_alu`)

`ROWS` (:39, now 87; the `rows` bitmask is a `u128`, so 4 more fit) gets
`(Fsin,32,1)`, `(Fcos,32,1)`, `(Flog2,32,1)`, `(Fexp2,32,1)`; `emit` has
one arm per row building the sequences above with `select::alu` (`ins`).
Sin and cos share a helper `fsincos(p, s0, cos)`. Temporaries are
`p.temp()` as the `(F2f16,16,1)` arm's. Source details:

* `s[0]` may be a uniform half or a register; the first `FmaF32` takes it in
  `src0`.
* Constants are `Src::of(Idx::Const(bits))`. They reach the FAU through
  the existing pass; `-0.0` is `NEG_ZERO`; `-1.0` of `FADD_LSCALE` is
  `Const(0x3F800000)` with `neg` (the reference prints `0x3F800000.neg`);
  the log2 base `1.0` is `ONE`.
* `FMA_RSCALE`'s scale is `Src::of(Idx::Const(0xFFFF_FFFF))` for the sine
  and `Const(24)` for the exp.
* `neg` of `e2h`, of `sinx`: `Src { neg: true, .. }`.

### bir.rs

New `Op`s (all have an `IsaOp` entry already, v10.rs:425, 343, 365, 323, 349,
405, 345: `FsinTableU6`, `FcosTableU6`, `FmaRscaleF32`, `FaddLscaleF32`,
`FlogdF32`, `FrexpeF32 { log: bool }` (or a fixed `special`), `FexpF32`, and a
`F32ToS32` with `round` none. For each, in this file:

* `src_info` (:775ff): `FsinTable/FcosTable/Flogd/Frexpe` source 0 as
  `info(true, true, false, false, 32)` (`FrsqF32`'s line; FSIN has no
  `absneg` in the ISA, so `(false, false, ..)` as `F16ToF32`'s: check
  against the ISA operands, v10.rs:3420, 3486, 4321, 4565, `src0` only, no `neg0/abs0`);
  `FmaRscaleF32` sources 0-2 `info(true, true, false, false, 32)`, source 3
  an integer `info(false, false, false, false, 32)`; `FaddLscaleF32` 0-1
  `info(true, true, ..)`; `FexpF32` source 0 an integer `info(false, ..)`,
  source 1 `info(true, true, ..)`.
* `nr_srcs` (:910ff): 1 for the four tables/`Flogd`/`Frexpe`, 4 for
  `FmaRscaleF32`, 2 for `FaddLscaleF32`/`FexpF32`.
* The destination-width list at :931 (`Op::FrsqF32` is in it: one 32-bit
  destination) for all.
* `is_signed` (:984): `FmaRscaleF32` if the LUT lookup of `-1` is by signed
  value; see the risks.

### opt.rs

The two tables `mods` (:62, `(abs, neg)` bits per source) and `takes_clamp` (:97) say
which modifier bits an op takes; `FmaRscaleF32` takes `neg`/`abs` on
sources 0-2 (`(0b111, 0b111)` as `FmaF32`) and `FaddLscaleF32` on 0-1, the
table ops none. `fuse_add_imm` is untouched (the `FADD_IMM` of row 2 comes
from the existing rule).

### pack.rs

One arm per new `Op` in `pack_one` (pattern: `Op::FrsqF32` :377,
`Op::FmaF32` :217):

* `FsinTableU6`/`FcosTableU6`: `Fill::new(IsaOp::...)`, `src0`, `offset` 0,
  `alu_dest(.., "dest_width")`.
* `FlogdF32`: `src0`, `widen0`, `neg0`/`abs0` (`fsrc`), `dest_width`.
* `FrexpeF32`: `src0`, `widen0`, `special` = 2 (`frexp_special_m::log`),
  `neg_result` 0, `dest_width`.
* `FaddLscaleF32`: `fsrc` 0 and 1, `round` 0, `clamp` 0, `sticky` 0,
  `dest_width_narrow`.
* `FmaRscaleF32`: `fsrc` 0-2, `src3` (an int source, `src_byte`),
  `special` 0 (none), `round` 0, `clamp` 0 (the `Ins::clamp`), `dest_width`.
* `FexpF32`: `src0` (int), `fsrc`-style `src1` with `widen1`, `neg1`,
  `abs1`, `dest_width`.
* `F32ToS32` none-round: as the present arm with `round` 0.

`pack.rs` is the generated `IsaOp` table's client, so a wrong field name is
a run-time `put` error in the first test, not a silent mis-encoding.

### select.rs

No new function. `select.rs` is the vertex stage's node loop (:632
`emit::emit(p, Row { op, bits, width }, &s)`) and the shared `alu` helper
(:207) that `emit.rs` calls as `ins`. The vertex walk (`nir.rs`) does not
get the new `Alu`s in this item. If the same four ops are later wanted for
the vertex stage, the rows are shared and `select.rs` needs nothing more.

### The ISA table

`src/shaderc/online/mali/isa/v10.rs` already has every opcode needed:
`FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32`, `FADD_LSCALE.f32`,
`FLOGD.f32`, `FREXPE.f32` (with `frexp_special_m` `log`), `FEXP.f32`,
`S32_TO_F32`, `F32_TO_S32`, `F16_TO_F32`, `FRSQ.f16` (vtransc), `FADD_IMM`,
`IADD_IMM`, `FMA.f32`, `V2F32_TO_V2F16`. It is generated
(`tools/mali/gen_isa_rs.py`); nothing to regenerate.

## Host tests fs-transc adds

All in `src/shaderc/online/mali/tests.rs`, in the style of
`the_flattened_ifs_are_byte_equal_to_mesa_26_2_1` (`check_program` /
`check_fs_stage`, which compare words, `work_reg_count`, preload, FAU and
padding):

1. `the_transcendentals_are_byte_equal_to_mesa_26_2_1`: `fsarith_77_transc`
   (`td!(".vert")`, `.frag`, `.expect.json`), both stages, plus
   `assert_eq!(include_str!("../../../../tools/shaders/es100/transc.frag"),
   td!("fsarith_77_transc.frag"))` and the same for `alu_transc` (the corpus
   file is the reference's source). Add `fsarith_77_transc` and
   `fsarith_77_vtransc` to the population in `fs_reference_sources()` (:2051)
   so `the_selection_rows_are_the_references` draws the four new rows.
2. Four small units (each against a one-lane program) that the rows are
   what is named: for `sin`, `cos`, `log2`, `exp2` of a mediump uniform
   lane, the instruction mnemonics in order (a disassembly through the ISA
   table, as the other row tests do) equal rows 0-10, 11-21, 22-29, 30-35
   of the index above. These locate a failure to one row before the whole
   64-bit word comparison.
3. `the_exp2_scale_is_a_register`: the `IADD_IMM #0x18` before `FMA_RSCALE`
   (nothing but `fau.rs` makes it; the test fixes the interaction).
4. `an_fp32_transcendental_result_is_narrowed_before_an_fp16_product`: the
   fp16-ness of `log2(z) * 0.25` (rows 28-29) with `log2(z)` alone (no
   `FMA.v2f16`, just row 28).
5. Constant fold refusals: `sin(1.0)` reaches the arm only if the front end
   did not fold it; assert `MaliError::ConstantFold` for a constant
   operand, as the other `Alu` folds (nir.rs:1872-1933).
6. ISA round trip for the six new `Op`s: `pack_one` of a hand-built `Ins`
   equals the reference word of the matching row (words 4, 5, 6, 23, 25,
   26, 31, 32, 33, 34 above), so a packer error shows without the whole
   program.
7. `ROWS`: the row count constant and the `bit` mask test (87 -> 91).
8. The corpus: `tools/mali/corpus-verdicts.txt` moves `es100/transc.frag`
   from the `Arithmetic` line to `ok` (that file is another item's: only
   the planner/gate item edits it; the regression-set run is
   `tools/mali/sweep_passsets.py --delta tools/cts/log`).

## vtransc (the second program of case 77)

`fsarith_77_vtransc.expect.json` (38 words, bytes equal to `alu_vtransc`):
`vec3 s = sin(u_v.xyz); vec2 r = inversesqrt(vec2(1.0, u_v.w));
gl_FragColor = vec4(s, r.y)`. It adds three things to this design:

* `FRSQ.f16 r2, u2.w1.h11` (`.wait0126`): `inversesqrt(u_v.w)` at fp16 is
  `Frsq` with `bits: 16`; a new `Op::FrsqF16` (`IsaOp::FrsqF16`, v10.rs:417) and an emit row
  `(Frsq,16,1)`; `vectorize16` already excludes `Frsq`/`F2f16`
  (nir_fs.rs:7176 `vectorize16`, the `!matches!` at :7179). `inversesqrt(1.0)` is folded.
* `V2F32_TO_V2F16 r0, r0^, r1^`: the narrowing of `s.x` and `s.y` as a pair
  (`(F2f16,16,2)` exists). Here the two sines are lanes 0 and 1 of one
  value, and `nir_opt_vectorize` pairs their `f2f16`s (the same rule
  `Leaf::Half` encodes for loads in `vectorise`, nir_fs.rs:5727-5736, and
  `Word::Pair` for the highp colour, :6587: "components of one value in one
  aligned pair"); `s.z` is narrowed alone (`FADD.f32 r1.h0`)
  and made a word with `r` (`MKVEC.v2i16 r1, r1.h0, r2.h0`). So in this item
  the narrowing must not be forced in the arm: the arm's results are kept
  as fp32 lanes tagged with their IR value and lane, and the colour store
  chooses between a pair leaf (an aligned pair of one IR value's lanes
  computed by an op: a new case of `vectorise`, :5638) and `Leaf::Pack`
  (:5487, emitted at :6559). For case 77
  itself the lanes come from four different IR values, so `Leaf::Pack` follows;
  the `* 0.25` of lane z is then the only reader that sees an fp16 value, so
  that lane alone is narrowed in the walk. If this is too much for the
  first commit, implement transc (rows 0-41) first and vtransc as a second
  commit; transc does not need the pairing.
* Three sines are computed one after the other (`F16_TO_F32` of `u2.w0.h0`,
  `.h1`, `u2.w1.h0`), the second and third without `.discard`.

## Risks

* **Scheduling order.** Mesa's `bi_pressure_schedule` (sched.rs, a
  transcription) reorders the four lanes' chains: the reference has lane x's
  chain whole, then lane y's, then z, then w, with the `F16_TO_F32` of the
  next lane after the previous lane's narrowing. The order the nodes are
  created in (`Nir::push_node`, `order`) and the DAG's tie-break (creation
  order, util/dag.c's head list) must be Mesa's: NIR order is the GLSL
  statement's constructor order, one instruction per lane (rows 0..42 of
  the program). The four ops must be added to the walk in lane order
  (x, y, z, w), `F16_TO_F32` first of each. A deviation shows as the same
  multiset of words in another order, a pure `assert_words` failure.
* **Register allocation.** `r0..r4` of the reference come from Mesa's
  `bi_ra_ssa` (ra.rs transcribed). Row 4/5 target `r2`/`r1` in the sin lane
  and `r3`/`r2` in the cos lane; `FSIN_TABLE` writes a fresh register
  while `FCOS_TABLE r1, r1^` writes over its own source (last use). The
  allocator is already the byte-exact one for the other references; the
  risk is a missing last-use flag (`^`) on a new op's source (the packer's
  `discard` bit, `Ins::src[..].discard`): `src_info` must give each new
  source its width so liveness sees all of them. `FEXP`'s two sources and
  `FMA_RSCALE`'s fourth are the likely misses.
* **`src_info` for the new ops.** `bir.rs` `src_info`/`nr_srcs` are
  consulted by FAU promotion (`resolve`/`lookup`), liveness and the
  packer; one wrong entry changes the constant table (e.g. `-1` for the
  scale). The FAU order of row 1-3's three constants (`3f22f98c, 49400000,
  bfc90fd0`) depends on `fau.rs` counting each constant (2 uses each) and
  tie-breaking by first use; a reordering of the instructions changes the
  FAU bytes and the `u3`/`u4` numbers.
* **`clamp_m1_1`.** `bir::clamp::M1_1` exists (bir.rs:241) and the `FmaF32` pack arm writes `i.clamp`; opt.rs `takes_clamp` must not strip a clamp from an `FmaF32` that is not an `FCLAMP`. A test of row 8 alone shows it.
* **`F32_TO_S32` rounding.** See row 33: using the existing op writes
  `.rtz` and fails only at that word.
* **`FADD_IMM` fusion.** Row 2 is `fuse_add_imm`, which requires the other
  source to have the `H01` swizzle and no modifier; an `FMA` result has
  both. A `.neg` constant flips bit 31 (`0x49400000 -> 0xC9400000`): a
  mis-specified `neg` produces `FADD` with a FAU constant and a different
  word.
* **The product by 0.25 is fp16.** If the walk leaves the log2 as an fp32
  lane, `fp32_lane`/the `Fmul`/`Fadd` arms (~:4041-4090) with `fp32_lane` would draw `FMA.f32 r, 0.25, -0.0`
  and one `FADD.f32 .h0` after: different words, and the log2 would be
  *more* precise than Mesa's. The reference is the authority.
* **Precision / semantics.** The sine and cosine take the Taylor
  approximation of `bi_lower_fsincos_32`, not an exact function: the board
  result of case 77 (0.5, 0.5, 0.25, 0.5 in eight bits, see the .frag
  comment) depends on the hardware tables, which only the board session can
  check; the host test checks bytes. The comment in the `.frag` says the
  tables take the argument in units of pi; that is what the
  `2/3.14159` and `-3.14159/2` constants compensate for, so the constants
  must be the f32 *roundings of Mesa's two expressions* (`0x3F22F98C`,
  `0xBFC90FD0`, computed in `f32`: `2.0f / 3.14159f`, not `0.63661977`).
* **A highp shader.** Mesa's `bi_emit_alu` is the same at fp32, but the
  walk's highp path (`lane32`) has no reference with a bare `sin`; the rows
  are shared, the highp arm is another item's.
* **Other shaders with these ops.** `es100/fmods` (alu_fmods) uses `log2`
  and `exp2` highp, twice each; `intuni_sharedint` a highp `sin`. Neither is
  in case 77. They need the highp arm above and add no new row, so the
  rows tested here are the rows they will use.

## Verification of this note

The tables were read from `tools/mali/testdata/fsarith_77_transc.expect.json`
(fs.fragment `words` and `text`, 42 each), `fsarith_77_vtransc.expect.json`,
and Mesa 26.2.1 `src/panfrost/compiler/bifrost/bifrost_compile.c` (lines as
given) and `bifrost_nir.c`. The constants were recomputed in `f32`. No code
was run against the new rows (there are none yet).

## Checked at 04ac35d3

Checked on the tip `04ac35d3` (plan #336, item transc-check). No code was
changed; the throwaway edits of "The next refusal of case 77" were reverted.

### References and the rerun

* Every `file:line` above was re-read on this commit and corrected in place:
  `fau.rs` has no `promote` (the pass is `lower_constants`, :690, the
  `va_mov_imm` comment :740); `arity` is nir.rs:252; the `ConstantFold`
  arms are nir.rs:1872-1933; opt.rs's tables are `mods` (:62) and
  `takes_clamp` (:97); the `v10.rs:NN` numbers the note gave were `IsaOp`
  discriminants, now written with the variant and its line (`FsinTableU6`
  :425, `FcosTableU6` :343, `FmaRscaleF32` :365, `FaddLscaleF32` :323,
  `FlogdF32` :349, `FrexpeF32` :405, `FexpF32` :345, `FrsqF16` :417); and
  the mediump colour is `vectorise` (nir_fs.rs:5638) and its `Pairs`
  trees, not `word_of`/`Word` (:6753, :6587), which are the highp colour's.
  The rest resolved as written: nir_fs.rs:4216, :1671, :1299, :1606, :1616,
  :2142, :7176; emit.rs:39 (87 rows), :200, :214-221, :276; pack.rs:217,
  :222, :377, :382-388; bir.rs:241, :776, :914, :984; tests.rs:2051;
  nir.rs:96.
* Mesa 26.2.1 (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, the tree
  `build_pan_shc.sh` made) was re-read: `bifrost_nir.c:42-49`;
  `bifrost_compile.c` :2450 (`bi_fexp_32`), :2457 (`round = BI_ROUND_NONE`),
  :2499 (`bi_flog2_32`), :2547-2549 (the three constants), :2552
  (`bi_lower_fsincos_32`), :2907, :2911, :2915, :2925 (the four `case`s). All
  as the note says. `2.0f / 3.14159f` and `-3.14159f / 2.0` round to
  `0x3F22F98C` and `0xBFC90FD0`.
* The committed reference was re-read: `fsarith_77_transc.expect.json`
  (`regions["fs.fragment"]`: 42 words, `work_reg_count` 32, preload
  `0x3000000000000000`, FAU `[0 x6, 3f22f98c, 49400000, bfc90fd0, 0]`); its
  42 words equal the index above one by one (checked by script), and
  `alu_transc.expect.json` is the same JSON. `fsarith_77_vtransc` has 38
  words.
* **The Mesa shim was rerun and matches.** `tools/mali/drm_shim_ref.sh
  build` of Mesa 26.2.1 (LLVM 21, in `$CONNECTED_DEPS_DIR/mesa-shim`), then
  `run` of each pair and `pan_ref_diff.py extract --draw`:

      tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
          tools/mali/testdata/fsarith_77_transc.vert tools/mali/testdata/fsarith_77_transc.frag $TMPDIR/transc
      python3 -I tools/mali/pan_ref_diff.py extract $TMPDIR/transc.trace --draw > $TMPDIR/transc.json
      cmp $TMPDIR/transc.json tools/mali/testdata/fsarith_77_transc.expect.json

  `cmp` is silent for `fsarith_77_transc` and for `fsarith_77_vtransc`: both
  expect.json files are reproduced byte for byte. The reference words above
  stand unchanged. The traces were not committed.

### The ISA ops fs-transc needs: all missing on this tip

`isa-transc` is not on `04ac35d3`: bir.rs, pack.rs, select.rs and emit.rs
have no `Fsin`, `FsinTable`, `FcosTable`, `FmaRscale`, `FaddLscale`,
`Flogd`, `Frexpe`, `FexpF32` or `FrsqF16`. Present: `Op::F32ToS32` (bir.rs:450;
pack.rs:382, `round` fixed to `Rtz`, :386), `FmaF32` with `clamp`,
`FaddImmF32` (bir.rs:320, opt.rs `fuse_add_imm` :404), `S32ToF32`,
`F16ToF32`, `V2f32ToV2f16`. The generated `IsaOp` entries all exist
(v10.rs). What fs-transc needs from isa-transc, with the word to test:

| `Op` (bir.rs) | `IsaOp` | operand form | modifiers | word |
|---|---|---|---|---|
| `FsinTableU6` | :425 | `src0` fp32 register; no neg/abs | `offset` 0 | `009cc20000040001` |
| `FcosTableU6` | :343 | the same | `offset` 0 | `009cc10000050041` |
| `FmaRscaleF32` | :365 | `src0..src2` fp32 with `neg`/`abs` (`neg0..2`, `abs0..2`), `src3` an integer source (the scale: LUT `-1` = `0xFFFFFFFF`, or a register holding 24) | `special` 0, `round` 0, `clamp` 0 | `0160c304c1c00000` (e*e*2^-1), `0160c30444c0d043` (exp: `s0, 1.0, -0.0, r(24)`) |
| `FaddLscaleF32` | :323 | `src0` (-1.0 = LUT 1.0 with `neg0`) and `src1` fp32 | `round` 0, `clamp` 0 | `00a4c440000902d0` |
| `FlogdF32` | :349 | `src0` fp32 only | none | `009cc20000080042` |
| `FrexpeF32` | :405 | `src0` fp32 | `special` = `log` (2), `widen0` 0, `neg_result` 0 | `0099c30002020002` |
| `FexpF32` | :345 | `src0` an **integer** (the fixed point), `src1` fp32 (`widen1`, `neg1`, `abs1`) | none | `00a4c30000084344` |
| `F32ToS32` with `round` 0 | :307 | as present | `round` 0 (bare, not `.rtz`) | `0090c400000c0003` (present arm writes `.rtz`: `0090c100c00c0041`) |
| `FrsqF16` (vtransc only) | :417 | `src0` an fp16 pair half | none | in `fsarith_77_vtransc` |

`src_info`, `nr_srcs`, the destination-width list and `mods` as the
sections above say. `FmaRscaleF32` source 3 and `FexpF32` source 0 are
integer sources, and the constant `-1` is found by the constant-table
lookup as an integer, so `is_signed` (bir.rs:984) needs `FmaRscaleF32` if
that lookup of `0xFFFFFFFF` goes by signed value.

### Fragment-side steps (fs-transc, in this order)

1. **nir.rs**: `Alu::{Fsin, Fcos, Flog2, Fexp2}` (:96), arity 1 (:252), the
   `ConstantFold` list (:1872-1933), every exhaustive `match` the compiler
   flags. The nodes stay single ops until emit.rs.
2. **nir_fs.rs:4216 arm** (text under "nir_fs.rs (the walk)" above), per
   lane in order x, y, z, w: `widen_any` (the `F16_TO_F32`) ->
   `op32(Alu::Fsin | ..)` -> `node(Alu::F2f16, ..)`. With the op stubbed as
   `Alu::Frcp` the shader is walked to the colour (see the next section),
   so `widen_any`, `op32` and `node(F2f16)` accept these shapes.
3. **The colour pair** (new; the earlier note put it under `word_of`):
   both pairs of case 77, `(sin, cos)` and `(log2 * 0.25, exp2)`, are two
   *lone computed lanes*. `vectorise` (nir_fs.rs:5638) has no case for
   them; its last arm (:5790) answers `ColourForm`. Needed: a case before
   that arm taking the pair as `Leaf::Pack { x, y }` (:5487; emitted at
   :6559 as `MKVEC.v2i16 x.h0, y.h0`; `Form::Mk` :5605; `mark` :6136), with
   `x` and `y` from `lane_tree` (:5962), whose new arm reads a lane that is
   an `F2f16` of an `Fsin | Fcos | Flog2 | Fexp2` of a uniform's component
   and builds a leaf for it (`pr.scalar[x] = true`). `lane_tree`'s `Fmul`
   arm then covers lane z's `* 0.25` as it is (`FMA.v2f16 r2, r2^.h00,
   0x34000000.h11, 0x0.neg`, word 29). The leaf is emitted like
   `Leaf::Frag` (`frag16`, nir_fs.rs:8722: rows drawn directly, the fp32
   chain, then `(F2f16,16,1)`, then the pack); `operand` (:8761) reads it
   from `val`, and `mark`/`form` list it. The narrowing of words 10, 21,
   28, 35 is that `(F2f16,16,1)` row. The program order is lane order x, y,
   z, w, `F16_TO_F32` first of each (creation order; sched.rs keeps it).
4. **emit.rs**: four rows `(Fsin|Fcos|Flog2|Fexp2, 32, 1)` and their arms,
   as the expansions above (ROWS 87 -> 91; `bit` :200 is `u128`), sin and
   cos sharing `fsincos(p, s0, cos)`; `-0.0` is `NEG_ZERO`, `1.0` is `ONE`.
5. **bir/pack/opt/select**: the isa-transc ops (table above).
6. **Precision.** *mediump* (case 77; the operand is an fp16 uniform lane):
   all four evaluate at fp32 (`bi_lower_bit_size` widens the source and
   narrows the result: `F16_TO_F32` ... `FADD.f32 r.h0, r, -0.0`, to nearest
   even); the product `* 0.25` follows at fp16 (`Fmul` 16,1; do not fold it
   into log2); sin and cos use Mesa's correction around the table lookup
   with the f32 constants `0x3F22F98C` (2/pi), `0xBFC90FD0` (-pi/2) and
   `0x49400000` (bias; `0xC9400000` as the `FADD_IMM`). *highp*: the same
   expansions without `F16_TO_F32` and without the narrowing; the arm of
   `lane32` (:2142; fmods, intuni_sharedint) is a later item. A highp
   result stored to the mediump colour is narrowed by the store as any fp32
   lane.
7. **Tests** (tests.rs), no new testdata: the references are
   `fsarith_77_transc.{vert,frag,expect.json}` and
   `fsarith_77_vtransc.{vert,frag,expect.json}`, already in
   tools/mali/testdata. Add `TRANSC: [(&str, &str, &str, &str); 2]` of
   `("fsarith_77_transc", td!("fsarith_77_transc.vert"), td!(".frag"),
   td!(".expect.json"))` (vtransc in the second commit), tested by
   `the_transcendentals_are_byte_equal_to_mesa_26_2_1`, the loop of
   `the_flattened_ifs_are_byte_equal_to_mesa_26_2_1` (tests.rs:6055);
   `the_corpus_transcendental_sources_are_the_references` (the `assert_eq!`
   of `es100/transc.frag` and `vtransc.frag` with the testdata copies, as
   `the_corpus_flattened_if_sources_are_the_references`); the sources in
   `fs_reference_sources()` (:2051) for
   `the_selection_rows_are_the_references` (:2002); and units 2-7 of
   "Host tests fs-transc adds". `tools/mali/corpus-verdicts.txt` line 101
   (`es100/transc.frag`) -> `ok`; line 132 (`vtransc.frag`) stays
   `Arithmetic` until `Frsq` at 16 bits and the `V2F32_TO_V2F16` pair exist.

### The next refusal of case 77

Case 77 is two programs, `transc` and `vtransc`. With the refusal at
nir_fs.rs:4216 disabled by a throwaway arm (the four ops widened, computed
as `Alu::Frcp` on an fp32 node, narrowed by `node(F2f16)`; reverted, `git
diff` empty), `tools/mali/corpus-verdicts.sh` gave:

* `es100/transc.frag`: **`MaliError::ColourForm`** ("a gl_FragColor lane
  other than the varying's own lane or a constant is not lowered yet"),
  raised at nir_fs.rs:5790, the last arm of `vectorise`, for the pair
  `(sin, cos)`: step 3. Nothing between the arm and the colour refuses.
* `es100/vtransc.frag`: still **`MaliError::Arithmetic`**, from the same
  catch-all (:4216): the mediump walk has no `Frsq` arm (only `lane32` has
  one, :2047), so `inversesqrt(u_v.w)` is refused before the colour. Beyond
  that, by reading and not by a run: `(s.x, s.y)` is two lanes of one value
  narrowed together (`V2F32_TO_V2F16`), `(s.z, r.y)` a `Leaf::Pack`; both
  are `vectorise` cases that do not exist.
