<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the dEQP-GLES2 `-0.0 through a variable` refusal (negzero), its shapes and Mesa 26.2.1 references

**Status: study** (plan #387, item `negzero-ref`; issues #20, #7). Nothing
under `src/` changed. The references are `tools/mali/testdata/deqp_negzero_*`,
pinned by `tests/mali_deqp_negzero.rs`. Measured at the tip **`cc89a1cf3085`**
(2026-11-20). Mesa paths are relative to `src/` of the 26.2.1 tree.

The refusal is `MaliError::NegZeroShared`: "a -0.0 that a comparison reads
through a variable, or that another value reads too (Mesa's rules make it 0.0
there)". `Tree::neg_zero_compares` and `Tree::plus_zero` raise it
(`src/shaderc/online/mali/glsl.rs:1824-2146`). It is family `negzero` in
`tools/cts/mali-family-refusals.txt`. On the NanoPC-T6 run `37305608` it was
the first refusal of **117 must-pass cases, all Fail, 0 NotSupported, VS 70 /
FS 47** (`docs/notes/mali-deqp-families.md` section 17).

## 1. How this was measured, and what was not

- The case list is the board's:
  `python3 -I tools/cts/mali-family-tree.py --max 1000 --refusal 'a -0.0 that a comparison reads through a variable' tools/cts/log/2026-11-15-t6-gles2-S{1,2,3,4}-37305608`
  (117 cases). Each case's stage (VS/FS) and status come from the slices'
  `refusals.tsv` and `results.tsv`.
- **`tools/mali/deqp-host-measure.sh` was not run.** It needs the VK-GL-CTS
  checkout at `93bca018` and its external sources, and this host had no
  network for the fetch (`git fetch` and `curl` of github.com were refused).
  So the case counts per shape below are the **board's counts at `37305608`**,
  not a per-case host measure at the tip. Only the shapes' representative
  pairs were compiled at the tip, through `tests/mali_ref`'s `compile_pair`.
  `rank` (plan #387) or a later run of the host measure gives the tip's per-case
  counts.
- The only dEQP random-shader source in the tree is the earlier host
  measure's pair `tools/mali/testdata/deqp_shapes/constop_neg_zero_variable.{vert,frag}`
  (`shaders.random.all_features.fragment.13`, verbatim from the `.qpa` at
  `abcc9013`). The random vertex shaders of this family have no source in the
  tree, so no vertex random pair is pinned here.
- The `array_arguments` pair is rebuilt from the "both" source of
  `copy_local_inout_on_call` in `data/gles3/shaders/functions.test:2547-2590`
  (VK-GL-CTS `93bca018`, in `$CONNECTED_DEPS_DIR/VK-GL-CTS-gles3`), with its
  `#version 300 es` line dropped. The ES 2 data file was not reachable. The
  wrapping is glsShaderLibraryCase's, copied from the committed
  `deqp_intval_reslib_float_fs` (fragment case) and `deqp_varylink_swz_xx`
  (vertex case) pairs.
- References: `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_negzero_<name> --instrument shim`,
  with the drm-shim built by `tools/mali/drm_shim_ref.sh build` from the
  pinned `mesa-26.2.1.tar.xz` (sha256 of `third_party/mesa-26.2.1/UPSTREAM`)
  into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's self-check
  (`gm2_cf_ifelse_fs`, `gm2_cf_ifelse_vs`) was byte-equal with this build.

## 2. The shapes

A shape is the code construct of section 3 of `mali-deqp-families.md`.

| Shape | Cases (board, `37305608`) | Fail / NS | VS / FS | dEQP groups | Example case | Reference | First refusal at `cc89a1cf` | What Mesa does with the -0.0 |
|---|---:|---|---|---|---|---|---|---|
| `constglobal`: random-generator shaders whose `const` globals hold int and bool constant expressions (`int(6.0) - 7` in a float vector, `int(c)` of a `const bool`, `float(ab)` of a compare) | 115 | 115 / 0 | 69 / 46 | `shaders.random.all_features` (30 VS, 19 FS), `.trigonometric` (19, 19), `.conditionals.vertex` (7), `.conditionals.fragment` (7), `.conditionals.combined` (11, 1), `fragment_ops.interaction.basic_shader.{42,91}` (VS) | `shaders.random.all_features.fragment.13` | `deqp_negzero_constglobal_fs` (verbatim), `deqp_negzero_constglobal_reduced` | `NegZeroShared` (both) | No -0.0 reaches a compare. The GLSL front end evaluates every `const` initialiser to an `ir_constant` (`compiler/glsl/ast_to_hir.cpp:4841-4842`, `constant_expression_value`), so the compares fold to constant booleans and unused globals disappear. The reduced pair becomes a constant colour (FS 7 words: `ATEST`, two `MOV.i32` of FAU `0x3c003c00`, `BLEND`). Mesa's fragment.13 shader (43 words) has no `FCMP` at all. |
| `array_inout`: `shaders.functions.array_arguments.copy_local_inout_on_call`: `float arr[4]` passed `inout` twice to one call, `negate` as `set(arr, -get(arr))`, `isEqual(a, b) ? 1.0 : -1.0` | 2 | 2 / 0 | 1 / 1 | `shaders.functions.array_arguments` | `..._fragment`, `..._vertex` | `deqp_negzero_array_inout_fs`, `deqp_negzero_array_inout_vs` (rebuilt) | **moved**: arithmetic ("arithmetic other than +, -, * and a dot or matrix product") | Mesa's `fneg` is `FADD x.neg, 0x0.neg` (`-x + -0.0`, `FADD.v2f16 r3, r2.neg, 0x0.neg`). The compare `absDiff(..) < 0.1` is `FCMP_OR.v2f16.lt` against the fp16 constant `0x2e66`, then `MUX.v2i16` picks between the value and its negation. No -0.0 constant is compared. |

How the `interaction.basic_shader` cases are counted: they come from the same
random shader generator (rsg, `glsFragOpInteractionCase`), so they are put in
`constglobal`. That is assumed, not measured, because no source of theirs is in
the tree.

### `constglobal` at the tip: the construct that refuses

A probe reduction of `all_features.fragment.13` at `cc89a1cf` dropped lines
while the refusal stayed, then folded sub-expressions by hand. The refusal
needs **none of `main`**: it comes from constant globals that nothing reads.
Three remain (`deqp_negzero_constglobal_reduced.frag`):

```glsl
const bool c = vec4(1.0, int(6.0) - 7, true, -1.0).brag == vec4(1, 1, -1, -1);
const bool ab = 7.25 <= -1.1979 - vec2(int(-4), int(c)).s + 0.001;
const bool l = float(-3) <= float(ab) + 0.001;
```

- With `int(6.0) - 7` written as `-1`, the pair **compiles**. The int-typed
  constant arithmetic inside a float vector is not folded, so `c` stays a
  run-time compare, and with it `int(c)`, `ab` and `l`.
- With `vec2(int(-4), int(c)).s` written as `float(int(-4))` or
  `vec2(-4.0, int(c)).s`, the pair compiles.
- With `+ 0.001` dropped from `l`, the pair compiles. With `+ 0.0` in its
  place, it refuses. The compare side `float(ab) + k` (a `b2f` of a
  compare that did not fold, plus a constant) is the value `plus_zero` takes as
  "may fold to -0.0". This is most likely the catch-all arm at
  `glsl.rs:2137-2141`. The arm was not instrumented: nothing under `src/` was
  touched.
- Unused constant globals are also walked at the tip, apart from this
  refusal. `const bool ab = 7.25 <= tan(-0.875);` alone, read by nothing,
  is refused as arithmetic. Mesa folds it and drops it.

So at the tip the family is mostly not about a real -0.0. It is a
constant-expression chain the front end does not fold, and on that chain
the -0.0 rule cannot prove a lane is not -0.0. It is the `S4` group of
`docs/notes/mali-deqp-constop.md` section 2 (`NegZeroShared`, 113 `random.*`
cases) seen from the refusal side.

## 3. Proposed lowering, with a forecast

**`constglobal` (115 cases).** Fold `const` initialisers as GLSL constant
expressions in the front end (`glsl.rs`), the way Mesa's
`constant_expression_value` does, before the IR is walked:

1. Fold the int and bool forms on constants: `int(float)` (truncation),
   `float(int)`, `bool(x)`, `int(bool)`, int `+ - *`, the vector
   constructors with int and bool lanes, swizzles, and the `==`/`!=` of
   vectors (`all`/`any` of the lane compares).
2. Fold the built-ins a `const` initialiser may call on constants (`tan`,
   `asin`, `atan`, `exp`, `exp2`, `log2`, `inversesqrt`, `sqrt`, `cos`). Mesa
   folds them with the host libm (`compiler/glsl/ir_expression_operation.py`:
   `sinf`, `asinf`, `exp2f`, ...). A folded
   value only matters where it reaches an output; there it must equal Mesa's
   bits. Pin each one against the reference that reads it. This is shared with
   constop's `const_call_transcendental` (`mali-deqp-families.md` section 8)
   and #367's `CONST_FOLD`.
3. Drop the constant globals nothing reads; Mesa never emits them.

With the chain folded, no compare of the reduced form reaches
`neg_zero_compares`: `deqp_negzero_constglobal_reduced` must then compile
**byte-equal** to its 7-word reference, which is the first un-ignore.
`deqp_negzero_constglobal_fs` un-ignores only once its `main` is
byte-equal too. Its `main` reads mediump varyings through `tan`, `exp2` and
`atan`, so its next refusal is expected in the arithmetic family. Forecast:
the 115 cases leave `NegZeroShared`. The earlier host measure at
`abcc9013` (`mali-deqp-families.md` section 8, `neg_zero_variable`, 116
cases) gives their next steps: 103 unblock this family's text, 7 next
refuse `intval`, 3 next refuse `CONST_FOLD`. How many then Pass is not
forecast here: it needs the host measure at the lowering's tip, because the
random shaders' `main` bodies meet the ARITH, FMA and colour-lane texts too.

**`array_inout` (2 cases).** Nothing to lower in this family at the tip: the
first refusal moved to the arithmetic text, which `arith-ref` /
`arith-lower-1` own. The two references stay pinned here. When arithmetic
lowers, `refusals_pinned` names them, and the pin moves to the next refusal
or the test un-ignores. Forecast for negzero: 0.

**Not a lowering of `plus_zero` itself.** The rule of
`mali-deqp-constop.md` section 10.6 (rewrite a known `Lane::K(-0.0)` where the
compare is built) is not what these shapes need. None of them holds a
known -0.0 lane at a compare.

## 4. The test

`tests/mali_deqp_negzero.rs`, with
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_negzero`:
four reference tests, all `#[ignore]` with their refusal quoted, and
`refusals_pinned`, which runs and asserts each one's exact current refusal
through `mali_ref::refusal_change`. It also checks that every
`deqp_negzero_*.expect.json` on disk is either in `REFUSALS` or in
`MATCHED`; `MATCHED` is empty at this tip. At `cc89a1cf`: 1 passed, 4 ignored. With
`--ignored`, each of the four fails only with "the pair is refused: ..." and
its pinned text.

## 5. Issue comment text (for #20; agents do not post it)

> negzero (`MaliError::NegZeroShared`, 117 board cases at `37305608`): the dEQP
> shapes are pinned against Mesa 26.2.1 references at `cc89a1cf`
> (`tools/mali/testdata/deqp_negzero_*`, `tests/mali_deqp_negzero.rs`,
> `docs/notes/mali-deqp-negzero.md`).
> - `constglobal`, 115 cases (`shaders.random.{all_features,trigonometric,conditionals}`,
>   `fragment_ops.interaction.basic_shader.{42,91}`): still refused at the tip.
>   The cause is the front end leaving a `const` initialiser's int and bool
>   constant expression unfolded (`int(6.0) - 7` in a `vec4`). `plus_zero`
>   cannot prove the chain `float(ab) + 0.001` is not -0.0. Mesa folds every
>   `const` initialiser in the GLSL front end, so no compare is left.
>   The lowering is constant folding of `const` initialisers, not the -0.0 compare
>   rule. The reduced pair `deqp_negzero_constglobal_reduced` (a 7-word
>   constant colour in Mesa) is the first un-ignore.
> - `array_inout`, 2 cases (`functions.array_arguments.copy_local_inout_on_call_*`):
>   moved to the arithmetic refusal at the tip.
> - Not measured: the per-case host first refusal at the tip.
>   `deqp-host-measure.sh` could not fetch VK-GL-CTS on this host, so the
>   counts are the board's.
