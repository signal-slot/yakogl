<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the highp fragment shapes, step 4a's pair trees, `mat4_pair_unread` and `load_mat3`: design and Mesa references (plan #387, item highpfs-refs)

Train 2 (`arith-lower-2`) left four groups of steps undone, and their
prototype diffs are lost. Only the notes describe them:

| Group | Where the notes describe it | References here |
|---|---|---|
| highp fragment shapes (`hp_*`) | `mali-arith-lowering.md` 4.1 ("left refused": `mod`, `smoothstep` highp), 4.2 / "Step 1b landed ... Not landed: the six `*_fs_highp` references", step 3 (`sign` highp) | 17 |
| step 4a's pair trees (`pt_*`) | `mali-arith-lowering.md` section 5, "Step 4" (a): `min max clamp` and the add/sub/mul family; `mali-lowering-schedule.md` A4a; `mali-opform-lowering.md` O1, O4 | 21 |
| `mat4_pair_unread` (`m4_*`) | `mali-deqp-irindex.md` 5.2 | 5 |
| `load_mat3` (`l3_*`) | `mali-deqp-irindex.md` 5.3 | 7 |

This note measures them at the integration tip `4d8a0b40` (after train 3a),
draws Mesa 26.2.1 references for the dEQP-GLES2 shapes that had none
(`tools/mali/testdata/deqp_highpfs_<shape>.{vert,frag,expect.json}`,
`tests/mali_deqp_highpfs.rs`), and writes each step: Mesa's shape, the
lowering, the dEQP groups and a forecast. No compiler code was changed.

## 1. Method (HOST-ONLY)

- **Sources.** The dEQP-GLES2 generator sources at VK-GL-CTS `93bca01`
  (`tools/cts/deps.toml`), fetched sparse into the dependency cache:
  `modules/gles2/functional/es2fShaderOperatorTests.cpp` (`ShaderOperatorCase`,
  `s_inSwizzles`, the scale/bias of each `BuiltinFuncInfo`),
  `es2fShaderMatrixTests.cpp` (`ShaderMatrixCase::init`),
  `modules/glshared/glsShaderLibraryCase.cpp` and
  `framework/opengl/gluShaderLibrary.cpp` (`genCompareFunctions`) with
  `data/gles2/shaders/{conversions,functions}.test`. A scratch script writes
  each case's pair as those templates do. Checks: its
  `matrix.add.dynamic_mediump_mat4_float_fragment` is byte-identical to the
  `.qpa`-verbatim `deqp_irindex_mat4_pair_unread`, and its
  `sin.highp_float_fragment` and the other `*_float` cases reproduce the
  refusals of the pinned `deqp_arith_*` references.
- **Measure.** `tests/mali_ref::compile_pair` on every generated pair at
  `4d8a0b40` (feature set `host-test,gpu-mali,shaderc-online,no-shader-table`),
  through a scratch test file that was not committed. The first refusal of
  the pair is the case's first refusal: the vertex shaders of these cases are
  pass-throughs. All 206 cases named below are in
  `tools/cts/gles2-mustpass.txt`. These are host compile outcomes, not board
  results.
- **References.** `tools/mali/deqp_ref.sh <pair> tools/mali/testdata/deqp_highpfs_<shape> --instrument shim`
  (`MESA_SHIM_WORK=$CONNECTED_DEPS_DIR/mesa-shim`, built by
  `tools/mali/drm_shim_ref.sh build`). Before drawing, the shim was checked
  against three committed references (`deqp_irindex_load_mat3`,
  `deqp_intval5_c_bool3`, `deqp_arith_op_angle_sin_fs_highp`): they came out
  byte-identical. `.nir.sha` files are from `tools/mali/nir_refs.sh`. G310 =
  G610 (`mali-g310-vs-g610.md`) is assumed, not rechecked for these 50 references.
- **Audit of what compiles.** For every case that compiles at the tip, a
  Mesa reference was drawn in scratch and compared. Two of the compiling
  shapes differ and are pinned (`DIFFERS`); a byte-equal representative of
  the others is pinned (`LOWERED`).

## 2. What the tip does (measured at `4d8a0b40`)

### 2.1 Highp fragment operator cases (50 must-pass cases)

`shaders.operator.<group>.<fn>.highp_<types>_fragment`, every type variant
dEQP-GLES2 has:

