<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): fragment-shader integer/boolean refusals at `8ff01055`: shapes and Mesa references

**HOST-ONLY.** Plan #402 item `ref-fsint` (issue #20, ROADMAP stage 5). This
is a study with references. It makes no compiler change. The case lists and
board statuses come from the NanoPC-T6 slices
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`. Every
statement about the tip (`e65567ea`) is a host compile, never a render
verdict. Nothing here was run on the board.

The text is `refused, fragment shader: Valhall: an integer or boolean value
is not lowered yet` (`MaliError::IntValue`, `docs/es2-cts-failures.md`
section M). This note continues `mali-deqp-intval3.md`. That note sorted
the same refusal's 206 cases at `3497799a` into sub-forms, each with a
reference. This note adds 8 references, `tools/mali/testdata/deqp_fsint_*`,
for the lane counts, precisions and operand mixes of those groups that no
committed reference draws. They are pinned by `tests/mali_deqp_fsint.rs`.

## 1. Method, and what could not be done here

- **Case list.** The four slices hold **210 cases** with this text: 188
  Fail and 22 NotSupported (the NotSupported ones are the 22
  `indexing.tmp_array`, `indexing.matrix_subscript` and
  `struct.*.nested_struct_array_dynamic_index` cases, as at `3497799a`).
  All of them are `_fragment` cases or fragment-stage refusals of a
  `_vertex` case (`constants.int_*_vertex`,
  `constant_expressions.trivial.int_vertex`).
- **No dEQP re-measure.** `tools/mali/deqp-host-measure.sh` needs a
  VK-GL-CTS checkout at `93bca01` with its external sources. Neither
  `DEQP_WORK` nor `$CONNECTED_DEPS_DIR/VK-GL-CTS` was on this host. A
  shallow `git fetch` of `93bca01` went through, but checking out its tree
  and running `external/fetch_sources.py` (HTTPS) were not permitted here,
  so no `deqp-gles2` could be built. Only the sparse
  `VK-GL-CTS-gles3` (`data/gles3/shaders`) is present. So the cases were
  **not** re-run through `deqp-gles2` at the tip. In its place:
  - Each case was mapped by its group and name to the sub-form table of
    `mali-deqp-intval3.md` section 2 and to the `deqp_intrnd_*` references.
    Section 2 below gives the result.
  - The committed references of those sub-forms were compiled at the tip
    through their own pin tests (`mali_deqp_intval`, `intval3`, `intval4`,
    `intval5`, `intrnd`, `gaps`; all pass, nothing changed). So each
    sub-form's shader is still refused at the tip with the text its pin
    names. Of the 22 `deqp_intval3_*`, 19 still refuse with this text.
    `opfs_sequence_bool_vec2_mediump` refuses with the gl_FragColor-lane
    text, `rsg_swizzle12_fs` with the uniform-form text, and `rsg_sconv55_fs`
    is byte-equal.
- **References.** Mesa 26.2.1's drm-shim was built the documented way
  into `$CONNECTED_DEPS_DIR/mesa-shim`:
  `tools/mali/drm_shim_ref.sh build "$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz" "$CONNECTED_DEPS_DIR/mesa-shim"`.
  The tarball's sha256 is `c47e81bd...a4442`, the one in
  `third_party/mesa-26.2.1/UPSTREAM`. As a check, the build redrew
  `deqp_intval3_opfs_not_equal_int_highp` and `deqp_intval4_ne_ivec2_fs`,
  and both `expect.json` files were byte-identical to the committed ones.
  Each new reference was drawn with:

```sh
MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
    <shape>.vert <shape>.frag tools/mali/testdata/deqp_fsint_<shape> --instrument shim
