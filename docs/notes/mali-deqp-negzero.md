<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the dEQP-GLES2 `-0.0 through a variable` refusal (negzero), its shapes and Mesa 26.2.1 references

**Status: study** (plan #387, item `negzero-ref`; issues #20, #7). Nothing
under `src/` changed. The references are `tools/mali/testdata/deqp_negzero_*`,
pinned by `tests/mali_deqp_negzero.rs`. Measured at the tip **`cc89a1cf3085`**
(2026-11-20; the host measure's `commit` reads `823c5d0f`, this item's first
commit, which changes nothing under `src/`). Mesa paths are relative to `src/`
of the 26.2.1 tree.

The refusal is `MaliError::NegZeroShared`: "a -0.0 that a comparison reads
through a variable, or that another value reads too (Mesa's rules make it 0.0
there)". `Tree::neg_zero_compares` and `Tree::plus_zero` raise it
(`src/shaderc/online/mali/glsl.rs:1824-2146`). It is family `negzero` in
`tools/cts/mali-family-refusals.txt`. On the NanoPC-T6 run `37305608` it was
the first refusal of 117 must-pass cases, all Fail, VS 70 / FS 47
(`docs/notes/mali-deqp-families.md` section 17).

## 1. How this was measured

- **Board list:**
  `python3 -I tools/cts/mali-family-tree.py --max 1000 --refusal 'a -0.0 that a comparison reads through a variable' tools/cts/log/2026-11-15-t6-gles2-S{1,2,3,4}-37305608`
  gives 117 cases. They fall in three groups: 113 `shaders.random.*`, the 2
  `fragment_ops.interaction.basic_shader.{42,91}`, and the 2
  `shaders.functions.array_arguments.copy_local_inout_on_call_{fragment,vertex}`.
- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh`, with
  VK-GL-CTS `93bca018` at `$CONNECTED_DEPS_DIR/VK-GL-CTS` and
  `cts-yakogl-target.patch` applied. It ran three times, with
  `--filter shaders.random.` (1,767 board-refused cases),
  `--filter fragment_ops.interaction.basic_shader` (97) and
  `--filter array_arguments.copy_local_inout_on_call` (2). The other groups
  were not run, so a case outside them whose first refusal became `negzero`
  after `37305608` is not counted here (rank, plan #387, measures the whole
  tree). HOST-ONLY: these are compile and link outcomes only.
- **Shapes:** each of the 137 cases that refuse `negzero` at the tip is
  reduced by a scratch line reducer. It drops lines of the refusing shader
  while the refusal stays, compiling through `tests/mali_ref`'s
  `compile_pair`. The reduced shaders are then grouped by what is left:
  1. only global initialisers;
  2. a statement in `main` together with built-in calls (`sin`, `asin`,
     `atan`, `log`, `exp`, `sqrt`, `inversesqrt`, ...);
  3. a statement in `main` with no built-in call.

  The grouping is a heuristic over the reduced text, not an instrumented
  refusal site: nothing under `src/` was touched.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_negzero_<name> --instrument shim`,
  on the drm-shim that `tools/mali/drm_shim_ref.sh build` made from the
  pinned `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The
  README's self-check (`gm2_cf_ifelse_fs`, `gm2_cf_ifelse_vs`) was byte-equal.
  Each dEQP pair is verbatim from the host measure's `.qpa`, with a
  provenance comment added. `deqp_negzero_constglobal_fs` comes from
  `tools/mali/testdata/deqp_shapes/constop_neg_zero_variable` (the same
  source, from the `abcc9013` measure).

## 2. Counts at the tip

| | Cases |
|---|---:|
| Board `negzero` cases (`37305608`) | 117 |
| ... still `negzero` first at the tip | 113 |
| ... moved: `all_features.fragment.86` to the constant-product text, `all_features.vertex.95` to the non-finite constant text | 2 |
| ... moved: `array_arguments.copy_local_inout_on_call_{fragment,vertex}` to the arithmetic text | 2 |
| Cases newly first `negzero` at the tip (board: 22 `367:CONST_FOLD`, 2 `intval`), all `shaders.random.*` | 24 |
| **`negzero` first at the tip, in the groups measured** | **137 (all Fail, 0 NS; VS 68 / FS 69)** |

## 3. The shapes

A shape is the code construct of section 3 of `mali-deqp-families.md`.

| Shape | Cases (tip) | Fail / NS | VS / FS | dEQP groups | Example case | References | What Mesa does with the -0.0 |
|---|---:|---|---|---|---|---|---|
| `constglobal`: the refusal comes from global initialisers alone, not from `main`. These are constant int and bool expressions and built-ins on constants (`int(6.0) - 7` in a `vec4`, `asin(...) < ...`), in `const` or non-const globals | 22 | 22 / 0 | 8 / 14 | `shaders.random.all_features` (8 VS, 13 FS), `.trigonometric.fragment` (1) | `shaders.random.all_features.fragment.13` | `deqp_negzero_constglobal_fs`, `deqp_negzero_constglobal_vs` (`all_features.vertex.10`), `deqp_negzero_constglobal_reduced` | None exists. The GLSL front end evaluates `const` initialisers to `ir_constant` (`compiler/glsl/ast_to_hir.cpp:4841-4842`, `constant_expression_value`). Constant non-const globals are propagated and dead-coded. No compare of them is left: the reduced pair's FS is a 7-word constant colour, and fragment.13's FS (43 words) and vertex.10's VS (113 words) hold no `FCMP`. |
| `mainbuiltin`: a compare in `main` (an `if` or a `?:`, or a `bool` inside a constructor) reads a value built from built-in calls on constants, beside int and bool conversions | 84 | 84 / 0 | 40 / 44 | `shaders.random.all_features` (21 VS, 25 FS), `.trigonometric` (19, 19) | `shaders.random.all_features.vertex.13` | `deqp_negzero_mainbuiltin_vs`, `deqp_negzero_mainbuiltin_fs` (`all_features.fragment.14`) | Mesa folds the constant side, so a compare is left only where an input is read. Vertex.13's VS keeps `FCMP_OR.f32.ge.m1 r2, 0x3F800000, r1` against a computed lane, with no -0.0 operand. Fragment.14's FS has no compare. |
| `mainplain`: the same without built-ins. Int and bool conversions of constants (`vec4(int(bool(true)), ...)`) and a local stored twice (`float g = ...; g = -12.0 * 0.75 + g;`, `c = 3.5; if (ab == vec3(b, c, t)) c = r - c;`) reach a vector compare | 31 | 31 / 0 | 20 / 11 | `shaders.random.conditionals.{vertex 7, fragment 7, combined 14}`, `.comparison_ops.fragment` (1), `fragment_ops.interaction.basic_shader.{42,91}` (VS) | `fragment_ops.interaction.basic_shader.91` | `deqp_negzero_mainplain_vs` (interaction.91), `deqp_negzero_mainplain_fs` (`comparison_ops.fragment.43`) | Mesa propagates the stored constant (`c = 3.5`) into the compare. Interaction.91 keeps `ICMP`/`CSEL` on the uniform lanes, with no -0.0 constant. Comparison_ops.43's unused compare statement is removed: FS of 9 words. |
| `array_inout` (moved): `shaders.functions.array_arguments.copy_local_inout_on_call`, `float arr[4]` passed `inout` twice to one call | 0 (board 2) | - | (board 1 / 1) | `shaders.functions.array_arguments` | `..._fragment` | `deqp_negzero_array_inout_fs`, `deqp_negzero_array_inout_vs` | First refusal at the tip: arithmetic. Mesa's `-x` is `FADD x.neg, 0x0.neg` (`-x + -0.0`), and `absDiff(..) < 0.1` is `FCMP_OR.v2f16.lt` against `0x2e66`. No -0.0 constant is compared. |

### The construct, from fragment.13's reduction

Fragment.13 refuses with three constant globals that nothing reads
(`deqp_negzero_constglobal_reduced.frag`):

```glsl
const bool c = vec4(1.0, int(6.0) - 7, true, -1.0).brag == vec4(1, 1, -1, -1);
const bool ab = 7.25 <= -1.1979 - vec2(int(-4), int(c)).s + 0.001;
const bool l = float(-3) <= float(ab) + 0.001;
```

- With `int(6.0) - 7` written as `-1`, the pair compiles. The int-typed
  constant arithmetic in a float vector is not folded, so `c` stays a
  run-time compare, and with it `int(c)`, `ab` and `l`.
- With `vec2(int(-4), int(c)).s` written as `float(int(-4))`, it compiles.
- With `+ 0.001` dropped from `l`, it compiles; with `+ 0.0` in its place,
  it refuses. `plus_zero` takes `float(ab) + k` (a `b2f` of an unfolded
  compare, plus a constant) as "may fold to -0.0". This is most likely the
  catch-all arm at `glsl.rs:2137-2141`.
- Unused constant globals are walked at the tip in general:
  `const bool ab = 7.25 <= tan(-0.875);` alone, read by nothing, is refused
  as arithmetic. Mesa folds it away.

The other reductions show the same pattern: constant sub-expressions
(built-ins on constants, int and bool conversions, swizzles of mixed
constructors) that the front end leaves unfolded meet a compare. The -0.0
rule then cannot prove a lane is not -0.0. Literal `-0.0` occurs in the
refusing shader of only 14 of the 137 cases (`(float(-1)) * -0.0` in
vertex.10). The family is the `S4` group of `docs/notes/mali-deqp-constop.md`
section 2, seen from the refusal side.

## 4. Proposed lowering, with a forecast

Fold constant expressions in the front end (`glsl.rs`), as Mesa's GLSL
compiler does, before `neg_zero_compares` runs. This is **not** the
`Lane::K(-0.0)` compare rewrite of `mali-deqp-constop.md` section 10.6: none
of the representatives holds a known -0.0 lane at a compare.

1. **`constglobal`, 22 cases.** Evaluate global initialisers that are
   constant expressions: int `+ - *`, `int(float)` (truncation), `float(int)`,
   `bool(x)`, `int(bool)`, constructors with mixed int, bool and float lanes,
   swizzles, the vector `==`/`!=` (`all`/`any` of lanes), and the built-ins
   on constants (`asin`, `atan`, `tan`, `exp`, `exp2`, `log`, `log2`,
   `sqrt`, `inversesqrt`, `sin`, `cos`). Drop the globals nothing reads. Mesa
   folds the built-ins with the host libm
   (`compiler/glsl/ir_expression_operation.py`: `sinf`, `asinf`, `exp2f`, ...):
   a folded value that reaches an output must equal Mesa's bits, so pin it
   against the reference that reads it. First un-ignore:
   `deqp_negzero_constglobal_reduced` (7-word constant colour).
2. **`mainbuiltin`, 84 cases.** The same folding inside `main`, for
   expressions whose operands are all constant, so the compare sees a
   literal on one side. It shares the built-in folding with constop's
   `const_call_transcendental` and #367's `CONST_FOLD`.
3. **`mainplain`, 31 cases.** Integer and boolean constant folding (shared with
   `intval`), plus copy propagation of a local's constant store into the
   compare that reads it (`c = 3.5; ... vec3(b, c, t)`), which is what the
   -0.0 rule calls "a local stored twice".

**Forecast:** the 137 cases leave `NegZeroShared` once the three steps land.
22 + 84 + 31 is an upper bound per step: a case may need two steps. In the
host measure, the partner shader of 127 of the 137 compiles already (its
`next_family` is empty). The other 10 next refuse in the partner shader:
`367:CONST_FOLD` 2, `367:CONTROL_FLOW` 2, `367:FRAGCOLOR_LANE` 2, `intval` 2,
`colourpair` 1, `mixprec` 1. What the refusing shader itself meets after
this family is not measured: random shaders' `main` bodies also reach the
arithmetic, FMA and colour-lane texts. How many Pass needs the host measure at
the lowering's tip.

**`array_inout`:** 0 for this family; its two cases now wait on the arithmetic
lowering (`arith-ref` / `arith-lower-1`). Its references stay pinned here, and
`refusals_pinned` names them when arithmetic lowers.

## 5. The test

`tests/mali_deqp_negzero.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_negzero`.

- Nine reference tests, all `#[ignore]` with their refusal quoted.
  Seven are `negzero`; the two `array_inout` ones are arithmetic.
- `refusals_pinned` runs. It asserts each one's exact current refusal
  (`mali_ref::refusal_change`). It also checks that every
  `deqp_negzero_*.expect.json` on disk is in `REFUSALS` or in `MATCHED`;
  `MATCHED` is empty at this tip.
- At `cc89a1cf`: 1 passed, 9 ignored. With `--ignored`, each of the nine
  fails only with "the pair is refused: ..." and its pinned text.

## 6. Issue comment text (for #20; agents do not post it)

> negzero (`MaliError::NegZeroShared`): at `cc89a1cf`, the host measure
> (`deqp-host-measure.sh`, groups `shaders.random`,
> `fragment_ops.interaction.basic_shader`, `array_arguments`) finds 137 cases
> with this first refusal, all Fail, VS 68 / FS 69. 113 of the board's 117
> at `37305608` are still here, and 24 random cases joined from
> CONST_FOLD/intval. 2 board cases moved to constop texts and 2
> (`array_arguments.copy_local_inout_on_call_*`) to arithmetic.
> The shapes are `constglobal` (22: global initialisers alone), `mainbuiltin`
> (84: a compare in `main` reads built-ins on constants) and `mainplain`
> (31: int and bool constant conversions, a local stored twice). They are pinned
> against Mesa 26.2.1 references in `tools/mali/testdata/deqp_negzero_*` and
> `tests/mali_deqp_negzero.rs`.
> In every one Mesa folds the constant side in the GLSL front end, so no -0.0
> reaches a compare. The lowering is constant folding in the front end (const
> initialisers, built-ins on constants, int and bool conversions, copy
> propagation of a constant store), not the -0.0 compare rule. First
> un-ignore: `deqp_negzero_constglobal_reduced`. Details:
> `docs/notes/mali-deqp-negzero.md`.
