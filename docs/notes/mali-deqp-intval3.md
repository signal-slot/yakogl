<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the integer/boolean refusal at `3497799a`: shapes, Mesa references, lowering order

**HOST-ONLY.** Plan #387 item `intval3-ref` (issue #7, ROADMAP stage 5). This
is a study with references. It makes no compiler change. Every case count in
this note comes from `deqp-gles2` run on a host against the Mali back end's C
pair on the Linux OSAL (`tools/mali/deqp-host-measure.sh`; the method and its
limits are in `mali-deqp-families.md` sections 1 to 3). What is real is each
shader's compile and link outcome and our info log, never a render verdict.
A board status quoted here (Fail / NotSupported) is the NanoPC-T6's own, from
the committed slices.

The text is `Valhall: an integer or boolean value is not lowered yet`
(`MaliError::IntValue`). This note continues `mali-deqp-intval.md` (sections
5 to 9, the 25 `deqp_intval_*` references, steps 1 and 2, intval-reslib). It
adds 22 references, `tools/mali/testdata/deqp_intval3_*`, for the forms that
still refuse at the tip and that no reference covers. They are pinned by
`tests/mali_deqp_intval3.rs`.

## 1. Method

- Tip **`3497799a`** (`3497799afc14956408857ab0d9a68928bf15eb9d`). CTS
  VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, cloned into
  `$CONNECTED_DEPS_DIR/VK-GL-CTS` the way `tools/cts/deps.toml` (`DEQP_WORK`)
  does it: `git clone`, `checkout --detach`, `python3
  external/fetch_sources.py`. `tools/cts/cts-yakogl-target.patch` is applied
  with `git apply` and left uncommitted there.
- Measure. On 4 cores the run took about 10 minutes, after the host
  `deqp-gles2` and Mali-pair builds under `build/host-deqp-mali/`:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$CONNECTED_DEPS_DIR/intval3-scratch/measure"      # --jobs 3 --chunk 100
