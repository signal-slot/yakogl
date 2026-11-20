<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: the constant-product, fused-multiply-add and algebraic refusals (constprod-ref)

Plan #387, item `constprod-ref`: a study and references, **no compiler
change**. Three `MaliError` texts of `src/shaderc/online/mali/mod.rs`:

| text | variant |
|---|---|
| "a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add)" | `ArithConstant` |
| "a fused multiply-add other than a matrix product's (a built-in function's)" | `FusedMultiplyAdd` |
| "a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product)" | `AlgebraicForm` |

glmark2's forms of the first are covered by the `algfold_*` references
(mali-algebraic-folds.md, mali-constant-products.md,
mali-vs-constant-products.md); dEQP's were not classified. This note
classifies them, names Mesa's rule for each shape, and splits the lowering
into one-session steps ranked by cases. The references are
`tools/mali/testdata/deqp_constprod_*`, pinned by
`tests/mali_deqp_constprod.rs`.

Every count below is **HOST-ONLY** (docs/notes/mali-deqp-families.md
sections 0 and 15): the compile and link outcome of each shader at the tip
under `deqp-gles2` on the host, never a render verdict. "Cases" are
must-pass cases whose FIRST refusal at the tip is the text; a case that
passes this refusal may meet another one after it (a random shader nearly
always has several).

## 1. Method

- Tip `9ac31f30`. CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
  (`tools/cts/deps.toml`) with `tools/cts/cts-yakogl-target.patch`, in
  `$CONNECTED_DEPS_DIR/VK-GL-CTS` (a shallow checkout of that commit, its
  external sources fetched).
- Mesa 26.2.1 for the references: `tools/mali/drm_shim_ref.sh build
  $CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz
  $CONNECTED_DEPS_DIR/mesa-shim` (the work directory `deqp_ref.sh` takes by
  default).
- Cases: the 10,597 refused cases of
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`, as
  `tools/mali/deqp-host-measure.sh` takes them:

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$TMPDIR/measure" --jobs 3 --chunk 100
```

  Of its `measure.tsv` the rows whose host refusal is one of the three texts
  are **533** cases: 386 `ArithConstant`, 139 `FusedMultiplyAdd`, 8
  `AlgebraicForm`. The board's numbers at `37305608` were 465, 100 and 6;
  the intval, unitype and constop steps that landed since moved cases in and
  out of these texts, which is why the tip's table is the one to use.
- Refusal sites. The compiler names no site, so the cases were split by the
  `return Err(..)` that raised the text. For that, a **temporary,
  uncommitted** patch (reverted; `git status` clean at the end) gave each of
  the 26 `Err(MaliError::{ArithConstant,FusedMultiplyAdd,AlgebraicForm})` sites of
  `glsl.rs`, `nir.rs` and `nir_fs.rs` a number in a static, put the number
  at the head of the message (the info log is cut after about 170
  characters) and split `glsl.rs:1681` by its condition (clamped, chain,
  not placed) and the stage. The measure was rerun over the same 10,597
  cases; the 533 cases are the same set. The sites below are the line
  numbers of `9ac31f30`.
- References: `tools/mali/deqp_ref.sh <case>.vert <case>.frag
  tools/mali/testdata/deqp_constprod_<shape> --instrument shim`, the pair
  verbatim from the `.qpa` of the smallest case of the shape (the
  shim: `drm_shim_ref.sh run <work> vs fs <prefix> --prim triangles
  --uniform-values`, then `pan_ref_diff.py extract --draw`). A
  `.license` sidecar per shader names the case.

## 2. The shapes

Sites: `glsl.rs` is the GLSL IR rewrite pass (`Tree`), `nir.rs` the vertex
walk, `nir_fs.rs` the fragment walk. VS/FS is the stage that refused
(the other half of the program compiled or was not reached).