| Function | float | vec2 | vec3 | vec4 | vecN_float / float_vecN | First refusals |
|---|---|---|---|---|---|---|
| `sin cos tan log2 log pow` (6 x 4) | MIXED | ARITH | ARITH | ARITH | - | MIXED 6, ARITH 18 |
| `exp` | ARITH | ARITH | ARITH | ARITH | - | ARITH 4 |
| `sign` | ARITH | ARITH | ARITH | ARITH | - | ARITH 4 |
| `exp2` | FORM | FORM | OPERAND | OPERAND | - | FORM 2, OPERAND 2 |
| `mod` | LOAD | FORM | FORM | FORM | vec2_float LOAD, vec3_float LOAD, vec4_float FORM | FORM 4, LOAD 3 |
| `smoothstep` | FORM | FORM | FORM | FORM | float_vec2 LOAD, float_vec3 LOAD, float_vec4 FORM | FORM 5, LOAD 2 |

ARITH = `Arithmetic`, MIXED = `MixedPrecision`, FORM = `ColourForm`,
OPERAND = `FsOperandForm`, LOAD = `FsVaryingLoad` (the whole texts are the
constants of `tests/mali_deqp_highpfs.rs`). In all: ARITH 26, MIXED 6,
FORM 11, LOAD 5, OPERAND 2.

### 2.2 Step 4a's pair-tree cases (144 must-pass cases)

`shaders.operator.binary_operator.{add,sub,mul}{,_assign_effect}.{lowp,mediump}_*_fragment`
and `common_functions.{min,max,clamp}.{lowp,mediump}_*_fragment`. **lowp
and mediump give the same outcome in every case**, and Mesa's words for
lowp equal mediump's (checked on `add_vec2`, `add_vec3`, `mul_vec3`,
`min_vec3`), so each count below is for both precisions together (2 x the
per-precision count):

| Form | add | sub | mul | add/sub/mul `_assign` | min / max / clamp | Outcome |
|---|---|---|---|---|---|---|
| `float` (one lane) | compiles | compiles | compiles | compiles | compiles | 2 x 9 = 18 byte-equal |
| `vec3` (both vec3) | compiles | compiles | OPERAND | add, sub compile; mul OPERAND | OPERAND | 2 x 4 byte-equal + 2 x 5 OPERAND |
| `vec3_float` | OPERAND | OPERAND | **compiles, differs** | add, sub OPERAND; **mul compiles, differs** | OPERAND | 2 x 2 DIFFERS + 2 x 7 OPERAND |
| `float_vec3` | OPERAND | OPERAND | OPERAND | - | - | 2 x 3 OPERAND |
| `float_vec4` | OPERAND | OPERAND | compiles | - | - | 2 x 1 byte-equal + 2 x 2 OPERAND |
| `vec2`, `vec4`, `vec2_float`, `vec4_float`, `float_vec2` | FORM (5) | FORM (5) | FORM (5) | FORM (4 each) | FORM (4 each) | 2 x 39 FORM |

In all: 28 byte-equal, 4 DIFFERS, 34 OPERAND, 78 FORM (144).

### 2.3 `mat4_pair_unread` and `load_mat3`

