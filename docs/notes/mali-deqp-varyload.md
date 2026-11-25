<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2 `varyload`: the 50 `fragment varying load or conversion` cases at 8ff01055

Date 2026-11-25. Plan #402 item `ref-varyload` (stage 5, issue #20, NanoPC-T6
Mali-G610). This note adds no reference and no test: every measurable shape
of the refusal is already drawn by an existing `deqp_*` reference (§2), and
the sixteen cases that are not drawn by one could not be re-measured on this
host (§1). Read `docs/notes/mali-deqp-mixprec3.md` first.

## 1. What was and was not measured

The cases are the 50 rows of
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose text
is `a fragment varying load or conversion of a form no reference draws`.

`tools/mali/deqp-host-measure.sh --cases` was **not** run. It needs a
VK-GL-CTS checkout at 93bca01 with its external sources and a
`deqp-gles2` build (`DEQP_WORK`, or `$CONNECTED_DEPS_DIR/VK-GL-CTS-host`).
This worker could clone VK-GL-CTS (blob-less, no checkout) into
`$CONNECTED_DEPS_DIR/VK-GL-CTS-host`, but could not run the `git checkout`
in it (the sandbox asked for approval and nobody could give it), so there
is no source tree, no `.qpa`, and no dEQP build. Every number below is the
board's text at 8ff01055 plus the shapes `mali-deqp-mixprec3.md` measured
from dEQP's generators. Nothing here is HOST-ONLY tip output.

## 2. Shape table

| shape | cases | `shaders.*` cases | covered by |
|---|---:|---|---|
| hp2_sum | 14 | `operator.binary_operator.{add,sub,mul}{,_assign_effect,_assign_result}.highp_float`, `common_functions.mix`, `geometric.{distance,dot,reflect}.highp_float`, `geometric.distance.highp_vec2` | `deqp_mixprec3_hp2_sum` |
| hp_vec3_sum | 3 | `operator.binary_operator.{add,sub,mul}.highp_float_vec3` | `deqp_mixprec3_hp_vec3_sum` |
| mp_vec3_scale | 3 | `unary_operator.{plus,post_increment_result,post_decrement_result}.mediump_vec3` | `deqp_mixprec3_mp_vec3_scale` |
| lp_vec3_scale | 3 | the same, `lowp_vec3` | `deqp_mixprec3_lp_vec3_scale` |
| vary_array_static | 4 | `indexing.varying_array.{float,vec2}_static_write_static{,_loop}_read` | `deqp_mixprec3_vary_array_static` |
| ck_scale_highp | 3 | `unary_operator.{plus,post_increment_result,post_decrement_result}.highp_vec2` | `deqp_mixcol_ck_scale_highp` |
| lz_rnd_other | 2 | `random.basic_expression.{combined,fragment}.45` | `deqp_mixcol_lz_rnd_other` |
| return | 2 | `return.conditional_return_{always,never}_fragment` | `deqp_gaps_mixprec_fs_varying_other` |
| random | 16 | `random.basic_expression.combined.{31,57}`, `.fragment.{8,31,33,57}`, `comparison_ops.fragment.15`, `conditionals.fragment.{19,36,75}`, `scalar_conversion.{combined,fragment}.65`, `swizzle.fragment.{15,36,41}`, `texture.fragment.110` | **not measured** |

14+3+3+3+4+3+2+2+16 = 50. 34 cases are covered by a reference that exists.
The 16 `random` cases come from dEQP's random-shader generator (seeded per
case); their fragment shaders exist only in a `.qpa` from a dEQP run, so no
shape can be named, grouped, or written as a reference without that run.

## 3. What is left

- Re-run on a host that can build dEQP:
  `tools/mali/deqp-host-measure.sh --cases <the 16 names> --cts <VK-GL-CTS at 93bca01 with external sources and the yakogl target>`.
  Group the 16 fragment shaders, drop the shapes `deqp_mixcol_*`,
  `deqp_mixprec3_*`, `deqp_varylink_*` and `deqp_gaps_*` draw, and draw up to
  eight of the rest with `tools/mali/deqp_ref.sh ... --instrument shim` under
  `deqp_varyload_*`, with `tests/mali_deqp_varyload.rs` in the style of
  `tests/mali_deqp_mixprec3.rs`.
- The 34 covered cases need nothing new: `lower-mixprec3` lifts them.