| shape | cases | VS / FS | site (`9ac31f30`) | text | dEQP groups |
|---|---:|---|---|---|---|
| `litfirst_scalar`: a negated literal first in a scalar product, `-3.0 * x`, in a vertex shader | 174 | 174 / 0 | `glsl.rs:1681` (`!placed`) | const product | `shaders.random.*` 167 (basic_expression 53, all_features 32, scalar_conversion 32, conditionals 22, comparison_ops 7, trigonometric 7, swizzle 5, texture 5, exponential 4), `fragment_ops.interaction.basic_shader` 5, `shaders.functions.array_arguments` 2 |
| `neglit_pair`: a product of two negated literals, `-0.5 * -2.0` (a user function's arguments, a random shader's folds) | 91 | 47 / 44 | `glsl.rs:1609` | const product | `shaders.random.*` 88, `shaders.functions.overloading.user_func_arguments_{vertex,fragment}` 2, `fragment_ops.interaction.basic_shader` 1 |
| `constchain`: a constant times a product with a constant (`Nir::alu`'s `is_const_product`) | 62 | 61 / 1 | `nir.rs:2414` | const product | `shaders.operator.angle_and_trigonometry.{degrees,radians}` 16, `default_vertex_attrib.{vec4,mat4}` 16, `shaders.invariance.*.loop_{0,3,4}` 9, `shaders.random.*` 21 |
| `chainlit`: a negated literal times a product, `x * y * -c` (a product of a product) | 33 | 10 / 23 | `glsl.rs:1681` (`chain`) | const product | `shaders.random.*` 33 (conditionals 10, all_features 10, scalar_conversion 5, basic_expression 5, swizzle 1, comparison_ops 1, texture 1) |
| `litfirst_vec`: a negated literal first in a vector or matrix product, `-2.0 * a` | 8 | 4 / 4 | `glsl.rs:1681` (`!placed`, w > 1) | const product | `shaders.functions.datatypes.mat{2,3,4}_{vertex,fragment}` 6, `shaders.random.exponential` 2 |
| `reassoc_mixed`: a constant on both sides of a reassociation that mixes operations | 2 | 2 / 0 | `glsl.rs:1492` | const product | `shaders.random.all_features.vertex.{17,94}` |
| `foldcall_fs`: a built-in called on constants, folded by Mesa (`const float cval = reflect(1.0, 1.0)`), in the fragment harness | 16 | 0 / 16 | `nir_fs.rs:3745` | const product | `shaders.constant_expressions.builtin_functions.{geometric (reflect, refract), trigonometry (acos, atan_separate)}_*_fragment` |
| `foldcall_vs`: the same in the vertex shader, trigonometric and geometric | 24 | 24 / 0 | `nir.rs:4781` | fma | `shaders.constant_expressions.builtin_functions.{trigonometry (acos, asin, atan_combined, atan_separate), geometric (reflect, refract)}_*_vertex` |
| `foldcall_mat_vs`: a constant matrix expression | 1 | 1 / 0 | `nir.rs:4781` | fma | `shaders.constant_expressions.operators.math_mat_vertex` |
| `foldcall_mod`: `mod()` of constants | 7 | 7 / 0 | `nir.rs:2649` (`vs_fmod`) | fma | `shaders.constant_expressions.builtin_functions.common.mod_*_vertex` |
| `trig_builtin`: `acos`/`asin`/`atan`/`atan2` of attributes | 47 | 47 / 0 | `nir.rs:4781` | fma | `shaders.operator.angle_and_trigonometry.{acos,asin,atan,atan2}.{highp,mediump}_*_vertex` 32, `shaders.random.trigonometric.vertex` 13, `shaders.random.all_features.vertex` 2 |
| `matmul`: a matrix product of attributes or constants in the vertex shader | 41 | 41 / 0 | `nir.rs:4781` | fma | `shaders.matrix.mul.{const,dynamic}_*_{mat2_mat2,mat3_mat3,mat4_mat4,matN_vecN}_vertex` 34, `shaders.matrix.mul_assign.dynamic_*_vertex` 7 |
| `geom_builtin`: `reflect`/`refract` of attributes | 19 | 19 / 0 | `nir.rs:4781` | fma | `shaders.operator.geometric.{reflect,refract}.*_vertex` 10, `shaders.invariance.*.subexpression_precision_*` 9 |
| `constmat_float`: a constant matrix times a float, the factoring `fadd(fmul(a,b), fmul(a,c))` with a sum other than 1.0 | 8 | 0 / 8 | `nir_fs.rs:1491` | algebraic | `shaders.matrix.mul.const_*_mat{2,4}_float_fragment` 6, `shaders.random.comparison_ops.fragment` 2 |
| **total** | **533** | **437 / 96** | | 386 / 139 / 8 | |

The three texts' totals: const product 174 + 91 + 62 + 33 + 8 + 2 + 16 =
**386** (VS 298, FS 88); fused multiply-add 24 + 1 + 7 + 47 + 41 + 19 =
**139** (all VS); algebraic 8 (all FS). Together **533**, the number the
measure gives.

## 3. Mesa's rule for each shape, and the references

All words are what Mesa 26.2.1 (panfrost, G610 on the shim, which the G310
shares: mali-g310-vs-g610.md) emits. NIR is the shim's `shaderdb` dump
after Mesa's optimisation loop. Nothing is invented: where this section
gives a Mesa rule, the reference shows its result; where it says the
compiler's reading is "not measured", the lowering session decides with the
reference.

| shape | reference (case) | Mesa |
|---|---|---|
| `litfirst_scalar` | `litfirst_vs` (`shaders.random.basic_expression.vertex.57`), `litfirst_array_vs` (`shaders.functions.array_arguments.global_in_float_vertex`) | `fmul(fneg(a), b)` is `fneg(fmul(a, b))` (`nir_opt_algebraic.py:2387`), after glsl_to_nir made `-3.0 * x` `fmul(-3.0, x)`; `fmul(a, -1.0)` is `fneg` (:296). In both references the product is dead or a `-1.0` factor: `litfirst_vs`'s varying program is a bare attribute store, `litfirst_array_vs`'s are four `fneg` of the f2f16 lanes (`f * vec4(arr)` with `f == -1.0`). So the words of the refused literal-first shape are `fneg`, not a product: the walk's `fneg(fmul(c, x))` of a negative constant (the mediump fragment's, `Fs::op`) reaches them |
| `neglit_pair` | `neglit_pair_vs`, `neglit_pair_fs` (`shaders.functions.overloading.user_func_arguments_{vertex,fragment}`: `func(in0) * func(-0.5, -2.0)`) | `fmul(fneg(a), fneg(b))` is `fmul(a, b)` (the :2387 rule twice, then `fneg(fneg(a))` is `a`, :230), and the constants fold: `-0.5 * -2.0` is `1.0`, so `func(in0) * 1.0` is `fneg(in0)` (a single `fneg` in the references' NIR). The fragment half is the isOk harness |
| `constchain` | `constchain_degrees_vs` (`degrees.highp_vec2_vertex`), `constchain_attrib_vs` (`default_vertex_attrib.vec4.vertex_attrib_1f`), `constchain_loop_vs` (`invariance.highp.loop_0`), `constchain_fs` (`random.comparison_ops.fragment.38`) | GLSL IR's `reassociate_constant` (`opt_algebraic.cpp:175-215`, :276-282) folds the two constants of `c1 * (x * c2)`, in the GLSL IR, in the front end. **Not always:** `degrees(in0) * 0.04 + 0.50` keeps both constants (`fmul(x, 57.29578)`, `f2f16`, `ffma(.., 0.04, 0.5)`): the highp-to-mediump conversion between the two is a node the rule does not see through. `default_vertex_attrib`'s `(a / 10 + 1) / 2 ... / 2` is `ffma(a, 0.1, 1.0)`, `fmul(.., 0.5)` for `.w` and `fmul(0.25, fadd(lane, lane))` for the sum of two lanes: the two `0.5`s of `(x * 0.5 + z * 0.5) * 0.5` combine to `0.25` outside the add (the factoring rule, `nir_opt_algebraic.py`'s `fadd(fmul(a, b), fmul(a, c))`, then the product's constants). The compiler refuses all of these on `is_const_product`; whether a given shape is one Mesa folds is the lowering's question (step 3) |
| `chainlit` | `chainlit_fs`, `chainlit_vs` (`random.conditionals.{fragment,vertex}.34`) | a product of a product with a negated literal: Mesa takes `x * c` first, then `* y` (mali-constant-products.md section 1, negmul_three); the references are generated shaders whose words are a few `fneg`/`fadd` of folded constants (`fadd(<-2.0, 7.0>, fneg(v.yx))`) |
| `litfirst_vec` | `litfirst_mat_vs`, `litfirst_mat_fs` (`functions.datatypes.mat2_{vertex,fragment}`: `return -2.0 * a`) | `fneg(fadd(a, a))`: the `2.0` product becomes `fadd(a, a)` (`nir_opt_algebraic.py:2387` then the `fmul(a, 2.0)` rule), the `fneg` outside; one `fadd` and one `fneg` per lane pair at fp16 (the mediump `mat2`) |
| `reassoc_mixed` | `reassoc_mixed_vs` (`random.all_features.vertex.94`) | the `mixed` reassociation of `reassociate_operands` (`glsl.rs:1482-1492`); the reference gives its words, the rule is not transcribed (a generated shader) |
| `foldcall_fs` | `foldcall_reflect_fs`, `foldcall_acos_fs` (`constant_expressions...reflect_float_fragment`, `acos_float_fragment`) | no ALU: GLSL IR's constant evaluation (`constant_expression.cpp`, host `acosf`, `atan2f`, `reflect`) leaves a literal; in the fragment harness `isOk` compares the uniform with `fadd(<const>, fneg(ref))` and the constant is rounded to fp16 where the harness is mediump (`mod(87.65, 3.7)` is `0x411a`, 2.5508) |
| `foldcall_vs` | `fma_foldcall_acos_vs`, `fma_foldcall_atan_vs`, `fma_foldcall_reflect_vs`, `fma_foldcall_refract_vs` | the same constants in a vertex shader: `out0 = const` (a constant varying, the vertex program has no ALU) |
| `foldcall_mod` | `fma_foldcall_mod_vs` | `mod(87.65, 3.7)` folded to the f32 constant; its fragment half is the same isOk |
| `trig_builtin` | `fma_trig_acos_vs` (`acos.highp_float_vertex`), `fma_trig_atan2_vs`, `fma_random_trig_vs` (`random.trigonometric.vertex.22`) | Mesa's `acos` (the front end's polynomial): `fabs`, `ffma` chain with `0.074261`, `-0.018729`, `-0.212114`, `1.570729`, `frsq` then `frcp` (a sqrt), `fneg`, `ffma(.., 1.570796)`, and the sign fix-up `flt_pan`/`bcsel_pan` on both sides (`-1.0 .. 1.0` select), `ffma(sign, r, 1.570796)`; `atan`/`atan2` have their own chain. `builtin_call.rs` has the front-end `acos`/`atan`; the vertex walk's catch-all `AluOp::Ffma` refuses them |
| `matmul` | `fma_matmul_const_vs` (`mul.const_highp_mat2_mat2_vertex`), `fma_matmul_dynamic_vs` (`mul.dynamic_highp_mat3_vec3_vertex`), `fma_matmul_mat_vs` (`mul.dynamic_lowp_mat3_mat3_vertex`), `fma_mulassign_vs` (`mul_assign.dynamic_highp_mat2_vertex`) | the matrix product of attributes is a chain of `ffma(a.y, v.x, fmul(v.y, a.x'))` terms; Mesa keeps the first term a `fmul` (or an `fadd` where a factor is `1.0`) and the others `ffma`; the vertex walk recognises some matrix shapes only (`is_reflect` and the matrix recogniser) |
| `geom_builtin` | `fma_geom_reflect_vs` (`geometric.reflect.highp_float_vertex`), `fma_geom_refract_vs`, `fma_invariance_vs` (`invariance.highp.subexpression_precision_highp`) | `reflect` is `ffma(fneg(fadd(d, d)), fmul(a, d), a)`: `fmul(a, d)`, `fadd(d, d)` for `2.0 * d`, `fneg`, `ffma`; `refract` adds `fsqrt`, `flt`, `fneg`, `ffma` and a select; `distance`, `fract` follow. The walk has the `is_reflect` shape for one reflect form |
| `constmat_float` | `alg_constmat_fs` (`mul.const_highp_mat2_float_fragment`: `mat2(0.6, 0.7, -1.0, 0.4) * v`) | `fmul(0.6, v)`, `fneg(v)` (the `-1.0` factor, :296), `fmul(v, 1.1)`: `0.7 * v + 0.4 * v` is `v * (0.7 + 0.4)` (the factoring rule), the sum 1.1 (`0x3f8ccccd` in f32) folded before the `f2f16` of each lane. The compiler takes the factoring only when the sum is 1.0 (`nir_fs.rs:1483-1491`) |