```

  All 8 were drawn twice, and the two sets are byte-identical.
- **Sources.** Without a `.qpa`, each pair is written after the generator
  template of the case it names (VK-GL-CTS 93bca01), as the
  `deqp_intval4_*` and `deqp_intval5_*` pairs are. The operator cases
  (`ShaderOperatorTests`) are the template of the committed
  `deqp_intval3_opfs_*` and `deqp_arith_op_*` pairs. Only the type, the
  precision and the input swizzles change: in0 `z`, `wy`, `zxy`, `yzwx`;
  in1 `x`, `yx`, `yzx`, `wzyx`, as the committed verbatim pairs have them.
  The output lanes are `x`, `yz`, `xyz`, `xyzw`. The scale and bias are
  those of the group's committed pair. The conversion cases are the
  `ShaderLibrary` frame of `deqp_intval_reslib_mixed_fs`, with the
  expression of `conversions.test`. The vertex shader of
  `conv_bool_float_int_to_bvec3` is byte-equal to that reference's. A
  `.license` sibling of each `.vert` and `.frag` names the case.

## 2. The 210 cases and the reference of each form

| Group (`dEQP-GLES2.functional.`) | Cases | Form | Reference |
|---|--:|---|---|
| `shaders.operator.unary_operator.{pre,post}_decrement_{effect,result}` | 48 | `--x` / `x--` of `int` / `ivecN` | `intval3_opfs_pre_decrement_result_int_mediump`, `intval3_opfs_post_decrement_effect_ivec2_highp`; **new**: `fsint_predec_effect_ivec4_lowp` (the 12 `_ivec4`), `fsint_postdec_result_ivec3_mediump` (the 12 `post_decrement_result`) |
| `shaders.operator.binary_operator.not_equal` | 12 | `a != b` of `int` / `ivecN` to `bool` | `intval3_opfs_not_equal_int_highp` (int), `intval4_ne_ivec2_fs` (ivec2); **new**: `fsint_ne_ivec4_highp` (the 6 `_ivec3`/`_ivec4`) |
| `shaders.operator.int_compare.notEqual` | 9 | `notEqual(ivecN, ivecN)` to `vecN` | `intval3_opfs_notequal_ivec3_mediump`; **new**: `fsint_notequal_ivec4_highp` (the 3 `_ivec4`) |
| `shaders.operator.sequence.no_side_effects.*_bool_vec2` | 3 | `(a && b, a, ivec2(vec2(a) + v))` | `intval3_opfs_sequence_bool_vec2_mediump`; **new**: `fsint_sequence_bool_vec2_highp` (highp) |
| `shaders.operator.common_functions.step` | 21 | `step(edge, x)` | `intval3_opfs_step_float_vec2_mediump`, `intval3_opfs_step_vec3_highp`; **new**: `fsint_step_float_lowp` (the 3 `_float`) |
| `shaders.operator.binary_operator.logical_{and,or}.bool` | 2 | `a && b` of two compares | `intval3_opfs_logical_and_bool`, `intval4_logical_or_fs` |
| `shaders.conditionals.if.*` | 4 | a literal folded into the condition; one-armed `if` with `RES && isOk` | `intval3_cond_complex_fs`, `intval3_cond_compound_fs` |
| `shaders.indexing.{tmp_array,matrix_subscript}`, `shaders.struct.*.nested_struct_array_dynamic_index` | 22 | a local written at a `uniform int` index (NotSupported on the board) | `intval3_idx_tmp_vec3_dynwrite_fs` |
| `shaders.functions.{qualifiers,array_arguments,misc}` (int) | 9 | the `isOk` of a function's `int` result | `intval3_reslib_inout_int_fs` |
| `shaders.constants.int_*_vertex`, `shaders.constant_expressions.trivial.int_vertex` | 6 | the `isOk` of a constant `_vertex` output | `intval3_reslib_const_int_vs` |
| `shaders.conversions.*` | 25 | `bool(int)`, `bvecN(...)` against a `uniform bool`/`bvecN` | `intval3_reslib_int_to_bool_fs`, `intval3_reslib_ivec4_to_bvec4_fs`; **new**: `fsint_conv_vec2_ivec2_to_bvec4`, `fsint_conv_bool_float_int_to_bvec3` (the 5 `vector_combine` cases with a float operand) |
| `shaders.functions.*` (bool) | 6 | a `bool` through a function against a `uniform bool` | `intval3_reslib_func_bool_fs` |
| `shaders.random.*`, `fragment_ops.interaction.basic_shader.85` | 43 | random programs | the 39 of `mali-deqp-intval3.md` section 2 (`intval3_rsg_*`, `intrnd_*`); `comparison_ops.fragment.13` and `.46` are `intrnd_dead_bool_cmp_fs` and `intrnd_dead_ivec_uniform_fs`; **not covered**: `conditionals.fragment.9`, `trigonometric.fragment.93` |
| **total** | **210** | | |

The 4 random cases not in `mali-deqp-intval3.md` are new to this refusal
since `37305608`. Two have `deqp_intrnd_*` references. At the tip those
two refuse with later texts (LaneCycle and the gl_FragColor-lane text,
their pins in `tests/mali_deqp_intrnd.rs`). So on the host, their first
refusal is no longer this one. The other two
(`conditionals.fragment.9`, `trigonometric.fragment.93`) are
`rsg::ProgramGenerator` output. Their sources cannot be written without
dEQP, so they have no reference and no shape. They are left to
`remeasure-402`.

## 3. The eight references

The fragment programs are Mesa's `fs.fragment`. The vertex programs are the
usual position and varying stores. "fs words" counts the tail and the
blend branch (`FS_WORDS` in the test). The FAU words are the ones the
`expect.json` lists.

| Ref (`deqp_fsint_*`) | Case (`dEQP-GLES2.functional.shaders.`) | Cases of the form | Nearest reference | Mesa's fragment program | fs words | Refusal at `e65567ea` |
|---|---|--:|---|---|--:|---|
| `ne_ivec4_highp` | `operator.binary_operator.not_equal.highp_ivec4_fragment` | 6 | `intval4_ne_ivec2_fs` | `any(ine)` of four lanes: four `ICMP_OR.u32.ne.m1` chained through the third source, one `MUX.v2i16` | 21 | int/bool |
| `notequal_ivec4_highp` | `operator.int_compare.notEqual.highp_ivec4_fragment` | 3 | `intval3_opfs_notequal_ivec3_mediump` | four `ICMP_OR.u32.ne.m1` + `MUX.v2i16`, two `MKVEC.v2i16`; the alpha lane computed (`ATEST` of `r0.h0`) | 26 | int/bool |
| `predec_effect_ivec4_lowp` | `operator.unary_operator.pre_decrement_effect.lowp_ivec4_fragment` | 12 | `intval3_opfs_post_decrement_effect_ivec2_highp` | four `IADD_IMM.i32 #0xFFFFFFFF` at 32 bits (the lowp ints too), `S32_TO_F32` + `FADD.f32 .h0` per lane, two `MKVEC` pairs, a computed alpha | 28 | int/bool |
| `postdec_result_ivec3_mediump` | `operator.unary_operator.post_decrement_result.mediump_ivec3_fragment` | 12 | none | `F32_TO_S32.rtz` then `S32_TO_F32` per lane, with no integer op. Unlike `intrnd_trunc_lane_fs`'s `FROUND.f32.rtz`, it is not folded to `ftrunc` | 18 | int/bool |
| `sequence_bool_vec2_highp` | `operator.sequence.no_side_effects.highp_bool_vec2_fragment` | 1 | `intval3_opfs_sequence_bool_vec2_mediump` | at fp32: `FCMP_OR.f32.lt.f1` (`b2f`), two `FADD.f32`, `F32_TO_S32.rtz`, `S32_TO_F32` | 21 | gl_FragColor lane |
| `step_float_lowp` | `operator.common_functions.step.lowp_float_fragment` | 3 | `intval3_opfs_step_vec3_highp` | a lowp scalar `step` at fp32 (`FCMP_OR.f32.ge.m1`, `MUX.v2i16`, `U16_TO_U32`, `FMA.v2f16`); the colour's constant lanes are the FAU words `3f400000` and `3a003400` | 12 | int/bool |
| `conv_vec2_ivec2_to_bvec4` | `conversions.vector_combine.vec2_ivec2_to_bvec4_fragment` | 5 | `intval3_reslib_ivec4_to_bvec4_fs` | `bool(vec2)` as one `FCMP_OR.v2f16.ne.m1` of the fp16 pair. Each lane is compared against the `uniform bvec4` at 16 bits (`ICMP_OR.v2u16.eq.m1 ... uN.wK.h00`, `S16_TO_S32`) and ANDed (`LSHIFT_AND.i32`) with the int lanes' `ICMP_OR`/`ICMP_AND.u32.eq` chain | 26 | int/bool |
| `conv_bool_float_int_to_bvec3` | `conversions.vector_combine.bool_float_int_to_bvec3_fragment` | (the same 5) | `intval_reslib_mixed_fs` | the bool and the float lane in one `FCMP_OR.v2f16.ne.m1 ... .h10`, each against the uniform at 16 bits. The int lane is `FMA.v2f16` by `1.0025` (`3c03`), then `F16_TO_S32.rtz`, `ICMP_OR.u32.ne` against 0, then `.eq` against the uniform word | 23 | int/bool |

