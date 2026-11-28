<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): vertex-shader uniform-type shapes no reference covers

Plan #402 item `ref-vsunif3`. Refusal: vertex shader "a uniform other than a
float, vec2, vec3, vec4 or mat4" (371 Fail at `8ff01055`, 362 at `37305608`).
Existing references: `deqp_vsunif_*`, `deqp_unitype_*`, `deqp_unif2_*`,
`deqp_t3gap_vsunif_*` and the rest of `deqp_*` (no `deqp_vsunif2_*` exists).

## 1. HOST-ONLY re-measure at `14e7a7421`

`tools/mali/deqp-host-measure.sh` (features osal-linux, gpu-mali,
shaderc-online, no-shader-table, report-es2; VK-GL-CTS 93bca01 from
`$CONNECTED_DEPS_DIR/VK-GL-CTS-host`) over the 463 cases of
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose text is
the vertex refusal (texts matched with and without `Valhall: `). Host first
refusal per case: 211 compile and link and now stop nowhere on the host (the
render is black, which the host cannot judge), 8 Pass, and 244 refuse:

| first refusal at the tip | cases |
|---|---:|
| vertex `ARITH` (arithmetic other than +, -, *, dot) | 110 |
| `varylink` (varying unwritten, vertex) | 53 (52 NotSupported) |
| fragment refusals (`FRAGCOLOR_LANE` 13, `unitype` fs 8, `ARITH` 8, `CONTROL_FLOW` 7, `mixprec` 5, `intval` 2, `CONST_FOLD` 2) | 45 |
| vertex `loops` | 20 |
| vertex `FMA` 5, `CONTROL_FLOW` 5, `CONST_FOLD` 3, `ATTR_FOLD` 1 | 14 |
| **vertex uniform type or precision (`unitype`)** | **2** |

So `lower-vsunif` has taken the refusal almost whole: two cases still stop at a
vertex uniform text. Both are `vs_mediump_struct`, the same
`UniformPrecision` refusal ("a vertex shader uniform that is not highp ...
that is neither an array nor a struct member,"):
`shaders.struct.uniform.nested_struct_array_vertex` and
`...nested_struct_array_dynamic_index_vertex`.

## 2. Shapes and coverage

| shape | cases | covered by |
|---|---:|---|
| `uniform S s[2]` (`S` holds `T b[3]`), constant indices | 1 | `deqp_unif2_arr_struct_array` (same source as the case, verbatim) |
| the same uniform read at `uniform int` indices | 1 | none |

The uncovered one is `deqp_vsunif3_struct_arr_dynidx` (source verbatim from the
`.qpa`, `.license` names the case). The nearest neighbours are different:
`deqp_t3gap_vsunif_nested_struct_array_dynidx` builds the struct in a local
from attributes and `uniform mediump float`s, and `deqp_vsunif_struct_dynidx`
and `deqp_unitype_vsstruct_*` hold single structs.

## 3. The reference

Mesa 26.2.1 drm-shim (`tools/mali/deqp_ref.sh ... --instrument shim`, with
`--uniform-values`). The test is `tests/mali_deqp_vsunif3.rs`:
`refusals_pinned` runs and holds the `UniformPrecision` text; the byte-compare
`struct_arr_dynidx` is `#[ignore]`d. A lowering that takes it moves it to
`MATCHED`, removes the `#[ignore]` and the pin in the same commit.

## 4. For the planner

- The 110 cases that reach vertex `ARITH` and the 53 `varylink` ones are other
  refusals' work, as is every fragment case.
- Nothing else of this refusal needs a reference; the 371 count is a board
  count at `8ff01055`, and at the tip the host sees 2.
- The generated `.nir.sha` files of other prefixes (`nir_refs.sh`) were not
  made: the contract of this item names references, test and note only.