`tests/mali_deqp_constprod.rs` has one test per reference, all 31
`#[ignore]`d: every reference is refused today, with the whole text of its
site (31 of 31: `refusals_pinned`). The test asserts the exact refusal; the
lowering that lifts one moves it to `LOWERED` (and removes the `#[ignore]`)
in the same commit.

References, one line each (`deqp_constprod_<name>`): `litfirst_vs`,
`litfirst_array_vs`, `neglit_pair_vs`, `neglit_pair_fs`,
`constchain_degrees_vs`, `constchain_attrib_vs`, `constchain_loop_vs`,
`constchain_fs`, `chainlit_fs`, `chainlit_vs`, `litfirst_mat_vs`,
`litfirst_mat_fs`, `reassoc_mixed_vs`, `foldcall_reflect_fs`,
`foldcall_acos_fs`, `fma_foldcall_{acos,atan,reflect,refract,mod}_vs`,
`fma_matmul_{const,dynamic,mat}_vs`, `fma_mulassign_vs`,
`fma_trig_{acos,atan2}_vs`, `fma_geom_{reflect,refract}_vs`,
`fma_invariance_vs`, `fma_random_trig_vs`, `alg_constmat_fs`. A reference
that is a generated random shader (`litfirst_vs`, `chainlit_*`,
`constchain_fs`, `reassoc_mixed_vs`, `fma_random_trig_vs`) is also the
smallest case of its shape; the non-random ones are the shape's own groups.