```

  It ran the 10,597 refused cases of the four committed slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608`. Of these, 6,132
  have a host refusal at the tip. **206 cases have this text as their first
  refusal** (`measure.tsv` column `host_family` = `intval`). All 206 are
  fragment-stage refusals: 184 board Fail and 22 NotSupported. In every one
  of them the partner shader (the case's vertex shader) compiles. The
  measure's "next refusal (partner shader)" column is `-` for all 206.
  What the refused fragment shader itself would hit after this text is not
  measurable until the text is lowered.
- The shapes are the script's `RULES['intval']` names. The sub-forms below
  split them by the GLSL form, and for `random_mixed` by what Mesa's program
  keeps (section 2). The sub-form counts were made with a scratch script and
  scratch draws of all 39 `random_mixed` pairs, none of it committed. They
  sum to the shape counts.
- Mesa 26.2.1's drm-shim was built the documented way into
  `$CONNECTED_DEPS_DIR/mesa-shim`:
  `tools/mali/drm_shim_ref.sh build "$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz" "$CONNECTED_DEPS_DIR/mesa-shim"`.
  The tarball's sha256 is `c47e81bd...a4442`, the one in
  `third_party/mesa-26.2.1/UPSTREAM`. As a check, the build redrew
  `deqp_intval_rsg_cond_fs`: its `expect.json` is byte-identical to the
  committed one apart from the trace file name.
- Each reference is the case's pair verbatim from its `.qpa` (the
  measure's `src/<case>.{vert,frag}`), made with:

```sh
MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
    <case>.vert <case>.frag tools/mali/testdata/deqp_intval3_<name> --instrument shim
```

  This runs `drm_shim_ref.sh run <work> vs fs <prefix> --uniform-values`
  (`gl_ref.py` draws once, 64x64 triangles, through Mesa's panfrost GLES 2
  driver on the G610 shim; every draw ended with `glGetError 0x0`), then
  `pan_ref_diff.py extract <trace> --draw`. gl_ref.py's uniform values: an
  `int` is an integer (16 is `00000010`), and a true `bool` is the word
  `ffffffff`. All 22 were drawn twice, and the two sets of `expect.json` are
  byte-identical. The G310 needs no reference of its own
  (`mali-g310-vs-g610.md`).
- `tests/mali_deqp_intval3.rs` pins them. At the tip **all 22 are refused
  with exactly this text** (`refusals_pinned`), so every byte-compare is
  `#[ignore]`d. `references_parse` checks the files and the fragment word
  counts quoted in section 3.

Since the ranked table at `9ac31f30` (`mali-deqp-families.md` section 18:
271 cases), intval-reslib (`mali-deqp-intval.md` section 9) took the 64
`_vertex` `int_uniform` cases, and one `random_mixed` case moved to another
text. The shapes `harness_isok`, `bvec_relational`, `float_compare` and
`int_loop_counter` of the older table (`mali-deqp-families.md` section 4)
have **no** case with this first refusal at the tip.

## 2. The shapes at `3497799a`

`Ref` is `tools/mali/testdata/deqp_intval3_<ref>.*`. All the cases are
fragment-stage refusals (VS / FS = 0 / n), and the partner compiles in each.

| Shape | Cases | Fail / NS | VS / FS | Example case (`dEQP-GLES2.functional.`) | Next refusal (partner) |
|---|--:|---|---|---|---|
| `ivec_operator` | 72 | 72 / 0 | 0 / 72 | `shaders.operator.binary_operator.not_equal.highp_int_fragment` | none (72 compile) |
| `random_mixed` | 39 | 39 / 0 | 0 / 39 | `shaders.random.comparison_ops.fragment.0` | none (39) |
| `int_uniform` | 37 | 15 / 22 | 0 / 37 | `shaders.indexing.matrix_subscript.mat4_dynamic_write_dynamic_loop_read_fragment` | none (37) |
| `bool_uniform` | 31 | 31 / 0 | 0 / 31 | `shaders.conversions.scalar_to_scalar.int_to_bool_fragment` | none (31) |
| `step_sign` | 21 | 21 / 0 | 0 / 21 | `shaders.operator.common_functions.step.highp_float_fragment` | none (21) |
| `bool_literal` | 2 | 2 / 0 | 0 / 2 | `shaders.conditionals.if.complex_condition_fragment` | none (2) |
| `bool_logic` | 2 | 2 / 0 | 0 / 2 | `shaders.conditionals.if.compound_statement_fragment` | none (2) |
| `scalar_conversion` | 2 | 2 / 0 | 0 / 2 | `shaders.operator.binary_operator.logical_and.bool_fragment` | none (2) |
| **total** | **206** | 184 / 22 | 0 / 206 | | |

The sub-forms, each with its reference:

| Shape | Sub-form (GLSL) | Cases | Groups | Ref |
|---|---|--:|---|---|
| `ivec_operator` | `--x` / `x--` of `int`/`ivecN` (pre/post, effect/result, 3 precisions) | 48 | `unary_operator.{pre,post}_decrement_{effect,result}` | `opfs_pre_decrement_result_int_mediump`, `opfs_post_decrement_effect_ivec2_highp` |
| | `a != b` of two run-time `int`/`ivecN` (to `bool`) | 12 | `binary_operator.not_equal` | `opfs_not_equal_int_highp` |
| | `notEqual(ivecN, ivecN)` to `vecN(bvecN)` | 9 | `int_compare.notEqual` | `opfs_notequal_ivec3_mediump` |
| | `(a && b, a, ivec2(vec2(a) + v))`: `vec2(bool)`, an fp16 sum to `ivec2` | 3 | `sequence.no_side_effects.*_bool_vec2` | `opfs_sequence_bool_vec2_mediump` |
| `random_mixed` | every int/bool value dead or folded: Mesa keeps no integer op of the shader's | 27 | random 26, `fragment_ops.interaction` 1 | `rsg_swizzle12_fs` (dead globals), `rsg_condcomb80_fs` (`d != d` folds) |
| | an int value read as a float (`vec4(.., u_int)`, `exp2(float(u_int))`) | 4 | `scalar_conversion` 2, `texture` 1, `exponential` 1 | `rsg_sconv55_fs`, `rsg_exp76_fs` |
| | `bool(float)` of a uniform or varying, as a colour lane | 3 | `scalar_conversion` 1, `texture` 1, `conditionals` 1 | `rsg_sconv72_fs` |
| | run-time int/bool compares and selects (`if (ivec != ivec)`, bvec of compares) | 5 | `conditionals` 3, `trigonometric` 2 | `rsg_cond12_fs` |
| `int_uniform` | a local array or matrix written at a `uniform int` index | 22 | `indexing.tmp_array` 16, `indexing.matrix_subscript` 4, `struct.*.nested_struct_array_dynamic_index` 2 (all NS on the board) | `idx_tmp_vec3_dynwrite_fs` |
| | int `isOk` of a function's `int` result (`out`/`inout int`, int array arguments, argument order) | 9 | `functions.qualifiers` 6, `functions.array_arguments` 2, `functions.misc` 1 | `reslib_inout_int_fs` |
| | int `isOk` of a constant `_vertex` output (Mesa's link moves it into the FAU) | 6 | `constants.int_0..4_vertex` 5, `constant_expressions.trivial.int_vertex` 1 | `reslib_const_int_vs` |
| `bool_uniform` | `bool(int)` / `bvecN(ivecM)` compared with a `uniform bool`/`bvecN` | 25 | `conversions.{scalar_to_scalar,scalar_to_vector,vector_to_scalar,vector_to_vector,vector_combine}` | `reslib_int_to_bool_fs`, `reslib_ivec4_to_bvec4_fs` |
| | a `bool` through a function (`!a`, `in bool`, bool array arguments, overloads) compared with a `uniform bool` | 6 | `functions.{datatypes,qualifiers,array_arguments,overloading}` | `reslib_func_bool_fs` |
| `step_sign` | `step(edge, x)` at mediump / lowp | 14 | `common_functions.step.{mediump,lowp}_*` | `opfs_step_float_vec2_mediump` |
| | `step(edge, x)` at highp | 7 | `common_functions.step.highp_*` | `opfs_step_vec3_highp` |
| `scalar_conversion` | `a && b` / `a \|\| b` of two `> 0.0` compares to `float(bool)` | 2 | `binary_operator.logical_{and,or}.bool` | `opfs_logical_and_bool` |
| `bool_literal` | `if (false \|\| a && b)`, `if (false, a)`: a literal folded into the condition | 2 | `conditionals.if` | `cond_complex_fs` |
| `bool_logic` | a one-armed `if` on an fp16 compare writing two outputs, `RES && isOk` | 2 | `conditionals.if` | `cond_compound_fs` |

These are the `random_mixed` members by sub-form, from Mesa's fragment
program in the scratch draws:

- dead or folded (27): `comparison_ops.fragment.{0,6,27}`,
  `conditionals.combined.80`, `conditionals.fragment.{31,33,66,70,78,80}`,
  `scalar_conversion.combined.32`, `scalar_conversion.fragment.{0,6,32,62}`,
  `swizzle.fragment.{12,37,38,45}`, `texture.fragment.{6,11,65,102,106,120}`,
  `trigonometric.fragment.70`, `fragment_ops.interaction.basic_shader.85`.
  `texture.fragment.106`'s `MUX.i32.bit` is the cube lowering's, not an int
  value.
- int as float (4): `scalar_conversion.fragment.{55,58}`,
  `texture.fragment.38`, `exponential.fragment.76`.
- `bool(float)` (3): `scalar_conversion.fragment.72`,
  `texture.fragment.72` (the same source), `conditionals.fragment.37`.
- run-time compare and select (5): `conditionals.combined.6`,
  `conditionals.fragment.{6,12}`, `trigonometric.fragment.{31,76}`.

Two references share a shader with a committed file:
`reslib_const_int_vs.frag` is `deqp_intval_reslib_int_vs.frag`. The pair
differs, though: its vertex shader writes a constant, and Mesa's program
differs (section 3). `cond_compound_fs` is the pair
`deqp_shapes/intval_bool_logic`, which had no reference until now.

## 3. Mesa 26.2.1's words, per reference

Only `fs.fragment` is quoted. The vertex programs are the usual position and
varying stores, and the tip already compiles every one of them. The tail
common to every fragment program (`ATEST`, `BLEND ... .end`, `IADD_IMM.i32
r48`, `BRANCHZI`) is left out. `uN.wK` is a FAU word. "fs words" is the
region's word count, as `FS_WORDS` in the test has it. What follows is the
lowering each one needs, in IR terms.

### ivec_operator

- **`opfs_not_equal_int_highp`** (11 fs words):
  `F32_TO_S32.rtz` of each fp32 lane, `ICMP_OR.u32.ne.m1 r0, a, b, 0x0`, then
  `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0.h00` for `float(bool)`, then
  `U16_TO_U32` into the colour. Needed: `ine` of two run-time ints (the
  "gap probe" of `mali-deqp-intval.md` section 7). An `ine` compare is
  emitted **`.u32`**, not `.s32` (equality has no sign). The 32-bit mask
  goes straight into the 16-bit `MUX` (`.h00`), as `reslib_int_fs` does it.
- **`opfs_notequal_ivec3_mediump`** (20): the same per lane (three
  `ICMP_OR.u32.ne.m1` and three `MUX.v2i16`), then `MKVEC.v2i16` pairs. The
  constant lane 1.0 is the FAU word `3c003c00` (`u2.w0.h0`). The mediump ints
  stay 32-bit: `F32_TO_S32.rtz` of the fp32 loads, with no fp16 narrowing
  first.
- **`opfs_pre_decrement_result_int_mediump`** (13): `F32_TO_S32.rtz`,
  `IADD_IMM.i32 r0, r0, #0xFFFFFFFF`, `S32_TO_F32`, `FADD.f32 r0.h0, r0, 0x0.neg`
  (the narrowing to fp16), then the scale and bias `FMA.v2f16 ... 0x2E66.h11,
  0x3800.h11`. Needed: `iadd x, -1` as `IADD_IMM` with the immediate
  `0xFFFFFFFF`. `opvs_post_increment_int` has `#0x1`, and the walk refuses
  the negative constant.
- **`opfs_post_decrement_effect_ivec2_highp`** (17): the same per lane: two
  `IADD_IMM.i32 #0xFFFFFFFF`, two `S32_TO_F32`, and `MKVEC.v2i16` of the
  narrowed lanes before the `FMA.v2f16`.
- **`opfs_sequence_bool_vec2_mediump`** (25): `vec2(in0)` of a bool is one
  `FCMP_OR.v2f16.lt.f1 r0, 0x0, x.h00` (a 1.0/0.0 result). The fp16 sums
  `FADD.v2f16` then go `F16_TO_S32.rtz` per lane, `S32_TO_F32`, `FADD.f32
  .h0` narrowing, `MKVEC`, `FMA.v2f16 * 0.5`. The `in0 && in1` of the
  sequence is dead. Needed: `f2i32` of an fp16 value (`F16_TO_S32.rtz`, as
  `reslib_mixed_fs` has it) and `b2f16` as the `.f1` compare.

### random_mixed

- **`rsg_swizzle12_fs`** (11): no integer op. The global `ivec2 e` and every
  bool are dead or fold. What is left is the varying `k` and FAU constants
  (`MKVEC.v2i16`, `FMA.v2f16`, `FADD_IMM.v2f16`). Needed: drop the dead int
  and bool globals and their stores, and fold the constant int/bool
  expressions **before** the walk refuses an int-typed value. This takes the
  27 dead/folded cases.
- **`rsg_condcomb80_fs`** (7): `if (d != d)` of a `uniform ivec4` folds to
  false (`ine a, a` -> false), and the program is constant colour words from
  the FAU. There is no varying program: Mesa's link drops the unread `e`.
- **`rsg_sconv55_fs`** (9): `S32_TO_F32 r0, u2.w0` of the `uniform int c`,
  then `FMA.f32` with a FAU constant and the `FADD.f32 .h0` narrowing. The
  `bvec4 a` is dead. Needed: `i2f32` of an int uniform's FAU word (fp32,
  since lower_precision lowers no conversion from an integer), as
  `rsg_cond_fs` has it.
- **`rsg_exp76_fs`** (27): `exp2(float(c))` of a `uniform int` is
  `LDEXP.f32 r2, 0x3F800000, u2.w0`. Mesa's `exp2(i2f(x))` -> `ldexp(1.0, x)`
  has no `S32_TO_F32`. `int g = int(i)` is dead. The `F32_TO_S32` /
  `FEXP.f32` / `FMA_RSCALE` are `exp2`'s own lowering (the arith family).
  Needed: that algebraic rule and `LDEXP.f32` (a new op for the walk).
- **`rsg_sconv72_fs`** (7): `vec4(bool(b), ...)` of a `uniform float` is one
  `FCMP_OR.v2f16.ne.f1 r0, u2.w0.h00, 0x0` (the uniform read as fp16, with a
  1.0/0.0 result). The `bvec2(...)` statement is dead.
- **`rsg_cond12_fs`** (31): `if (m - l != k + ivec2(j))` of a `uniform
  ivec2` with constant `l`, `k`, `j` becomes `IADD_IMM.i32 r1, u3.w0,
  #0xFFFFFFFD` and `ICMP_OR.u32.ne.m1 r1, r1, 0x3020100.b2, 0x0`, then
  `IADD_IMM.i32 r2, u3.w1, #0x5` and `ICMP_OR.u32.ne.m1 r1, r2, 0xFFFFFFFF,
  r1` (the `any(!=)` chained through the third source, `.u32`). Then come
  the four colour lanes as `CSEL.u32.eq r, mask, 0x0, else, then` at fp32,
  after `F16_TO_F32` of the fp16 constants, and the `FADD.f32 .h0`
  narrowing. The `else if (d < ...)` folds (`d` and the bound are
  constant). Needed: `ICMP_OR` with an inline byte-lane constant
  (`0x3020100.b2` is 2), the `_OR` chaining of `any(ine)`, and a 32-bit
  `CSEL` select of colour lanes.

### int_uniform

- **`idx_tmp_vec3_dynwrite_fs`** (77): `arr[ui_k] = v` of a local `vec3
  arr[4]` at `uniform mediump int` indices is fully unrolled, with no
  scratch memory. Per index there are three `ICMP_OR.s32.lt.m1 r, u2.wK,
  <lane const>, r1` against 1, 2 and 3 (`0x1000000.b3`, `0x3020100.b2`,
  `0x3020100.b3`) and the masks combined (`LSHIFT_OR.i32 ... .not`,
  `LSHIFT_AND.i32`) into one "index == e" mask per element. Each element's
  value is then picked with `MUX.v2i16 ... rM.h00` (fp16 lanes) or `IADD.v2u16
  .h00` + `CSEL.v2u16.eq` (an fp16 pair). Needed: `ilt` of an int uniform
  with a constant (`.s32`), `inot`/`iand`/`ior` of 32-bit masks
  (`LSHIFT_*.i32`), a 32-bit mask into a 16-bit select, and the walk's
  handling of a local array written at a run-time index (Mesa's
  `nir_lower_indirect_derefs` to a `bcsel` chain). This takes 22 cases, all
  NotSupported on the board.
- **`reslib_inout_int_fs`** (15): `in0 = int(v_in0 * 1.0025)` is `FADD.f32
  r0.h0` narrowing, `FMA.v2f16 r0, r0.h00, u2.w1` (the 1.0025 as fp16
  `3c03`), then `F16_TO_S32.rtz`. `f * in0` with `f = -1` folds to
  `ISUB.s32 r0, 0x0, r0` (`ineg`). `isOk` is `ICMP_OR.u32.eq.m1 r0, r0,
  u2.w0, 0x0` against `ref_out0`, then `MUX.v2i16` into `vec4(RES, RES, RES,
  1.0)` (`IADD.v2u16 .h00`, `MKVEC.v2i16 ... u3.w0.h0`). Needed: `ineg` as
  `ISUB.s32 0, x`. The rest is `reslib_int_fs`'s compare.
- **`reslib_const_int_vs`** (14): the fragment shader is the one of
  `deqp_intval_reslib_int_vs`, but the vertex shader's `v_out0` is a
  constant, so Mesa's link moves it into the FAU (`u2.w1` = `42f70000`) and
  there is no varying program. The words are `S32_TO_F32 r0, u2.w0`,
  `IADD_IMM.i32 r1, u2.w0, #0x1`, `S32_TO_F32`, `FCMP_OR.f32.ge.m1 r1, r1,
  u2.w1, 0x0`, `FCMP_AND.f32.ge.m1 r0, u2.w1, r0, r1`, then `MUX.v2i16`.
  The fp32 compare's other side is the **constant varying as a FAU word**,
  not a widened fp16 local, and that form is refused today
  (`mali-deqp-intval.md` section 9, "A constant on the other side is
  refused"). `mali_ref` runs the GL core's link, which may not move the
  constant (the module comment of `tests/mali_ref`). The lowering may need
  the unit tests in `src/shaderc/online/mali/tests.rs`.

### bool_uniform

- **`reslib_int_to_bool_fs`** (15): `F16_TO_S32.rtz` as above, `bool(in0)` is
  `ICMP_OR.u32.ne.m1 r0, r0, 0x0, 0x0`, and `a == b` against a `uniform
  bool` is `ICMP_OR.u32.eq.m1 r0, r0, u2.w0, 0x0` (the bool word
  `ffffffff`). Then `MUX.v2i16`. Needed: `i2b` (`ine x, 0`) and `ieq` of two
  32-bit booleans, one of them a bool uniform's FAU word.
- **`reslib_ivec4_to_bvec4_fs`** (28): `V2F32_TO_V2F16` pairs, `FMA.v2f16`
  by 1.0025, four `F16_TO_S32.rtz` (`.h0` / `.h1`), four `ICMP_OR.u32.ne.m1
  x, 0x0`, then `bvec4 == bvec4`. The lanes are compared one by one
  (`ICMP_OR.u32.eq.m1` against `u2.w0`, `u2.w1`, `u3.w0`, `u3.w1`). The
  first `&&` is fused as `ICMP_AND.u32.eq.m1 r1, r1, u3.w1, r3` and the rest
  are `LSHIFT_AND.i32`. Needed: `ICMP_AND.u32` (`fuse_cmp`'s integer AND
  form, the counterpart of `FCMP_AND.f32`) and the order of the `all()`
  reduction.
- **`reslib_func_bool_fs`** (13): `bool(v_in0)` then `!a` folds to one
  `FCMP_OR.v2f16.eq.m1 r0, r0.h00, 0x0, 0x0` (`!(x != 0)`). `== ref_out0`
  is **16-bit**: `ICMP_OR.v2u16.eq.m1 r0, r0.h00, u2.w0.h00, 0x0`, the
  bool uniform's word read as two halves. A bool compared with an fp16
  mask stays 16-bit, unlike `reslib_int_to_bool_fs`, where the mask is
  32-bit. Needed: the 16/32 choice for a bool uniform by the width of the
  other side (`Fs::mask_op`'s 16/32 mix, `mali-deqp-intval.md` section 8).

### step_sign

- **`opfs_step_float_vec2_mediump`** (16): `step(e, x)` of fp16 values is
  one `FCMP_OR.v2f16.ge.f1 r0, x.h10, e.h00, 0x0`, a 1.0/0.0 result in both
  halves, then `MKVEC` and the scale/bias `FMA.v2f16`. `V2F32_TO_V2F16`
  narrows the vec2 varying. The lowp cases are presumed to give the same
  program (lowp is mediump on Mali). Needed: `step` as `b2f16(fge)` in the
  `.f1` form (`opfs_not` has `.f1` for `float(!(x > 0))`).
- **`opfs_step_vec3_highp`** (16): at highp, `FCMP_OR.f32.ge.m1` per lane and
  `MUX.v2i16 0x0, 0x3C00.h11, mask.h00` (`b2f16` of the 32-bit mask into the
  fp16 colour). Needed: an fp32 compare whose mask turns into an fp16 1.0/0.0.

### scalar_conversion, bool_literal, bool_logic

- **`opfs_logical_and_bool`** (12): `(a > 0.0) && (b > 0.0)` is `FMIN.v2f16`
  and one `FCMP_OR.v2f16.lt.f1 r0, 0x0, min.h00` (the same rewrite as
  `opvs_logical_and`, in the fragment stage, with `.f1` since the result is
  `float(res)`). `logical_or` is presumably `FMAX.v2f16` (not drawn).
  Needed: `iand(flt(0,a), flt(0,b))` -> `flt(0, fmin(a,b))` and
  `ior` -> `fmax` (nir_opt_algebraic), then `b2f16`.
- **`cond_compound_fs`** (22): `if (in0 >= 1.0) { out0 = 1.0; out1 = 0.0; }`
  is `FCMP_OR.v2f16.ge.m1 r0, in0.h00, 0x3C00.h11` and two `MUX.v2i16`
  selects (`out1` = `MUX 0x3C00, 0x0, m`, `out0` = `MUX 0x0, 0x3C00, m`).
  Each `isOk(a, b, 0.05)` is `FADD.v2f16 a - u`, `FADD.v2f16 |u|`,
  `FMA.v2f16 eps * |u| + eps` (`u3.w0` = `2a66`), and `FCMP_OR.v2f16.ge.m1
  ..., |a-u|`. `RES && isOk` is `LSHIFT_AND.v2i16`. The colour is `MUX`,
  `IADD_IMM.i32 #0x3C003C00`, `MKVEC.v2i16`, `IADD.v2u16 .h00`. Needed: the
  flattened one-armed `if` with a bool mask selecting two constants, and the
  16-bit `&&` of two `isOk` masks (`LSHIFT_AND.v2i16`, as `reslib_float_fs`).
- **`cond_complex_fs`** (25): `false || (in0 >= 1.0) && (in0 - 2.0*in0 <
  0.0)` drops the `false`. `2.0*in0` is `FADD.v2f16 r1, in0, in0`, and the
  second compare is `FCMP_OR.v2f16.lt.m1 r1, in0.h00, r1.h00`
  (`in0 - 2in0 < 0` -> `in0 < 2in0`, an algebraic rewrite). The two masks
  are joined by `LSHIFT_AND.v2i16`, then follows the `cond_compound_fs`
  tail. Needed: the literal-`false` fold and that compare rewrite.

## 4. Proposed lowering order for `intval-rest` (by cases unblocked)

The counts are first-refusal cases at `3497799a`. Each step names the
references it should flip, from `REFUSED` to `LOWERED`. Whether a case then
passes depends on what the shader hits next, and the measure cannot see that
yet (section 1). Re-measure with `tools/mali/deqp-host-measure.sh --filter
<group>` after each step.

1. **Integer operator gaps (72 cases, `shaders.operator`)**: `iadd x, -1` as
   `IADD_IMM #0xFFFFFFFF` (48), `ine` of two run-time ints as
   `ICMP_OR.u32.ne` (21), and `f2i32` of an fp16 sum with `b2f16` as `.f1`
   (3). Refs: `opfs_pre_decrement_result_int_mediump`,
   `opfs_post_decrement_effect_ivec2_highp`, `opfs_not_equal_int_highp`,
   `opfs_notequal_ivec3_mediump`, `opfs_sequence_bool_vec2_mediump`. These
   are small changes to the step-1 walk and the biggest single block.
2. **Dead and folded int/bool values (27 of `random_mixed`, plus 3 + 4 of
   its other sub-forms = 34)**: drop dead int/bool globals, locals and
   statements and fold the constant ones before the refusal. After that,
   `bool(float)` as `FCMP_OR.v2f16.ne.f1` (3) and `i2f32` of an int uniform,
   `S32_TO_F32` / `LDEXP` (4). Refs: `rsg_swizzle12_fs`, `rsg_condcomb80_fs`,
   `rsg_sconv72_fs`, `rsg_sconv55_fs`, `rsg_exp76_fs`. Random shaders often
   hit another family next (`mali-deqp-families.md` section 18), so the
   forecast must be measured.
3. **Integer-to-bool conversions against bool uniforms (31 cases,
   `shaders.conversions` 25, `shaders.functions` 6)**: `ine x, 0`, `ieq` of
   32-bit booleans against a bool FAU word, `ICMP_AND.u32` fusing, and the
   16-bit bool-uniform compare (`ICMP_OR.v2u16 ... u.h00`) beside an fp16
   mask. Refs: `reslib_int_to_bool_fs`, `reslib_ivec4_to_bvec4_fs`,
   `reslib_func_bool_fs`.
4. **Dynamic-index writes of local arrays and matrices (22, all NS on the
   board)**: the `bcsel` chain of `nir_lower_indirect_derefs` with
   `ICMP_OR.s32.lt` against byte-lane constants and 32-to-16-bit mask
   selects. Ref: `idx_tmp_vec3_dynwrite_fs`. This is the largest program
   (77 words) and may belong with the uniarray or loops items; the
   matrix_subscript and struct members have no reference of their own.
5. **`step` (21, `shaders.operator.common_functions.step`)**: `b2f(fge)` as
   `FCMP_OR.v2f16.ge.f1` (mediump/lowp, 14) and `FCMP_OR.f32.ge.m1` +
   `MUX.v2i16` (highp, 7). Refs: `opfs_step_float_vec2_mediump`,
   `opfs_step_vec3_highp`. This is small. It could come before step 4 for
   cost, at one case fewer.
6. **Functions' int results (9)**: `ineg` as `ISUB.s32 0, x` and the
   argument-order and array-argument forms. Ref: `reslib_inout_int_fs`.
7. **Run-time int/bool selects in random shaders (5)**: `rsg_cond12_fs`
   (`ICMP_OR` chaining, 32-bit `CSEL` of colour lanes).
8. **Constant `_vertex` int outputs (6)**: an fp32 compare against a FAU
   constant that the link moved. Ref: `reslib_const_int_vs`. It may not be
   reachable through `mali_ref` (section 3).
9. **`if` and `&&` of fp16 compares (6)**: `opfs_logical_and_bool` (`fmin`
   rewrite, 2), `cond_compound_fs` and `cond_complex_fs` (4).

Steps 1 to 3 together are 137 of the 206 cases. No single sub-form reaches
the plan's threshold of 100 blocked cases. The family as a whole (206) does,
so the plan is not done with it until steps 1 to 3 at least have landed.

## 5. Caveats

- HOST-ONLY: these are compile outcomes, not renders. The 22 NotSupported
  (`int_uniform` dynamic writes) are NS on the board for reasons the host
  cannot see.
- The `random_mixed` sub-forms come from Mesa's program in scratch draws
  of all 39 pairs (not committed). A case counted as dead or folded may
  still need a form no reference here has, once its dead code is dropped.
- `logical_or` and the lowp `step` cases are presumed to follow their
  drawn neighbours. They were not drawn.
