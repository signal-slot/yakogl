<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2 `varyload`: the 50 `fragment varying load or conversion` cases at 8ff01055

Date 2026-11-25. Plan #402 item `ref-varyload` (stage 5, issue #20, NanoPC-T6
Mali-G610). It adds eight references (`tools/mali/testdata/deqp_varyload_*`,
Mesa 26.2.1 drm-shim, GPU g610) and `tests/mali_deqp_varyload.rs`. It changes
no code. Read `docs/notes/mali-deqp-mixprec3.md` first: it measured 34 of the
50 cases from dEQP's generators without a dEQP build. This note measured all
50 with one, so the 16 `shaders.random.*` cases have their real sources.

## 1. How it was measured

`tools/mali/deqp-host-measure.sh --cases <the 50>` with
`DEQP_WORK`-style `--cts` = VK-GL-CTS 93bca01 (yakogl target applied,
external sources fetched) at the tip `ef5ce720`, with features
`osal-linux gpu-mali shaderc-online no-shader-table report-es2`. HOST-ONLY:
compile and link outcomes only, nothing ran on the board. Result: 50 cases
began, 43 refused on the host, 14 not reproduced (7 refuse differently, 7 do
not refuse on the host). The 16 random cases all refuse on the host with the
same `FsVaryingLoad` text as on the board.

## 2. Shape table

| shape | cases | `shaders.*` cases | covered by |
|---|---:|---|---|
| hp2_sum | 14 | `operator.binary_operator.{add,sub,mul}{,_assign_effect,_assign_result}.highp_float`, `common_functions.mix`, `geometric.{distance,dot,reflect}.highp_float`, `geometric.distance.highp_vec2` | `deqp_mixprec3_hp2_sum` |
| hp_vec3_sum | 3 | `operator.binary_operator.{add,sub,mul}.highp_float_vec3` | `deqp_mixprec3_hp_vec3_sum` |
| mp_vec3_scale | 3 | `unary_operator.{plus,post_increment_result,post_decrement_result}.mediump_vec3` | `deqp_mixprec3_mp_vec3_scale` |
| lp_vec3_scale | 3 | the same, `lowp_vec3` | `deqp_mixprec3_lp_vec3_scale` |
| vary_array_static | 4 | `indexing.varying_array.{float,vec2}_static_write_static{,_loop}_read` | `deqp_mixprec3_vary_array_static` |
| ck_scale_highp | 3 | `unary_operator.*.highp_vec2` | `deqp_mixcol_ck_scale_highp` |
| lz_rnd_other | 2 | `random.basic_expression.{combined,fragment}.45` | `deqp_mixcol_lz_rnd_other` |
| return | 2 | `return.conditional_return_{always,never}_fragment` | `deqp_gaps_mixprec_fs_varying_other` |
| rnd57 | 2 | `random.basic_expression.{fragment,combined}.57` | `deqp_varyload_rnd57` |
| rnd31 | 2 | `random.basic_expression.{fragment,combined}.31` | `deqp_varyload_rnd31` |
| cond36 | 1 | `random.conditionals.fragment.36` | `deqp_varyload_cond36` |
| basic33 | 1 | `random.basic_expression.fragment.33` | `deqp_varyload_basic33` |
| cond19 | 1 | `random.conditionals.fragment.19` | `deqp_varyload_cond19` |
| swz41 | 1 | `random.swizzle.fragment.41` | `deqp_varyload_swz41` |
| basic8 | 1 | `random.basic_expression.fragment.8` | `deqp_varyload_basic8` |
| tex110 | 1 | `random.texture.fragment.110` | `deqp_varyload_tex110` |
| not drawn | 6 | `random.comparison_ops.fragment.15`, `swizzle.fragment.{15,36}`, `conditionals.fragment.75`, `scalar_conversion.{fragment,combined}.65` | none: past the cap of eight |

14+3+3+3+4+3+2+2+2+2+1+1+1+1+1+1+6 = 50. The eight new references cover 10 of
the random cases. Every one is refused at the tip with `FsVaryingLoad`
(`nir_fs.rs`), so all eight tests are `#[ignore]`d and `refusals_pinned`
(running) pins the text. Sources are the verbatim `.qpa` shaders of the
host run, each `.license` says so. The vertex shaders are dEQP's (`dEQP_Position`
and one `a_*` attribute per varying).

## 3. What else the host run showed

The 34 operator, indexing and return cases do not all refuse on the host with
the board's text any more (14 not reproduced):
`binary_operator.{add,sub}.highp_float` and their assigns (6) now say `a gl_FragColor
lane other than the varying's own lane or a constant`,
`geometric.distance.highp_float` (1) says the highp/mediump mix refusal, and 7
(`binary_operator.mul{,_assign_effect,_assign_result}.highp_float`,
`geometric.dot.highp_float`, `unary_operator.*.highp_vec2`) compile on the
host and fail in dEQP only, which a host run cannot judge. The mixprec3
references pin the refusals by shape, so none of this moves a pin here.

## 4. Reproducing

```sh
tools/mali/deqp_ref.sh <shape>.vert <shape>.frag tools/mali/testdata/deqp_varyload_<shape> --instrument shim
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_varyload
```

`lower-mixprec3` lifts these: when one is byte-equal, remove its `ignore`
and move it from `REFUSED` to `MATCHED` in the same commit.