## 4. Lowering map and steps

Every step below is checked against its references (byte-equal) and moves
them from `REFUSED` to `LOWERED`; the other half of a pair (the harness, the
position program) compiles already. A step does not change a number of
cases by itself: **cases here are the first refusals that go away**; a
random shader meets its next refusal (`Next` in `mali-deqp-families.md`)
after the step.

| step | cases | shapes | where | what |
|---|---:|---|---|---|
| 1 | 174 | `litfirst_scalar` | `glsl.rs` `Tree::neg_products` (`:1637`), `nir.rs` `Nir::alu` `Alu::Fmul` | stop refusing a negated literal first in a scalar product in a vertex shader and a highp fragment: leave `fmul(-c, x)` to the walk as `fneg(fmul(c, x))`, as the mediump fragment already does (`glsl.rs` `mediump_fragment && w == 1 && !placed` leaves it, `nir_fs.rs` `Fs::op` lowers it); `-1.0 * v` is `fneg` (`nir.rs` `Prod::FmulMinusOne`, :2392). References `litfirst_vs`, `litfirst_array_vs` |
| 2 | 91 | `neglit_pair` | `glsl.rs:1609`, `nir.rs:2378` (`Fmul if is_alu(a, Fneg) && is_alu(b, Fneg)`, no case reaches it first today), `nir_fs.rs:1433` | `fmul(fneg(a), fneg(b))` is `fmul(a, b)`: fold two negated literals to their f32 product (exact: Mesa folds it in NIR), and a product of two `fneg` into the product with no `fneg`; the walk's two `fneg` sites share the rule. References `neglit_pair_vs/fs` |
| 3 | 62 | `constchain` | `nir.rs:2414` `is_const_product`, `glsl.rs` reassociation (`:1482-1492`) | decide per shape, with the four `constchain_*` references, whether Mesa's GLSL IR rule folds (a `c1 * (x * c2)` visible in one expression) or the value is not folded (a conversion between them: `constchain_degrees_vs` is written as is) and refuse only the former, or transcribe `reassociate_constant`. The `degrees`/`radians` (16) and `default_vertex_attrib` (16) cases are the first ones to settle |
| 4 | 48 | `foldcall_fs`, `foldcall_vs`, `foldcall_mat_vs`, `foldcall_mod` | `glsl.rs` constant folder, `builtin_call.rs`, `nir.rs:2649`, `nir_fs.rs:3745` | evaluate a built-in call on constants in the front end as GLSL IR does (`acos`, `asin`, `atan`, `atan2`, `reflect`, `refract`, `mod`, constant matrix products) with f32 libm results, round to fp16 where the use is mediump, and do not lower an `ffma` for them: no `ffma` reaches the walks. References `foldcall_*_fs`, `fma_foldcall_*_vs`. (The measure's constop family `const_call_*` of mali-deqp-families.md section 8 is the same shapes seen before this site) |
| 5 | 47 | `trig_builtin` | `nir.rs:4781`, `builtin_call.rs` | lower `acos`/`asin`/`atan`/`atan2` of a vertex attribute as Mesa does (the fp32 polynomials of `fma_trig_acos_vs`, `fma_trig_atan2_vs`, with their `flt_pan`/`bcsel_pan` sign fix-up); the fragment walk has the front half (`builtin_call.rs` `atan`, `nir_fs.rs` ffma case). References `fma_trig_*`, `fma_random_trig_vs` |
| 6 | 41 | `matmul` | `nir.rs` matrix-product recogniser near `:4765`, `is_reflect` | the matrix product of vertex attributes and constants beyond the shapes the recogniser knows (mat3 x vec3, mat2 x mat2, mat4 x mat4, `*=`): the term order and the first-term `fmul` of `fma_matmul_*`, `fma_mulassign_vs`. The fragment's `const_mat_product` has the constant forms |
| 7 | 33 | `chainlit` | `glsl.rs:1681` `chain` | a product of a product with a negated literal: take `x * c` first then `* y` (negmul_three), for the two stages. References `chainlit_vs/fs` |
| 8 | 19 | `geom_builtin` | `nir.rs:4781`, `is_reflect` | `reflect`/`refract` of attributes beyond `is_reflect`'s form; `distance`, `fract`, `refract` in `invariance`. References `fma_geom_*`, `fma_invariance_vs` |
| 9 | 8 + 8 + 2 | `litfirst_vec`, `constmat_float`, `reassoc_mixed` | `glsl.rs:1681` (w > 1), `nir_fs.rs:1483-1491`, `glsl.rs:1492` | `-2.0 * a` for a vector or matrix (`fneg(fadd(a, a))` at fp16); the factoring's sum other than 1.0 folded in f32 (`0.7f + 0.4f` is `0x3f8ccccd`) and the `-1.0` factor an `fneg`; the `mixed` reassociation. References `litfirst_mat_*`, `alg_constmat_fs`, `reassoc_mixed_vs` |

The steps sum to 174 + 91 + 62 + 48 + 47 + 41 + 33 + 19 + 18 = **533**.
Step 1 to 3 are the three biggest and share `glsl.rs` and the `Nir::alu`
fneg rules, so they are one lowering session each in that order; step 4 is
the one whose cases are mostly shader-library harness cases
(`constant_expressions`), so each of its 48 has no other construct and
should pass once its vertex and fragment halves compile; step 5 to 8 are the
vertex walk's built-ins; step 9 is the small tail.

**Forecast (HOST-ONLY).** The first-refusal forecast above is the count of
cases whose first refusal goes away. The cases that then pass the compile
(not only the first refusal) are the ones with no other refusal: step 4's
48 (the `constant_expressions` library cases) and steps 5 to 8's 107 cases (15 of them random; the rest `operator`,
`matrix`, `invariance`) are the ones the sites above decide alone; the random shaders (about 330 of the 533) will move to their
next refusal and the forecast per step needs the host measure rerun after
the step. A rerun of the command of section 1 at the step's commit, with the
three texts as the filter, gives it.

## 5. What this does not establish

- The classification of random shaders is by the refusing site, not by the
  construct: 330 of the 533 cases are generated `shaders.random.*` and
  `fragment_ops.interaction` shaders, and their shapes inside a site are
  not subdivided (steps 1, 2, 3 and 7 take them whole).
- Which site refuses a case is read from a numbered, temporary build of the
  compiler; the measure's "first refusal" is the first the compiler
  reaches in the vertex shader, then the fragment shader. The compiler's
  order is not Mesa's.
- Each reference's other half (position program, fragment harness) is
  Mesa's own program for the pair; a pair whose partner is the isOk harness
  compiles that half already.
- The G310 needs nothing separate: the references are the G610's and serve
  both boards (mali-g310-vs-g610.md).
