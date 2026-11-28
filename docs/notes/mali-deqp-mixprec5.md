<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# dEQP-GLES2 mixed-precision and varying-load references, set 5 (`deqp_mixprec5_*`)

Plan #402 item `ref-mixprec5`. Test: `tests/mali_deqp_mixprec5.rs`.

## Method

* **Measure.** `tools/mali/deqp-host-measure.sh` HOST-ONLY at the tip
  (14e7a7421; features host-test-equivalent `osal-linux gpu-mali
  shaderc-online no-shader-table report-es2`), the slices
  `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`, VK-GL-CTS 93bca01
  (`HOST_DEQP_CTS`, `tools/cts/deps-env.sh --build`): 5,838 cases began, 4,574
  with a host refusal. The tool names a family from the refusal text, with or
  without the `Valhall: ` prefix. Family `mixprec` at the tip: 190 cases
  (`fs_varying_other` 96, `fs_highp_declared` 60, `fs_mixed_other` 32,
  `fs_varying_plain` 2). Those are the board's MIX 168 and LOAD 50 minus the
  cases the host refuses differently, plus a few of other families that now
  land here.
* **Tip only.** `lower-mixprec3` (81b3ce19, branch
  `connected/01a21f6f-c1d8-76ce-9751-bcc81024775c`, reachable, not merged) was
  not measured over the whole tree: it is based on c04d3fdef, whose
  `capi/Cargo.toml` still has the duplicate `vivante-st` key. Only the six new
  references were run on it, in a scratch worktree with the duplicate line
  deleted (never committed): `lt_vec3` byte-equal, `refract_vec2` refused
  "arithmetic other than +, -, * ...", `length_vec3` refused MIX, the other
  three unchanged.
* **Coverage.** Every candidate fragment shader of the non-random cases was
  compared byte for byte with every `tools/mali/testdata/*.frag`; a shader
  with an identical text in an existing reference is covered and dropped.
  Dropped that way: `binary_operator.add.highp_vec2_float`
  (`deqp_varyres_st_vec2_float`), `common_functions.smoothstep.highp_float_vec2`
  (`deqp_varyres_st_smooth_vec2`), `exponential.{log,pow}.highp_float`,
  `angle_and_trigonometry.cos.highp_float`, `common_functions.mod.highp_float`
  (`deqp_arith_op_*_fs_highp`), `common_functions.fract.highp_float`
  (`deqp_mixprec2_hp_fract`), `float_compare.equal.highp_vec3`
  (`deqp_irindex_load_vec3_cmp`).
* **References.** Sources are the verbatim programs of the measure
  (`programs/<id>.vert|.frag`), drawn with `tools/mali/deqp_ref.sh
  --instrument shim` (Mesa 26.2.1 drm-shim in `$CONNECTED_DEPS_DIR/mesa-shim`).

## Shape table

| reference | stands for | board cases of its kind | refusal at the tip |
|---|---|---|---|
| `mulasg_vec3_float` | `binary_operator.mul_assign_result.highp_vec3_float_fragment`; the sibling `{add,sub,div,mul}_assign_{effect,result}.highp_{vec2,vec3}_float` | 16 | LOAD |
| `div_vec2_float` | `binary_operator.div.highp_vec2_float_fragment`; also `div.highp_vec3_float`, `div.highp_float_{vec2,vec3}` | 4 | LOAD |
| `refract_vec2` | `geometric.refract.highp_vec2_float_fragment`; also `highp_vec3_float` | 2 | LOAD |
| `length_vec3` | `geometric.length.highp_vec3_fragment`; also `highp_vec4` | 2 | LOAD |
| `mod_vec2_float` | `common_functions.mod.highp_vec2_float_fragment`; also `highp_vec3_float`, `highp_float` is covered | 2 | LOAD |
| `lt_vec3` | `float_compare.lessThan.highp_vec3_fragment`; the five operators on vec3 | 5 | MIX |

All six are refused at the tip; `refusals_pinned` holds the texts.

## Shapes left without a reference (cap of six reached)

`common_functions.{clamp,max,min}.highp_{vec2,vec3}_float`,
`common_functions.mix.highp_{vec2,vec3}_float`,
`geometric.distance.highp_{vec2,vec3,vec4}`, `geometric.reflect.highp_float`,
`exponential.sqrt.highp_float`, `binary_operator.{equal,less,greater,...}.
highp_{float,vec2,vec3,vec4}` (highp compare into a lane), `operator.sequence.*`,
`matrix.{add,sub,div}.const_highp_mat{2,3,4}_mat{2,3,4}`, `matrix.mul.const_highp_mat4_vec4`,
`functions.qualifiers.{in,out,inout}_highp_float` (covered by `mixcol_hp_param`),
the six `loops.*.basic_highp_float` NotSupported cases, the `shaders.random.*`
cases (28 `fs_varying_other`, 29 `fs_mixed_other`) and
`fragment_ops.interaction.basic_shader.{1,64}`. Only identical-text coverage
was checked for the non-random ones above, so some may be covered by a
reference of a different spelling.

Note: `unary_operator.{plus,post_{in,de}crement_result}.{mediump,lowp}_vec3`
(6 cases) still refuse at the tip although `deqp_mixprec3_{mp,lp}_vec3_scale`
exist for them: `lower-mixprec3` is their lowering.
