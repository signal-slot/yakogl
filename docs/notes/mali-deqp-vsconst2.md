<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): what is left of the non-finite constant refusal at the tip, and its Mesa 26.2.1 references

**Status: study** (plan #402, item `ref-vsconst2`; issue #20). Nothing under
`src/` changed. The references are `tools/mali/testdata/deqp_vsconst2_*`;
`tests/mali_deqp_vsconst2.rs` pins them. Measured HOST-ONLY at the tip
**`14e7a7421`** (2026-11-28); nothing here was checked on the NanoPC-T6.
Follows `docs/notes/mali-deqp-vsconst.md`.

## 1. Measure

`tools/mali/deqp-host-measure.sh` (features gpu-mali, shaderc-online,
no-shader-table; VK-GL-CTS `93bca018` with `cts-yakogl-target.patch`, fetched
shallow into `$CONNECTED_DEPS_DIR`) over the 259 cases whose text in
`refusals.tsv` of `2026-11-24-t6-gles2-S{1..4}-8ff01055` is
`MaliError::ConstantFold` ("a constant operation on or to a value that is not
a finite normal float"). The host text was matched with and without
`Valhall: `. 188 cases have a host refusal, 71 compile.

| First host refusal at the tip | Cases |
|---|--:|
| this refusal (`ConstantFold`) | **19**, all fragment shader, 0 vertex |
| "a constant product in a shape Mesa's algebraic rules move or fold" | 107 (`shaders.indexing` loops, 106, and `random.exponential` 1) |
| arithmetic / fused multiply-add (#367's text) | 48 |
| other (uniform past 128 words, mixed precision, integer value, unwritten varying, highp sampler, algebraic rewrite) | 14 |
| none (compiles on the host) | 71 |

The vertex-shader count of this refusal is **0** at the tip, against 144 on the
board at `8ff01055`: the loop and constant lowerings since then moved the 188
vertex cases on, 107 of them to the constant-product refusal (the
`deqp_vsconst_*` pins, `tests/mali_deqp_vsconst.rs`).

Refusal sites came from a scratch copy of the tip with an `eprintln!` at each
`MaliError::ConstantFold` (not committed):

| Cases | Site | Shapes | Covered by |
|--:|---|---|---|
| 16 | `nir.rs:2430` | FS `log2`, `log`, `pow`, `atan` of constants (`constant_expressions`) | `deqp_cfold2_const_call_transcendental_{log2,pow,atan}` (the only existing references that still refuse with this text; checked by compiling every `deqp_*` pair) |
| 3 | `nir_fs.rs:1400` | `fold_rcp16` of a zero operand: `random.trigonometric.fragment.{3,10}`, `random.all_features.fragment.40` | **none** |

No existing reference refuses at `nir_fs.rs:1400`: `deqp_cfold2_*rcp16` and
`zero_operand_rcp16` now compile.

## 2. The uncovered shape: an `atan` of zero at mediump

Minimal pairs (found by cutting the three cases down): `atan(0.0)` and
`atan(vec4(0.3, 0.0, -6.0, 0.0))` in a mediump colour refuse at
`nir_fs.rs:1400`; `atan(0.5)`, `atan(2.0)` and `asin` of non-zero lanes do not
reach it (they refuse with #367's arithmetic text first, since `atan` is no
supported operation). So a zero lane of `atan` produces a 16-bit `frcp` of
0.0 for the fold. The cause inside Mesa's `atan` lowering was not traced.

| Reference | Source |
|---|---|
| `rcp0_atan_zero` | minimal: `vec4(atan(0.0), 0.0, 0.0, 1.0)` |
| `rcp0_atan_vec4_lanes` | minimal: `atan(vec4(0.3, 0.0, -6.0, 0.0))` |
| `rcp0_atan_uniform` | `shaders.random.trigonometric.fragment.10` verbatim |
| `rcp0_asin_atan_const` | `shaders.random.trigonometric.fragment.3` verbatim |
| `rcp0_all_features` | `shaders.random.all_features.fragment.40` verbatim |

Made with `tools/mali/deqp_ref.sh <vert> <frag> <prefix> --instrument shim` on
the drm-shim built from the pinned `mesa-26.2.1.tar.xz` into
`$CONNECTED_DEPS_DIR/mesa-shim`. All five refuse at the tip with the text
above, so every test is `#[ignore]`d with it, and `refusals_pinned` holds the
text. No `.nir.sha` was made (outside the item).

## 3. On `lower-vsconst`'s branch

`2c0c65963` (its base is older than the tip: `92344e4d`) compiled the same 259
programs through `tests/mali_ref::compile_pair`: 27 still refuse with this text
(the 16 above, the 3 random ones, `trigonometric.fragment.{47,49}` and the six
`matrix.div.const_{lowp,mediump}_*` of the older base). All five new references
refuse there with the same text; lower-vsconst does not touch them. The
vertex-shader count of this refusal is 0 on that branch too.

## 4. For the lowering

The references hold the words Mesa emits for the zero lanes; how Mesa folds
the 16-bit `frcp` of 0.0 and what reads it was not traced in this item.
