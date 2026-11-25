<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
-->

# dEQP-GLES2 mixed-precision references, set 4 (`deqp_mixprec4_*`)

Plan #402 item `ref-mixprec4`. Test: `tests/mali_deqp_mixprec4.rs`.

## Method and limits

* Cases: the `MIX` (`a fragment shader that mixes highp and mediump or lowp
  float values`) and `LOAD` (`a fragment varying load or conversion ...`)
  rows of `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv`
  (random and matrix cases left out).
* Coverage was judged by the header tables of `mali_deqp_mixprec3.rs`,
  `mali_deqp_mixcol.rs` and the mixcol, mixprec and varylink notes.
  `tests/mali_deqp_varyload.rs` is owned by the running item `ref-varyload`
  and was not read.
* **The HOST-ONLY re-measure with `tools/mali/deqp-host-measure.sh` was not
  run**: no VK-GL-CTS tree with sources (`DEQP_WORK`,
  `VK-GL-CTS-host3`, `VK-GL-CTS-host`) exists on this machine
  (`$CONNECTED_DEPS_DIR/VK-GL-CTS-gles3` holds only `data`) and building
  deqp-gles2 was out of reach. Shapes are grouped from the board's case names.
* The fragment shaders are written from the shaders of the existing
  `deqp_mixcol_hp_bool` and `deqp_mixprec3_hp_bvec4` references (dEQP's
  ShaderOperatorCase) with the operator and width changed, not from a `.qpa`.
  The cases they stand for may differ in swizzle or in the exact colour
  statement; a measurement with a dEQP build should confirm them.
* The Mesa shim was built with `tools/mali/drm_shim_ref.sh build` into
  `$CONNECTED_DEPS_DIR/mesa-shim`; references by
  `tools/mali/deqp_ref.sh --instrument shim`.

## Shape table

All six are highp compares written into the mediump `color`; none is
byte-equal at the tip.

| shape | stands for (`shaders.operator.*`) | board cases | tip refusal |
|---|---|---|---|
| `gt_vec2` | `float_compare.greaterThan.highp_vec2_fragment`: `greaterThan` of two highp vec2, bvec2 into `color.yz` | 1 | MIX |
| `ge_vec4` | `float_compare.greaterThanEqual.highp_vec4_fragment`: bvec4 into `color.xyzw` | 1 | IrIndex back-end defect |
| `eq_vec4` | `float_compare.equal.highp_vec4_fragment`: `equal` on vec4 | 1 | MIX |
| `le_vec2` | `float_compare.lessThanEqual.highp_vec2_fragment`: bvec2 into `color.yz` | 1 | MIX |
| `bin_ge_float` | `binary_operator.{greater_or_equal,greater,less,less_or_equal}.highp_float_fragment`: `in0 >= in1`, bool into `color.yz` | 4 | MIX |
| `bin_eq_vec4` | `binary_operator.equal.highp_vec4_fragment`: `in0 == in1`, one bool | 1 | MIX |

The existing references draw only `equal` on vec2 (`mixcol_hp_bool`) and
`lessThan` on vec4 (`mixprec3_hp_bvec4`); the other operators and widths
select different compare instructions and are not covered by them.

Refusal texts are pinned in `refusals_pinned`. `ge_vec4` is a new minimal
trigger of the IrIndex defect (compare against `triage-irindex` /
`ref-irindex3`).

## Shapes not drawn

Left for a run with dEQP sources: `struct.local.nested_struct_array_fragment`,
`fragment_ops.interaction.basic_shader.73`, `functions.qualifiers.{in,out,inout}_highp_float`
(covered by `mixcol_hp_param`), `linkage.uniform_struct_partial_*` (covered by
`mixcol_int_uniform_struct`), unary/trigonometric highp float cases (covered
by `mixcol_hp_scale`), the `LOAD` highp_vec2 cases (`mixcol_ck_scale_highp`).