- `shaders.matrix.{add,sub,mul}.dynamic_{lowp,mediump}_mat4_float_fragment`,
  `post_{increment,decrement}.dynamic_{lowp,mediump}_mat4_fragment` (the 10
  cases of `mali-deqp-irindex.md` section 4): `add`, `sub` and the two
  postfix ones (8) are `Defect(IrIndex)`. The two `mul` cases are now
  `AlgebraicForm` (constprod K8's text), so the `IrIndex` site is behind or
  gone for them.
- The 7 `load_mat3` siblings drawn here (section 3) are all
  `Defect(IrIndex)` at the tip, like `deqp_irindex_load_mat3`.

## 3. The references

50 references; `refusals_pinned` holds the outcome of each.

| Reference | dEQP-GLES2 case (`functional.`) | Tip | Step | fs words |
|---|---|---|---|--:|
| `hp_sin_vec2` | `shaders.operator.angle_and_trigonometry.sin.highp_vec2_fragment` | ARITH | HF1 | 29 |
| `hp_sin_vec3` | `...sin.highp_vec3_fragment` | ARITH | HF1 | 39 |
| `hp_sin_vec4` | `...sin.highp_vec4_fragment` | ARITH | HF1 | 52 |
| `hp_cos_vec3` | `...cos.highp_vec3_fragment` | ARITH | HF1 | 39 |
| `hp_tan_vec3` | `...tan.highp_vec3_fragment` | ARITH | HF1 | 54 |
| `hp_log2_vec3` | `shaders.operator.exponential.log2.highp_vec3_fragment` | ARITH | HF1 | 25 |
| `hp_log_vec3` | `...log.highp_vec3_fragment` | ARITH | HF1 | 30 |
| `hp_pow_vec3` | `...pow.highp_vec3_fragment` | ARITH | HF1 | 35 |
| `hp_exp_float` | `...exp.highp_float_fragment` | ARITH | HF1 | 13 |
| `hp_exp_vec3` | `...exp.highp_vec3_fragment` | ARITH | HF1 | 25 |
| `hp_sign_vec3` | `shaders.operator.common_functions.sign.highp_vec3_fragment` | ARITH | HF1 | 21 |
| `hp_exp2_float` | `shaders.operator.exponential.exp2.highp_float_fragment` | FORM | HF2 | 12 |
| `hp_exp2_vec2` | `...exp2.highp_vec2_fragment` | FORM | HF2 | 18 |
| `hp_mod_vec3` | `shaders.operator.common_functions.mod.highp_vec3_fragment` | FORM | HF2 | 28 |
| `hp_mod_vec3_float` | `...mod.highp_vec3_float_fragment` | LOAD | HF2 | 23 |
| `hp_smoothstep_vec3` | `...smoothstep.highp_vec3_fragment` | FORM | HF2 | 35 |
| `hp_smoothstep_float_float_vec3` | `...smoothstep.highp_float_vec3_fragment` | LOAD | HF2 | 32 |
| `pt_add_vec3`, `pt_sub_vec3`, `pt_add_assign_vec3`, `pt_sub_assign_vec3`, `pt_mul_float_vec4` | `binary_operator.{add,sub}.mediump_vec3`, `{add,sub}_assign_effect.mediump_vec3`, `mul.mediump_float_vec4` | byte-equal | PT0 | 17, 17, 20, 20, 15 |
| `pt_mul_vec3_float` | `shaders.operator.binary_operator.mul.mediump_vec3_float_fragment` | **DIFFERS** | PT3 | 13 |
| `pt_mul_assign_vec3_float` | `...mul_assign_effect.mediump_vec3_float_fragment` | **DIFFERS** | PT3 | 15 |
| `pt_add_vec3_float` | `...add.mediump_vec3_float_fragment` | OPERAND | PT1 | 13 |
| `pt_sub_float_vec4` | `...sub.mediump_float_vec4_fragment` | OPERAND | PT1 | 15 |
| `pt_mul_vec3` | `...mul.mediump_vec3_fragment` | OPERAND | PT1 | 17 |
| `pt_mul_assign_vec3` | `...mul_assign_effect.mediump_vec3_fragment` | OPERAND | PT1 | 20 |
| `pt_min_vec3`, `pt_min_vec3_float`, `pt_clamp_vec3` | `common_functions.min.mediump_{vec3,vec3_float}`, `clamp.mediump_vec3` | OPERAND | PT1 | 20, 15, 26 |
| `pt_add_vec2`, `pt_add_vec4`, `pt_add_float_vec2`, `pt_add_vec4_float` | `...add.mediump_{vec2,vec4,float_vec2,vec4_float}_fragment` | FORM | PT2 | 15, 17, 13, 14 |
| `pt_mul_vec4` | `...mul.mediump_vec4_fragment` | FORM | PT2 | 17 |
| `pt_max_vec4`, `pt_clamp_vec2_float_float` | `common_functions.max.mediump_vec4`, `clamp.mediump_vec2_float` | FORM | PT2 | 22, 19 |
| `m4_sub_float` | `shaders.matrix.sub.dynamic_mediump_mat4_float_fragment` | IrIndex | M4 | 37 |
| `m4_add_lowp_float` | `shaders.matrix.add.dynamic_lowp_mat4_float_fragment` | IrIndex | M4 | 37 |
| `m4_postinc`, `m4_postdec` | `shaders.matrix.post_{increment,decrement}.dynamic_mediump_mat4_fragment` | IrIndex | M4 | 48, 48 |
| `m4_mul_float` | `shaders.matrix.mul.dynamic_mediump_mat4_float_fragment` | AlgebraicForm | M4, then constprod K8 | 31 |
| `l3_mat3_to_mat2_fs`, `l3_mat3_to_mat4_fs` | `shaders.conversions.matrix_to_matrix.mat3_to_{mat2,mat4}_fragment` | IrIndex | L3 | 20, 41 |
| `l3_vec3x3_to_mat3_fs` | `shaders.conversions.matrix_combine.vec3_vec3_vec3_to_mat3_fragment` | IrIndex | L3 | 30 |
| `l3_float_mat3_fs` | `shaders.functions.datatypes.float_mat3_fragment` | IrIndex | L3 | 29 |
| `l3_mat3_to_mat3_vs`, `l3_float_to_mat3_vs`, `l3_vec3x3_to_mat3_vs` | `conversions.matrix_to_matrix.mat3_to_mat3_vertex`, `scalar_to_matrix.float_to_mat3_vertex`, `matrix_combine.vec3_vec3_vec3_to_mat3_vertex` | IrIndex | L3 | 30 each |

The `*_float` highp references of HF1 (`sin cos tan log2 log pow`), the
scalar HF2 ones (`mod`, `smoothstep`, highp float) and the `float_vec3` /
`vec3_float` pair trees were already pinned in `tests/mali_deqp_arith.rs`
(`deqp_arith_op_*`). They are not drawn again. The steps below name them
as their byte-compares too.

## 4. The steps

### HF1: highp built-in kernels and `sign` at fp32, narrowed at the colour (32 cases)

- **Cases.** `angle_and_trigonometry.{sin,cos,tan}.highp_*` 12,
  `exponential.{log2,log,pow,exp}.highp_*` 16, `common_functions.sign.highp_*`
  4: 26 ARITH (the vectors, `exp`, `sign`) + 6 MIXED (the float `sin cos tan
  log2 log pow`).
- **Mesa's shape** (`hp_sin_vec2`, `hp_sign_vec3`, `hp_exp_float`). The
  highp varying is loaded once as fp32 (`LD_VAR_BUF_IMM.f32...v4`). Each
  lane runs the fp32 kernel on its own word, in the order of `in0`'s
  swizzle (`v_in0.wy`: `w`'s kernel first). Sin has 9 instructions:
  `FMA.f32 x, u2.w0, u2.w1`, `FADD_IMM.f32 #0xC9400000`, `FMA.f32`,
  `FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32`, `FMA.f32 .neg`,
  `FMA.f32.clamp_m1_1`, `FADD.f32`. After every lane's kernel, each lane is
  narrowed (`FADD.f32 rN.h0, rN^, 0x0.neg`). The lanes go into the colour
  words with `MKVEC.v2i16` (a constant lane `0x0.h0` or the `u2.w1.h0` alpha),
  then the scale/bias is one `FMA.v2f16` per colour word.
  `sign` is two `FCMP_OR.f32.lt.f1` and one `FADD.f32 a, b.neg` per lane, in
  the order x, z, y for `in0 = v_in0.zxy`. **`exp(x)` is not a fragment
  kernel**: Mesa's link moves `x * log2(e)` into the vertex shader (the
  vertex NIR stores `fmul a_in0.z, 0x3fb8aa3b`, the varying is that scalar),
  so the fragment program is `exp2`'s (`MOV.i32 r1, u2.w0`, `FMA_RSCALE.f32
  x, 1.0, -0, r1`, `F32_TO_S32`, `FEXP.f32`). Its first five words equal
  `hp_exp2_float`'s. The difference is in `vs.varying`.
- **Lowering.** Extend the highp island of `fclane-lower-1` (`nir_fs.rs`
  `Fs::hp32`: a highp varying's arithmetic is kept fp32 and narrowed where
  `color` takes it; it covers `/ ceil floor min max clamp`) to the kernels.
  Train 2 landed them as 32-bit `emit.rs` rows (`(Fsin|Fcos|Flog2|Fexp2, 32,
  1)`: `fsincos`, `flog2`, `fexp2`, the shared `tan` reduction) for the
  vertex walk. HF1 uses the same rows in `hp32`. Add `Fsign` as the
  `lower_fsign` pair (as the vertex arm of arith step 2 does). Then add
  vectors: one kernel per lane in the swizzle's lane order, every narrowing
  after every kernel, and the `MKVEC` pairing of the colour words. For `exp`,
  check that `motion.rs` moves the constant product to the producer the way
  Mesa's `nir_opt_varyings` does. The `vs.varying` region of `hp_exp_*` is
  the check. If the move is not there, `exp` is its own sub-step.
- **Byte-compares.** `hp_sin_vec{2,3,4}`, `hp_cos_vec3`, `hp_tan_vec3`,
  `hp_log2_vec3`, `hp_log_vec3`, `hp_pow_vec3`, `hp_exp_float`, `hp_exp_vec3`,
  `hp_sign_vec3`, and `deqp_arith_op_{angle_{sin,cos,tan},exponential_{log2,log,pow}}_fs_highp`,
  `deqp_arith_op_common_sign_fs_highp`.
- **Forecast.** 32 cases leave `Arithmetic`/`MixedPrecision`. These shaders
  have nothing else to lower, so if the words are Mesa's, all 32 should
  compile. Owner overlap: the 6 MIXED are mixcol M3's in the arith note's
  words, and `mali-lowering-schedule.md` M2-4c (`mixprec2-lower-2`) holds the
  highp island for `sqrt fract length` and the highp binary operators. HF1
  should be that item's sibling and land after it, or with it, in one
  commit order. It must not be a second implementation of `hp32`.

### HF2: highp `exp2`, `mod`, `smoothstep` lanes in the colour (16 cases, + 2 for opform)

- **Cases.** `exp2.highp_{float,vec2}` (FORM 2),
  `mod.highp_{vec2,vec3,vec4,vec4_float}` (FORM 4),
  `mod.highp_{float,vec2_float,vec3_float}` (LOAD 3),
  `smoothstep.highp_{float,vec2,vec3,vec4,float_vec4}` (FORM 5),
  `smoothstep.highp_{float_vec2,float_vec3}` (LOAD 2). `exp2.highp_{vec3,vec4}`
  (OPERAND 2, `deqp_arith_op_exponential_exp2_fs_highp`) is opform O1's
  `exp2` row, not this step's.
- **Mesa's shape.** Everything stays fp32 up to the colour. `mod`: one
  `FRCP.f32` of the divisor lane, then `FMA.f32` quotient, `FROUND.f32.rtn`
  and `FMA.f32 y.neg, q, x` per lane. With a scalar divisor (`vec3_float`),
  the one `FRCP.f32` is shared by every lane. `smoothstep`: all `x - e0`
  first, then all `e1 - e0`, all `FRCP.f32`, all `FMA.f32.clamp_0_1`, then
  `3 - 2t` as `FMA.f32 0x40000000.neg, t, u2.w0` and two products (each stage
  over every lane before the next). `exp2`: the `FMA_RSCALE / F32_TO_S32 /
  FEXP.f32` row per lane. All three end like HF1: every lane narrowed, then
  `MKVEC` with `0x0.h0` / alpha, the scale/bias `FMA.v2f16`, and the
  `MOV.i32` word swap where the colour's first word is the second pair
  (`hp_mod_vec3`, `hp_exp2_vec2`).
- **Lowering.** The kernels are in `hp32` after HF1. What refuses now is
  their place: the `ffma` of `mod` and `smoothstep` stands outside the
  island (`hp32_op`, the reason the arith note gives for
  `op_common_{mod,smoothstep}_fs_highp`). The island must take `Ffma`, `Frcp`
  and `Fround` (rtn) of island values. The LOAD cases are a scalar operand
  shared by every lane (the `vec3_float` divisor, the `float, float` edges).
  Their load (`LD_VAR_BUF_IMM.f32.slot1.src_f32`, one word) must be read
  once, as an fp32 island leaf, not converted beside the mediump load.
- **Byte-compares.** `hp_exp2_float`, `hp_exp2_vec2`, `hp_mod_vec3`,
  `hp_mod_vec3_float`, `hp_smoothstep_vec3`, `hp_smoothstep_float_float_vec3`,
  `deqp_arith_op_common_{mod,smoothstep}_fs_highp`.
- **Forecast.** 16 cases. Needs HF1 (the island's kernels and narrowing).

### PT0: the pair trees that already compile byte-equal (28 cases, no work)

The one-lane (`float`) forms of every function, `add`/`sub` of two vec3
(and their `_assign`), and `mul.float_vec4` (opform O3's
`mul_float_vec4` landed in train 3a) are byte-equal. `pt_add_vec3`,
`pt_sub_vec3`, `pt_add_assign_vec3`, `pt_sub_assign_vec3` and
`pt_mul_float_vec4` are in `LOWERED`, so a later step that breaks them fails
`refusals_pinned`.

### PT3: `vec3 * float` compiles with other words than Mesa's (4 cases): fix first

- **Cases.** `binary_operator.mul.{lowp,mediump}_vec3_float_fragment`,
  `mul_assign_effect.{lowp,mediump}_vec3_float_fragment`. They are
  "compiled, not Mesa's", which the project rule does not allow.
- **The difference** (`pt_mul_vec3_float`, words 3-8 of 13). Mesa narrows
  the scalar `in1` first (`FADD.f32 r0.h0, r0^`), then the lone lane
  (`FADD.f32 r2.h0, r4^`). It multiplies the converted pair whole
  (`FMA.v2f16 r1, r1^, r0.h00`) and the lone lane (`FMA.v2f16 r0, r2^.h00,
  r0^.h00`), then packs `MKVEC r0, r0^.h0, r1.h0` and `MKVEC r1, r1^.h1,
  alpha`. We narrow in the other order, first pack the lone lane with the
  pair's low half (`MKVEC r2, r2^.h0, r1.h0`), multiply the pair's high half
  twice (`r1^.h11`) and the packed word whole, and take `r1^.h0` for the
  last word. By hand trace the colour values are the same lane for lane, so
  this is a word hazard, not a wrong pixel. The `*=` form
  (`pt_mul_assign_vec3_float`) has the same six differing words.
- **Lowering.** In the pair builder, a product of a converted pair by a
  splat scalar keeps the pair as one operand (Mesa's `FMA.v2f16 pair,
  s.h00`). Do not repack the lone lane with half of it first. The narrowing
  order is Mesa's: the scalar operand before the lone lane. Until that
  lands, the safe change is to refuse the shape by name. That is not this
  item's to do (no `src/` change).
- **Byte-compares.** `pt_mul_vec3_float`, `pt_mul_assign_vec3_float`
  (`DIFFERS`; move them to `LOWERED` with the fix). `add`/`sub` of the same
  form refuse (OPERAND, PT1), so the fix should be made where PT1's pair
  graph is built.

### PT1: OPERAND pair trees of vec3 results (34 cases): opform O1 and O4

- **Cases.** `add`/`sub` `.{lowp,mediump}_{float_vec3,float_vec4,vec3_float}`
  12, `mul.{lowp,mediump}_{vec3,float_vec3}` 4,
  `{add,sub}_assign_effect.*_vec3_float` 4, `mul_assign_effect.*_vec3` 2,
  `{min,max,clamp}.{lowp,mediump}_{vec3,vec3_float}` 12.
- **Mesa's shape** (`pt_add_vec3_float`, `pt_min_vec3`). The varying is
  loaded `.v4`. Its aligned pair is converted with `V2F32_TO_V2F16` and a
  lone lane is narrowed alone (`FADD.f32 r.h0`). The pair op is one
  `FADD.v2f16 r1, r1^, r0.h00` (scalar splat) or one-lane ops with
  `.h00/.h11` swizzles (`FMIN.v2f16 r1, r1^.h00, r2.h11`). The colour words
  are built with `MKVEC` (`r1^.h1, alpha`).
- **Owner.** This is opform **O1** (binary operators: "Mesa's per-read
  narrowing of lanes, pair graph from it") and **O4** (`Fmax | Fmin` with
  lane bounds, `min_vec3`, `clamp_vec3_float`) of `mali-lowering-schedule.md`
  5.2. Train 3a's commits at `4d8a0b40` are U1, U2, O2 and O3. **O1 is not
  among them**, so the arith references (`deqp_arith_op_binary_*_fs_mediump`)
  still refuse. This item adds byte-compares for O1/O4 and no step of its
  own. O4 is in train 3c. O1 should be named there too, or in train 4.
- **Byte-compares.** `pt_add_vec3_float`, `pt_sub_float_vec4`, `pt_mul_vec3`,
  `pt_mul_assign_vec3`, `pt_min_vec3`, `pt_min_vec3_float`, `pt_clamp_vec3`,
  and arith's `op_binary_{add,sub,mul}{,_assign}_fs_mediump`.
- **Forecast.** 34 cases. With PT3's fix in the same pair graph, all should
  compile.

### PT2: pair trees of vec2 and vec4 results, the colour lanes (78 cases)

- **Cases.** For each of `add sub mul`: the `vec2`, `vec4`, `vec2_float`,
  `vec4_float` and `float_vec2` forms (15 per precision). For each of the
  three `_assign_effect` and `min max clamp`: `vec2`, `vec4`, `vec2_float`
  and `vec4_float` (24 per precision). Both precisions: 78. All of them are
  `ColourForm`.
- **Mesa's shape** (`pt_add_vec2`, `pt_add_vec4`, `pt_mul_vec4`). dEQP's
  swizzles (`v_in0.wy`/`.yzwx` against `v_in1.yx`/`.wzyx`) cross the
  converted pairs, so Mesa does **not** make a two-lane op of a result pair.
  Every result lane is its own `FADD.v2f16` / `FMA.v2f16` / `FMIN.v2f16`
  with `.h00` / `.h11` operand swizzles of the converted pairs. For vec4 that
  is four one-lane ops: `FADD.v2f16 r4, r0.h11, r3.h11`, `r3, r1.h00,
  r3^.h00`, `r1, r1^.h11, r2.h11`, `r0, r0^.h00, r2^.h00`. Then
  `MKVEC.v2i16` pairs them into the colour words, and a `MOV.i32` swap when
  the first word is built second. For vec2 (`color.yz`) the words are
  `(0.0, res.x)` and `(res.y, alpha)` (`MKVEC r1, 0x0.h0, r1^.h0`; `MKVEC
  r2, r0^.h0, u2.w0.h0`). A lone lane of `in0` is narrowed alone
  (`FADD.f32 r.h0`), and only aligned pairs get `V2F32_TO_V2F16`.
- **Lowering.** `lane_tree` must accept a colour word whose two halves come
  from two one-lane pair ops (fclane's mechanism D, "fp16 vector trees over
  converted pairs with `.hN` operand swizzles and mixed word arrangements",
  for operator results rather than `rnd_vec4`/`vec3_local`). It must not
  vectorise them back into one op. The one-lane ops and the `MKVEC` pairs
  must be emitted in the order the references show. The order is not derived
  here; the byte-compare decides it.
- **Byte-compares.** `pt_add_vec2`, `pt_add_vec4`, `pt_add_float_vec2`,
  `pt_add_vec4_float`, `pt_mul_vec4`, `pt_max_vec4`, `pt_clamp_vec2_float_float`.
- **Forecast.** 78 cases. The largest step of this note. Shares the pair
  builder with PT1/PT3 and opform O1: land after O1.

### M4: `mat4_pair_unread` (8 of the 10 cases; 20 by `mali_deqp_irindex2.rs`'s count)

- **Cases.** `matrix.{add,sub}.dynamic_{lowp,mediump}_mat4_float_fragment`,
  `matrix.post_{increment,decrement}.dynamic_{lowp,mediump}_mat4_fragment`
  (8). `mali_deqp_irindex2.rs` counts 20 cases at the same `split_lanes`
  site. `deqp_irindex3_mat4_{mat4,addassign,preinc}` pin the other matrix
  forms. `mul.dynamic_*_mat4_float` (2) now refuse with `AlgebraicForm`.
- **Mesa's shape** (`m4_postinc`, `m4_sub_float`). Three or four `.v4`
  loads. Each column's aligned pairs are converted (`V2F32_TO_V2F16`) and
  the swizzle's lone lanes narrowed (`FADD.f32 r.h0`). `++` is
  `FADD_IMM.v2f16 #0x3C003C00` on converted pairs and `FADD.v2f16 x.h00,
  0x3C000000.h11` on lone lanes. The column-sum reduction
  `res[0].xyz + res[1].yzw + ...` is one-lane `FADD.v2f16` with `.h00`/`.h11`
  swizzles, packed with `MKVEC`. This is the same one-lane form as PT2.
- **Lowering.** `mali-deqp-irindex.md` 5.2: in `split_lanes`, a lane with no
  `lane_of` entry answers "not split" (`Ok(false)`) instead of `?`. After
  that the sum meets the PT2 one-lane trees, so expect `ColourForm` or the
  PT2 form next. Check the words against these references rather than
  assume a compile.
- **Byte-compares.** `m4_sub_float`, `m4_add_lowp_float`, `m4_postinc`,
  `m4_postdec`, `deqp_irindex_mat4_pair_unread`, `deqp_irindex3_*`.
  `m4_mul_float` stays refused (`AlgebraicForm`, constprod K8) and is pinned
  so that K8 has a matrix byte-compare.
- **Forecast.** 8 cases leave `Defect(IrIndex)`. How many compile depends
  on PT2.

### L3: `load_mat3` (18 cases)

- **Cases.** `mali-deqp-irindex.md` section 4: `conversions.matrix_combine` 7,
  `matrix_to_matrix` 6, `scalar_to_matrix` 3, `functions.datatypes.float_mat3_fragment`,
  `linkage.varying_type_mat3`. **11 of the 18 are `_vertex` cases**, and
  their fragment shader is the one the shader library generates for a
  `mat3` output (`l3_*_vs`: `varying mat3 out0`, `isOk(out0, ref_out0, 0.05)`).
  That shader is the same for every `*_to_mat3_vertex` case, so
  `l3_mat3_to_mat3_vs` stands for all 11.
- **Mesa's shape** (`l3_mat3_to_mat3_vs`, `l3_mat3_to_mat2_fs`). The `mat3`
  varying is packed in three loads: `.v4`, `.v4`, then one scalar
  (`LD_VAR_BUF_IMM.f32.slot2.src_f32 @r3`). The loads are interleaved with
  the compare: column pairs `V2F32_TO_V2F16`, `FADD.v2f16 col, ref.neg`,
  `FMAX.v2f16 .abs` per pair, and the second load is issued only after the
  first column's differences. Then the `FMAX` reduction, one
  `FCMP_OR.v2f16.ge.f1` against `eps`, `IADD.v2u16 r0.h00` and the `MKVEC`
  alpha word (fclane mechanism C's broadcast).
- **Lowering.** `mali-deqp-irindex.md` 5.3: emit a varying's load where
  `src32` (or the fp16 reader) first reads a lane whose raw value is not
  there yet, in Mesa's place: after the reads of the previous load's lanes,
  as the references show. The scalar third load (the compacted
  `mat3[2].z`, plus a vec3's lone lane) is a one-word load.
- **Byte-compares.** `l3_*` (7) and `deqp_irindex_load_mat3`.
- **Forecast.** 18 cases leave `Defect(IrIndex)`. 11 of them share one
  fragment shader, so one byte-equal `l3_mat3_to_mat3_vs` lifts most of
  them. Each case's vertex shader (the conversion, fp32) is not pinned
  here.

## 5. Order and forecast

| Order | Step | Cases (first refusal) | Text left | Needs | Item |
|--:|---|--:|---|---|---|
| 1 | PT3 | 4 (compiled, differ) | none (a hazard) | - | train 4 (or a by-name refusal first) |
| 2 | L3 | 18 | `Defect(IrIndex)` | - | train 4 |
| 3 | M4 | 8 | `Defect(IrIndex)` | best after PT2 (the next form) | train 4 |
| 4 | HF1 | 32 | `Arithmetic` 26, `MixedPrecision` 6 | `mixprec2-lower-2` (M2-4c, the island) | train 4, after train 3c |
| 5 | HF2 | 16 | `ColourForm` 11, `FsVaryingLoad` 5 | HF1 | train 4 |
| - | PT1 | 34 | `FsOperandForm` | - | opform O1 / O4 (train 3c / 4) |
| 6 | PT2 | 78 | `ColourForm` | O1 (the pair builder) | train 4 |

Sum 4 + 18 + 8 + 32 + 16 + 34 + 78 = **190 must-pass cases** HOST-ONLY. 156
are this note's own steps; the 34 of PT1 belong to opform. By text:
`ColourForm` 89 (PT2 78, HF2 11), `FsOperandForm` 34 (+2 `exp2` highp),
`Arithmetic` 26, `Defect(IrIndex)` 26, `MixedPrecision` 6, `FsVaryingLoad` 5. A
case only passes on a board once every word is Mesa's. This is a forecast
of first refusals that move, not of passes.

## 6. Risks and files

- All steps edit `src/shaderc/online/mali/nir_fs.rs` (the pair builder,
  `lane_tree`, `hp32`, `split_lanes`, `src32`). PT2 and PT3 touch the same
  vectoriser as fix C (`Defect(LaneCycle)`, `mali-deqp-defects.md`) and
  opform O1-O8. Serialise them in one train.
- Tests at risk: `tests/mali_fs_arith.rs`, `mali_fragment_random.rs`,
  `mali_glmark2_corpus.rs` (pair registers, `MKVEC.v2i16` order),
  `mali_deqp_arith.rs`, `mali_deqp_opform.rs`, `mali_deqp_mixprec*.rs`,
  `mali_deqp_irindex*.rs`, and this file's `LOWERED` five.
- `tests/mali_deqp_highpfs.rs` is not in a `tools/check.sh` row. The
  `irindex-fix` item, in review, owns the rule that runs every passing
  `tests/mali_deqp_*.rs` in the Mali family row.

## 7. What this note does not know

- The `file:line` site of each refusal was not instrumented. The texts are
  measured. The sites quoted are the irindex and arith notes', at their
  tips.
- The 4 DIFFERS were checked for equal colour values by hand trace, not on a
  board.
- The vertex shaders of the 11 `load_mat3` `_vertex` cases were not drawn,
  and `linkage.varying_type_mat3` (a linkage-template pair) was not drawn.
- Whether `motion.rs` already moves `exp`'s `fmul` by `log2(e)` into the
  vertex shader is unchecked (HF1).