The "cases of the form" counts overlap: the 3 `post_decrement_result.*_ivec4`
cases are in both decrement rows. Together the eight forms cover 39 of the
210 cases.

What the lowering (`lower-fsint`) needs beyond `mali-deqp-intval3.md`
section 3:

- `any(ine)` of more than two lanes as one `ICMP_OR` chain, each compare
  ORed in through the third source (`ne_ivec4_highp`).
- A computed alpha lane: `ATEST` takes the lane's half (`r0.h0` / `r0.h1`)
  in place of `0x3F800000` (`notequal_ivec4_highp`,
  `predec_effect_ivec4_lowp`).
- A lowp `int` is 32-bit like a mediump one: lower_precision leaves
  integers alone (`predec_effect_ivec4_lowp`).
- `i2f(f2i(x))` kept as the two conversions when the int is mediump
  (`postdec_result_ivec3_mediump`).
- The highp sequence at fp32, with `b2f` as `FCMP_OR.f32.lt.f1`
  (`sequence_bool_vec2_highp`). Its next refusal is the colour lane, as
  for the mediump one.
- A bool compared at 16 bits: `ICMP_OR.v2u16.eq.m1` of a `.f1`/`.m1`
  fp16 compare's half against the uniform's `.h00`, then `S16_TO_S32` into
  the 32-bit `LSHIFT_AND` chain (both `conv_*`).

## 4. Reproducing

```sh
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsint
```

`refusals_pinned` holds each refusal, `fs_words_pinned` the word counts,
and each byte-compare is `#[ignore]`d until the lowering takes it. The
lowering moves a reference to `MATCHED` (or `DIFFERS`) and drops its
`#[ignore]` in the same commit.
