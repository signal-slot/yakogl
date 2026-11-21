<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): lowering design for the constant-product, fused-multiply-add and algebraic refusals (HOST-ONLY design)

Plan #387, item constprod-design. A design note: it changes nothing under
`src/`, `tests/` or `tools/`. It reads the 31 Mesa 26.2.1 references of item
constprod-ref (`tools/mali/testdata/deqp_constprod_*`, pinned refused by
`tests/mali_deqp_constprod.rs`, shapes and counts in
`docs/notes/mali-deqp-constprod.md`, which this note does not repeat) and the
compiler at `ca7cdc52`, and says how to lower each shape to Mesa's words in
one-session steps.

Read first: `mali-deqp-constprod.md` (the 533 cases and the 13 shapes),
`mali-constant-products.md`, `mali-vs-constant-products.md`,
`mali-deqp-constop.md` section 10 (the folds that landed, section 3 below),
`mali-arith-lowering.md` (the FS built-ins this overlaps), `mali-deqp-negzero.md`.

**What is measured and what is read.** The case counts are constprod-ref's
host measure at `9ac31f30` (HOST-ONLY: first refusal of the 10,597 refused
cases of the `37305608` slices, never a board result). The Mesa sequences are
read from the `text` of each `expect.json` (shim draw, `vs.varying` and
`fs.fragment` regions). **The refusing sites are read from the source at
`ca7cdc52`, not from a run**: no build was made for this note. constprod-ref
measured the sites at `9ac31f30` with a numbered temporary build; `glsl.rs`
and `nir.rs` have not moved since (the line numbers below are the tip's),
`nir_fs.rs:3745` is now `:3785`/`:3803`. Step 0 below makes the sites a
measured fact. A sequence that is long (`constchain_loop_vs` 123 words,
`fma_invariance_vs` 89, `fma_matmul_mat_vs`, `fma_random_trig_vs`) is
summarised by what decides its shape; the whole text is the `expect.json`.

Notation as in `mali-mixcol-lowering.md`: `.h0` the low half, `x.h00` a half
duplicated, `FADD.f32 rN.h0, x, 0x0.neg` Mesa's narrowing of an fp32 value to
fp16.

## 1. The references: Mesa's words, the refusing site, the mechanism, the step

Sites (tip): `glsl.rs` is the GLSL IR rewrite pass (`Tree`), `nir.rs` the
vertex walk and the shared `Nir::alu`, `nir_fs.rs` the fragment walk. Steps
are section 4's (S1 to S8). "FS half" is the isOk harness / colour program,
which compiles already.

| reference (case) | Mesa's words (what decides them) | site (tip) and text | mechanism | step |
|---|---|---|---|---|
| `litfirst_vs` (`random.basic_expression.vertex.57`: `v_color = vec4(a, a, -0.0 * b, a)`) | VS: one load of `a`, **one** store: lanes x, y, w are the one varying (`LD_ATTR_IMM.f32`, `STORE`), no word for lane z. FS: `LD_VAR_BUF_IMM.f32`, `FADD.f32 r0.h0, r0^, 0x0.neg`, `MKVEC.v2i16 r2, u2.w0.h0, r0^.h0`: z is a FAU constant (+0.0), no product | `glsl.rs:1681` (`!placed`: `fmul(fneg(0.0), b)`), const product | `fmul(a, ±0.0)` is `0.0` (`nir_opt_algebraic.py:277-278`); the lane is a constant, so the varying carries no lane z | S1 |
| `litfirst_array_vs` (`functions.array_arguments.global_in_float_vertex`) | VS: four `fneg` of the f2f16 lanes (`FADD.v2f16 r0, r0^.neg, 0x0.neg`, twice, then `F16_TO_F32` x4): `f * vec4(arr)` with `f == -1.0`. FS half: isOk of the four lanes against uniforms (`FADD.v2f16 .. u2.w1.neg`, `FMA.v2f16 .. u3.w0`, `FCMP_OR.v2f16.ge.m1`, `LSHIFT_AND.v2i16`, `MUX.v2i16`) | `glsl.rs:1681` (`!placed`), const product | `fmul(a, -1.0)` is `fneg(a)` (:296); a negated literal first is `fneg(fmul(c, x))` (:2387), the product gone when `c == 1.0` | S1 |
| `neglit_pair_vs` (`functions.overloading.user_func_arguments_vertex`: `func(in0) * func(-0.5, -2.0)`) | VS: load `in0`, **one** `FADD.f32 r0, r0^.neg, 0x0.neg` (`-in0`), store. The product of the two literals is the constant 1.0 and `x * 1.0` is `x` | `glsl.rs:1609` (both factors `fneg(ConstF32)`), const product | `fmul(fneg(a), fneg(b))` is `fmul(a, b)` (the :2387 rule twice, then `fneg(fneg(a))`, :230); two constants fold to their f32 product, `-0.5 * -2.0 = 1.0` | S2 |
| `neglit_pair_fs` (`..._fragment`) | FS: `FADD.f32.discard r0.h0, r0^, 0x0.neg`, `FADD.v2f16 r0, r0^.neg.h00, u2.w0.neg.h00`: the `-in0` is the isOk compare's `fadd(neg(x), -ref)`, no product | `nir_fs.rs:1433` (`neg(x) && neg(y)`, the mediump walk's twin of `nir.rs:2378`), const product | the same rule in the fragment walk | S2 |
| `chainlit_vs` (`random.conditionals.vertex.34`) | VS: `FADD.f32 r1, 0x40000000.neg, r1^.neg`, `FADD.f32 r0, u5.w1, r0^.neg`: `fadd(<-2.0, 7.0>, fneg(v))` per lane; no product survives | `glsl.rs:1681` (`chain`: the literal's other factor is a product), const product | `x * y * -c` is `fneg(fmul(fmul(x, c), y))` and the constants fold into the sum: Mesa takes `x * c` first (mali-constant-products.md section 1, `negmul_three`) | S2 |
| `chainlit_fs` (`random.conditionals.fragment.34`) | FS: `V2F32_TO_V2F16` x2, `FADD.v2f16 r0, u2.w0, r0^.neg.h10`, `FADD.v2f16 r1, u2.w1, r1^.neg`: a sum of a folded constant pair and the negated lanes, lane order `.h10` swapped | `glsl.rs:1681` (`chain`), const product | as `chainlit_vs` at fp16 | S2 |
| `constchain_degrees_vs` (`operator.angle_and_trigonometry.degrees.highp_vec2_vertex`) | VS: **both constants kept**: `FMA.f32 r0, r3^, u5.w1, 0x0.neg` (`x * 57.29578`), `FADD.f32 r0.h0, r0^, 0x0.neg`, `MKVEC.v2i16 r0, 0x0.h0, r0^.h0`, `FMA.v2f16 r0, r0^, u6.w0, 0x38000000.h11` (`* 0.04 + 0.50`, fp16 pair). The highp-to-mediump narrowing between the two is a node the constant rule does not see through | `nir.rs:2414` (`user && ... is_const_product`), const product | **none to fold**: the compiler must not refuse where the conversion separates the constants (`is_const_product` is a refusal too wide here) | S3 |
| `constchain_attrib_vs` (`default_vertex_attrib.vec4.vertex_attrib_1f`) | VS: `FMA.f32 r, a, 0x3DCCCCCD, 0x3F800000` per lane (`a / 10 + 1`), `FMA.f32 r3, r3^, 0x38000000.h1, 0x0.neg` (`* 0.5`), `FADD.f32 r0, r0^, r2`, `FMA.f32 r0, 0x34000000.h1, r0^, 0x0.neg` (`0.25 * (x + z)`: the two `0.5`s of `(x * 0.5 + z * 0.5) * 0.5` meet outside the add) | `nir.rs:2414`, const product | the factoring `fadd(fmul(a, b), fmul(a, c))` is `fmul(a, fadd(b, c))` (`nir_opt_algebraic.py`), then the product's constants fold (`0.5 * 0.5`) | S3 |
| `constchain_loop_vs` (`invariance.highp.loop_0`) | VS: unrolled; `value *= 1.9` is `FMA.f32 r4, r4^, u3.w0, 0x0.neg` (one FAU constant, no folding of `1.9` with another constant), the accumulation `FADD.f32`; 14 iterations x (FMA, FADD) per lane | `nir.rs:2414`, const product | **none to fold**: each `value *= 1.9` has one constant; the `is_const_product` test sees a constant times a product with a constant across the loop's carried value. Step 0 names which operand is the second constant | S3 |
| `constchain_fs` (`random.comparison_ops.fragment.38`) | FS highp: `S32_TO_F32.discard r0, u2.w0`, `FMA.f32 r0, r0^, r2^, 0x0.neg`, `FMA.f32 r2, u2.w1, r3^, 0x0.neg`, `FMA.f32 r0, 0x44000000.neg.h1, r1^, 0x0.neg` (`-2.0` as a widened half, `.neg.h1`), each narrowed `FADD.f32 .h0`, `MKVEC.v2i16` | `nir.rs:2414` (the highp FS shares `Nir::alu`), const product; its int uniform then meets `IntValue` (intval) | the :2414 refusal is a guard against a shape Mesa folds; here the words show the products are kept | S3 |
| `reassoc_mixed_vs` (`random.all_features.vertex.94`) | VS: `FREXPE.f32.log`, `FLOGD.f32` (a `log2`), `FADD_IMM.f32 r1, r1^, #0xC0600000`, `FMA.f32 r1, r6^.neg, r3^, r1^`, `FADD.f32 r0, r3^, r0^`, `FMA.f32 r0, u6.w1, r2^, r0^`: the `mixed` reassociation leaves one `FADD_IMM` per lane and constants as FAU halves (`F16_TO_F32 r1, u5.w1.h1`) | `glsl.rs:1492` (`mixed`), const product | `reassociate_operands` over a sum mixing `+` and `*` (`opt_algebraic.cpp`'s `reassociate_constant` + the operands pass); the reference gives the words, the rule is not transcribed | S3 |
| `foldcall_reflect_fs` (`constant_expressions.builtin_functions.geometric.reflect_float_fragment`: `const float cval = reflect(1.0, 1.0)`) | FS (isOk): `FMA.f32.discard r0, u2.w1, u2.w0.abs, u2.w1`, `FADD.f32 r1, 0x3F800000.neg, u2.w0.neg`, `FCMP_OR.f32.ge.m1 r0, r0^, r1^.abs, 0x0`, `MUX.v2i16`: the constant `-1.0` is the immediate of the FADD, **no ALU for the call** | `nir_fs.rs:3803` (the front end's `ffma` by a constant 1.0 / -1.0: `reflect`'s expansion on constants), const product | GLSL IR's constant evaluation of the call (`reflect` is `I - 2 * dot(N, I) * N`: dot, products, difference, f32); the walk must see a constant | S4 |
| `foldcall_acos_fs` (`...trigonometry.acos_float_fragment`) | FS: `FMA.f32 r0, u2.w1, u2.w0.abs, u2.w1` (the harness's tolerance), `FCMP_OR.f32.ge.m1 r0, r0^, u2.w0.abs, 0x0`: the folded `acos` is the FAU word `u2.w0` | `nir_fs.rs:3803`, const product | constant `acos`, f32, as above | S4 |
| `fma_foldcall_acos_vs` / `_atan_vs` / `_reflect_vs` / `_refract_vs` (`..._vertex`) | VS: **no `vs.varying` region** (the constant varying is linked into the FS as a FAU word); FS: `FADD.v2f16.discard r0, u2.w0.abs.h00, 0x0.neg`, `FMA.v2f16 r0, u2.w1, r0^.h00, u2.w1`, `FCMP_OR.v2f16.ge.f1 ...`; `atan` adds `FADD.v2f16 r1, u3.w0, r1^.neg.h00` (a second constant), `reflect`/`refract` `FADD.v2f16 r1, 0x3C000000.neg.h11, u2.w0.neg.h00` (the reference value `1.0` as an fp16 immediate) | `nir.rs:4781` (`AluOp::Ffma` catch-all of the vertex walk), fused multiply-add | the call's constant operands reach the walk as an `ffma` of constants; `Nir::alu_sized` folds that (section 3), the catch-all is before it | S4 |
| `fma_foldcall_mod_vs` (`common.mod_float_vertex`: `mod(87.65, 3.7)`) | FS: `IADD_IMM.i32 r0, 0x0, #0x411A411A` (the harness's reference, 2.5508 in fp16), `FADD.v2f16 r0, r0^, u2.w0.neg.h00`, the same compare; VS has no word | `nir.rs:2649` (`vs_fmod`, `neg_floor` not an `fneg`: a constant), fused multiply-add | the constant `mod`: `x - y * floorf(x / y)` in f32 (`ce_mod_vec2_float_fs` is the landed twin) | S4 |
| `fma_trig_acos_vs` (`operator.angle_and_trigonometry.acos.highp_float_vertex`) | VS fp32: `FMA.f32 r0, r2.abs, u5.w1, r0^` x3 (`0.074261`, `-0.018729`, `-0.212114`, `1.570729`, Horner in `|x|`), `FADD.f32 r1, 0x3F800000, r2.neg.abs`, `FRSQ.f32` then `FRCP.f32` (a sqrt), `FMA.f32 r0, r1^.neg, r0^, u7.w1` (`pi/2 - sqrt * poly`), the sign fix-up `FCMP_OR.f32.lt.f1` x2, `FADD`, `FMA ... u7.w1` | `nir.rs:4781`, fused multiply-add | the front end's expansion of `acos` (`builtin_call.rs:1384` `asin`, `:1094`), per lane, as `ffma` terms the walk accepts for `acos`/`asin` of a **vertex** value | S5 |
| `fma_trig_atan2_vs` (`atan2.highp_float_vertex`) | VS fp32: `FADD.f32 r1, r0.abs, 0x0.neg`, `FCMP_OR.f32.ge.m1`, `CSEL.u32.eq` x2 (swap), `FRCP.f32`, 5-term even Horner `FMA.f32 ... u6.w1, u7.w0 ...` in `x^2`, copysign `LSHIFT_AND.i32` x2 + `LSHIFT_OR.i32`, quadrant `FMA.f32 r0, r2^, u10.w1, r0^`, `FMIN.f32`, `CSEL` | `nir.rs:4781`, fused multiply-add | `builtin_call.rs:1292` `atan`, `:1334` `atan2` at the vertex stage | S5 |
| `fma_random_trig_vs` (`random.trigonometric.vertex.22`) | VS: per lane `FRCP.f32`, `FCMP_OR.f32.ge.m1 1.0 >= |x|`, `CSEL`, 5 `FMA.f32` (Horner), `CSEL`, `FMA` (the `atan` chain), copysign by `LSHIFT_AND.i32` / `LSHIFT_OR.i32` | `nir.rs:4781`, fused multiply-add | as `fma_trig_atan2_vs` for four lanes | S5 |
| `fma_matmul_const_vs` (`matrix.mul.const_highp_mat2_mat2_vertex`) | VS: no varying region; FS: `NOP`, `ATEST`, `MOV.i32 r0, u2.w0`, `MOV.i32 r1, u2.w1`, `BLEND`: **the whole product is a constant colour** | `nir.rs:4781`, fused multiply-add | the constant matrix product folds (the front end's `ffma` terms of constants, `alu_sized`: the S4 routing); listed in S6 because its sibling cases are dynamic | S6 (needs S4's routing) |
| `fma_matmul_dynamic_vs` (`mul.dynamic_highp_mat3_vec3_vertex`) | VS fp32: `FMA.f32 r3, r4^, r2, 0x0.neg` (first term a product), then `FMA.f32 r3, r0, r4^, r3^` per further term; a `FADD.f32 r4, r8^, r1` where a factor is 1.0 (the `1.0`-column of the matrix) | `nir.rs:4781`, fused multiply-add | Mesa keeps the first term a `fmul` (`FMA .. 0x0.neg`) and the others `ffma`; `fadd` for a `1.0` factor; the recogniser (`is_mat_term`, `mat_column`) knows `vec4(position, 1.0)`-shaped products only | S6 |
| `fma_matmul_mat_vs` (`mul.dynamic_lowp_mat3_mat3_vertex`) | VS fp16: `V2F32_TO_V2F16`, `FMA.v2f16 r4, r0, u6.w1.h11, 0x0.neg`, `FMA.v2f16 r4, r1, u6.w1.h00, r4^` ..., `FADD.v2f16` of the two chains, column by column, a `FADD.f32 r0.h0, r2^, 0x0.neg` for the third lane | `nir.rs:4781`, fused multiply-add | the same term order at fp16, constants as FAU halves `.h00/.h11` | S6 |
| `fma_mulassign_vs` (`mul_assign.dynamic_highp_mat2_vertex`) | VS fp32: `FMA.f32 r2, r0, u6.w0, 0x0.neg`, `FMA.f32 r2, r4, u5.w1, r2^`, per row | `nir.rs:4781`, fused multiply-add | `m *= m2` is the same term chain | S6 |
| `fma_geom_reflect_vs` (`geometric.reflect.highp_float_vertex`) | VS fp32: `FADD.f32 r1, r0, r0` (`2 * d`), `FMA.f32 r0, r4, r0^, 0x0.neg` (`a * d`), `FMA.f32 r0, r1^.neg, r0^, r4^` (`a - 2d * (a*d)`): `fneg(fadd(d, d))` as the `.neg` of the `ffma`, for scalars | `nir.rs:4781` (`is_reflect` does not match this form), fused multiply-add | `reflect` of non-vector operands (`is_reflect`, `nir.rs:2595`, takes a dot of vectors) | S7 |
| `fma_geom_refract_vs` | VS fp32: `FMA.f32 r1, r0, r4, 0x0.neg`, `FMA.f32 r2, r1.neg, r1, 0x3F800000`, `FMA.f32 r2, r7, r2^, 0x0.neg`, `FMA.f32 r2, r7.neg, r2^, 0x3F800000`, `FRSQ.f32` + `FRCP.f32`, two `FMA`, `FCMP_OR.f32.lt.m1 r1, r2^, 0x0, 0x0`, `CSEL.u32.eq r0, r1^, 0x0, r0^, 0x0` (the total-internal-reflection zero) | `nir.rs:4781`, fused multiply-add | the front end's `refract` expansion, scalar form | S7 |
| `fma_invariance_vs` (`invariance.highp.subexpression_precision_highp`) | VS fp32: a `distance`/`dot`/`reflect`-of-`FADD_IMM` sequence of 89 words: `FMA.f32` products of FAU constants, `FADD.f32 .neg` differences, `FRSQ`/`FRCP`, `FROUND.f32.rtn` + `FADD.f32 .neg` (a `fract`), `CSEL.u32.eq` | `nir.rs:4781`, fused multiply-add | `refract` / `distance` / `fract` in one shader; shares S7's `refract` | S7 |
| `litfirst_mat_vs` (`functions.datatypes.mat2_vertex`: `return -2.0 * a`) | VS fp16 (mediump `mat2`): `FADD.v2f16 r0, r0^, r0^` (`2a`), `FADD.v2f16 r0, r0^.neg, 0x0.neg`: **`fneg(fadd(a, a))`**, per column pair, then `F16_TO_F32` x4 to the varyings | `glsl.rs:1681` (`!placed`, `w > 1`), const product | `fmul(fneg(2.0), a)` is `fneg(fmul(a, 2.0))` (:2387), `fmul(a, 2.0)` is `fadd(a, a)` | S8 |
| `litfirst_mat_fs` (`..._fragment`) | FS isOk: `FMA.v2f16 r0, 0x40000000.neg.h11, r0^, u2.w0.neg` (the `-2.0` kept as a `.neg.h11` immediate in an `FMA`: the harness's `-ref` addend takes the product) | `glsl.rs:1681` (`!placed`, `w > 1`), const product | the fragment half keeps the product (`ffma(-2, a, -ref)`), the VS half the `fneg(fadd)`: the two stages differ, as `negmul_*_lanes*` pin | S8 |
| `alg_constmat_fs` (`matrix.mul.const_highp_mat2_float_fragment`: `mat2(0.6, 0.7, -1.0, 0.4) * v`) | FS: `FMA.f32 r1, u2.w0, r0, 0x0.neg` (`0.6 * v`), `FMA.f32 r2, r0, u2.w1, 0x0.neg` (`v * 1.1`, `0.7v + 0.4v` factored, `1.1f` = `0x3f8ccccd`), `FADD.f32 r0.h0, r0^.neg, 0x0.neg` (the `-1.0` column is an `fneg`), narrowed per lane | `nir_fs.rs:1491` (`AlgebraicForm`: the factoring takes a sum other than 1.0) | `fadd(fmul(a, b), fmul(a, c))` is `fmul(a, fadd(b, c))`, the sum folded in f32 and kept (`b + c` as a constant, rounded once) | S8 |

All 31 are mapped: S1 2, S2 4, S3 5, S4 7, S5 3, S6 4, S7 3, S8 3.

## 2. The refusal sites at the tip, grouped by the mechanism they need

| site (tip) | what it refuses | mechanism | refs |
|---|---|---|---|
| `glsl.rs:1609` | two negated-literal factors of one product | fold `c1 * c2`, drop two `fneg` | `neglit_pair_*` |
| `glsl.rs:1681` | a negated literal first (`!placed`), clamped, or a product of a product (`chain`) | leave `fmul(fneg(c), x)` to the walks as `fneg(fmul(c, x))`; lit `±0.0` is a constant; chain takes `x * c` first | `litfirst_*`, `chainlit_*` |
| `glsl.rs:1492` (`mixed`) | a reassociation across `+` and `*` | transcribe `reassociate_operands` for the mixed case | `reassoc_mixed_vs` |
| `nir.rs:2378`, `nir_fs.rs:1433` | `fmul(fneg(a), fneg(b))` | `fmul(a, b)` | `neglit_pair_*` |
| `nir.rs:2414` (`is_const_product`) | a constant times a product with a constant, `user` values | refuse **only** what Mesa folds; leave the rest as `fmul`s | `constchain_*` |
| `nir_fs.rs:1491` | a factoring with a sum other than 1.0 | fold the f32 sum | `alg_constmat_fs` |
| `nir_fs.rs:3785`, `:3803` | the front end's `ffma` of constants by `1.0`/`-1.0` in the fragment walk | the operand is constant: fold (S4) | `foldcall_*_fs` |
| `nir.rs:2649` (`vs_fmod`) | `mod` of a constant (`neg_floor` is no `fneg`) | fold | `fma_foldcall_mod_vs` |
| `nir.rs:4781` (`AluOp::Ffma` catch-all) | every `ffma` the vertex walk has no arm for | per shape: fold constants (S4), the trig chains (S5), the matrix term chain (S6), `reflect`/`refract` (S7) | `fma_*` |

The three texts: `ArithConstant` 386 (S1, S2, S3, S4's two fragment halves,
S8's two), `FusedMultiplyAdd` 139 (S4's vertex halves, S5 to S7),
`AlgebraicForm` 8 (S8).

## 3. What constop landed, and what step 4 reuses

`lower-constop` (commit `0fd576ea`, "constant operations on dEQP's built-ins,
booleans and -0.0 fold as Mesa's front end folds them") landed
mali-deqp-constop.md section 10's steps 1 and 2 in `nir.rs`, so at the tip:

- `Nir::alu_sized` (`nir.rs:2111-2280`) folds an operation whose operands are
  all `Val::Const`: `Fadd`, `Fmul`, unfused `Ffma` (`f(0) * f(1) + f(2)`,
  `:2166-2175`, Mesa's `ir_expression_operation.py:755`), `Frcp`, `Frsq`
  (correctly rounded), `Fmin`/`Fmax`, `Fclamp`, the compares, `Bcsel`, `B2f`,
  `Ffloor`/`Fceil`/`Ftrunc` through `fold::fold_float`. It keeps refusing
  `Fsin`, `Fcos`, `Flog2`, `Fexp2`, `Fpow` (`ConstantFold`) and every
  non-`plain` operand.
- `const_dot` (`nir.rs:2801`) is the `result = a0*b0; += a1*b1` fold of GLSL's
  `dot`, unfused; the vertex walk's `Fdot*` arm uses it.
- `deqp_constop_ce_{floor,ceil,fract,sign,inversesqrt,mod,smoothstep,asin,dot,
  normalize}_fs` are un-ignored: the fragment walk's folds of those built-ins
  are byte-equal. **`ce_atan_separate_fs` is still ignored**: folding the front
  end's `atan` expansion gives `0xc016cbd6`, Mesa's GLSL body `0xc016cbe4`
  (`tests/mali_deqp_constop.rs:24`): the bits depend on the GLSL IR body's
  operation order, not on our NIR-style expansion (constop section 10.2 R7).

Step 4 (constant folding of `acos`, `asin`, `atan`, `atan2`, `reflect`,
`refract`, `mod`) is therefore **not a new fold**: `acos`, `asin`, `mod`,
`dot` of constants fold in the fragment walk today. What is refused is the
**route**: the constant call reaches a walk's `ffma` arm before
`alu_sized` is asked.

- `nir.rs:4781`: the vertex walk matches `AluOp::Ffma` by shape (`is_fmod`,
  `is_cross`, `is_mix`, `is_reflect`, ...) and refuses the rest; with all
  four operands `Val::Const` none of the arms must be tried: ask
  `vs.alu(Alu::Ffma, x(0), x(1), x(2))` (`alu_sized` folds), as the arms for
  `is_cross`/`is_mix` do for non-constants.
- `nir.rs:2649` (`vs_fmod`): the same: a constant `neg_floor` makes the whole
  `mod` a constant.
- `nir_fs.rs:3785-3803`: the any-other-`ffma` guard (a constant `1.0`/`-1.0`
  scale) looks at lane values; a lane whose operands are all constants is the
  fold's.

**Owner: constprod S4, one step.** Reasons: (a) the refusals are
`ArithConstant`/`FusedMultiplyAdd` raised in the vertex and fragment walks, the
sites S4 edits; constop's `alu_sized` and `const_dot` are the code it reuses,
unchanged; (b) lower-constop is finished and no constop step is open for these
shapes (its open items are `ce_atan_separate_fs` and the `ConstantFold` ones
below); (c) giving the 48 cases to a constop step would reopen a closed item.
**The reuse is exact**: S4 adds no folding arithmetic. It adds the route
(`all operands Val::Const` is `alu_sized`'s) and the fp16 rounding of the
fragment harness's reference.

**The boundary with constop's `const_call_*` shapes and with cfold2.**
`mali-deqp-families.md` section 8's `const_call_*` (`common` 94, `transcendental` 32, `geometric` 24, `other` 18) are the same
`constant_expressions.builtin_functions.*` cases seen at **their first
refusal in the tip's order**: `const_call_transcendental` 32 and
`const_call_geometric` 24 include these `acos/asin/atan/reflect/refract`
cases. Where their first refusal is `ConstantFold` (the value is not foldable:
`sin`, `cos`, `log`, `exp`, `pow`, `tan` of constants, `Fsin`... in
`alu_sized`), the owner is the cfold items (`cfold-ref`, running, will write
`docs/notes/mali-deqp-cfold2.md`; it does not exist at `ca7cdc52`).
**The split is by the refusal text a case reaches first**:
`ArithConstant`/`FusedMultiplyAdd` (the route) is S4's; `ConstantFold` (the
value) is cfold's. S4 must not add `Fsin`/`Fcos`/`Flog2` folds; cfold must not
touch the walk's `ffma` arms. If cfold's lowering lands first and fixes the
route as a side effect, S4 reduces to re-pinning the seven references (step 0
tells). **S4's numeric risk is `atan`**: `fma_foldcall_atan_vs` and
`foldcall_acos_fs` fold the front end's expansion; `acos`/`asin` are
byte-equal in constop's `ce_asin_fs`, `atan` is not (`0xc016cbd6` against
`0xc016cbe4`). The host probe of step 0 prints, for each of the seven
references, the folded f32 against the FAU word in the `expect.json`
(`u2.w0` of the FS region): `atan` stays refused (re-pinned, ignored) if the
bits differ, no fitted constant, as constop R7 decided.

## 4. Steps, ranked by cases

Counts are constprod-ref's first-refusal cases (533 = 386 + 139 + 8); a step
does not change a number of passing cases by itself, the random shaders meet
their next refusal (mali-deqp-constprod.md section 4, last paragraph). Each
step is one session: one rule in one or two files, its references byte-equal,
the `tests.rs` pins that name it re-pinned.

**Step 0 (before S1, no change).** Make the sites a measured fact: a probe
(`tests/mali_deqp_constprod.rs` run with `--ignored`, printing the `file:line`
of each reference's first refusal, a temporary numbered patch as
constprod-ref did) checks the table of section 1. It costs one build and
decides the three "decided by the probe" rows (`constchain_loop_vs`,
`constchain_fs`, `litfirst_array_vs`).

| step | cases | shapes | files | what | un-ignores |
|---|---:|---|---|---|---|
| **S1** | 174 | a negated literal first in a scalar product (vertex and highp fragment) | `glsl.rs` `neg_products` (`:1597-1690`), `nir.rs` `Alu::Fmul` (`:2370-2420`) | stop refusing `!placed` for `w == 1`: leave `fmul(fneg(c), x)` to the walk as `fneg(fmul(c, x))`, as the mediump fragment does (`glsl.rs:1675`, `nir_fs.rs` `Fs::op`); `c == 1.0` is the product alone; `c == ±0.0` makes the lane the constant `+0.0` (`fmul(a, ±0.0)`, :277-278), so a varying with that lane carries one lane less | `litfirst_vs`, `litfirst_array_vs` |
| **S2** | 124 (91 + 33) | a product of two negated literals; a negated literal times a product | `glsl.rs:1609`, `:1681` (`chain`), `nir.rs:2378`, `nir_fs.rs:1433` | `fmul(fneg(a), fneg(b))` is `fmul(a, b)`; two `ConstF32` factors fold to their f32 product (`-0.5 * -2.0 = 1.0`, `x * 1.0 = x`); `x * y * -c` takes `x * c` first (negmul_three) and the constant sums fold | `neglit_pair_vs`, `neglit_pair_fs`, `chainlit_vs`, `chainlit_fs` |
| **S3** | 64 (62 + 2) | a constant times a product with a constant; the mixed reassociation | `nir.rs:2414` (`is_const_product`, `:2496`), `glsl.rs` `reassociate_constant` (`:1456`), `:1492` | decide per shape with the five references: `degrees(in0) * 0.04 + 0.50`, `value *= 1.9` and `x * 0.5` chains **compile with their constants kept**; refuse only what Mesa's GLSL IR rule folds (a `c1 * (x * c2)` visible in one expression) and transcribe `reassociate_constant` for that form; the `attrib` form is the factoring plus the product's constants (`0.25`); `mixed` at `:1492` per `reassoc_mixed_vs`'s words | `constchain_degrees_vs`, `constchain_attrib_vs`, `constchain_loop_vs`, `constchain_fs`, `reassoc_mixed_vs` |
| **S4** | 48 (+ the const share of S6's 41) | the built-in called on constants: `acos`, `asin`, `atan`, `atan2`, `reflect`, `refract`, `mod`, a constant matrix product | `nir.rs:4781`, `:2649`, `nir_fs.rs:3785-3803` | route constant operands to `alu_sized` (section 3); fp16 rounding of the harness's reference where the use is mediump (`mod`: `0x411A411A`); no new fold arithmetic. Owner: this step (section 3) | `foldcall_reflect_fs`, `foldcall_acos_fs`, `fma_foldcall_{acos,atan,reflect,refract,mod}_vs` (7; `atan` repinned if the bits differ) |
| **S5** | 47 | `acos`/`asin`/`atan`/`atan2` of a vertex value | `nir.rs:4781`, `builtin_call.rs:1094-1400` | the fp32 chains of `fma_trig_acos_vs`, `fma_trig_atan2_vs` as the front end's expansions deliver them to the vertex walk: accept the `ffma` terms of `asin`/`acos`/`atan` (the polynomial, the `FRSQ`+`FRCP` root, the `FCMP`/`CSEL` sign and quadrant); the fragment walk's `atan` front half is the model | `fma_trig_acos_vs`, `fma_trig_atan2_vs`, `fma_random_trig_vs` |
| **S6** | 41 | the matrix product of vertex attributes and constants beyond `vec4(position, 1.0)` shapes | `nir.rs:4700-4781` (`is_mat_term`, `mat_column`, `vs_two_terms`) | first term `fmul` (`FMA .. 0x0.neg`), the others `ffma`, `fadd` for a `1.0` factor; fp16 chains for lowp (`FMA.v2f16 .. .h11/.h00`); a constant product folds through S4's route | `fma_matmul_const_vs` (after S4), `fma_matmul_dynamic_vs`, `fma_matmul_mat_vs`, `fma_mulassign_vs` |
| **S7** | 19 | scalar `reflect`/`refract`/`distance`/`fract` of attributes | `nir.rs:4781`, `is_reflect` (`:2595`) | `fneg(fadd(d, d))` as the `.neg` of the `ffma`; `refract`'s `FRSQ`+`FRCP`, `FCMP_OR.f32.lt.m1`, `CSEL` zero; `fract` as `FROUND.f32.rtn` + `FADD .neg` | `fma_geom_reflect_vs`, `fma_geom_refract_vs`, `fma_invariance_vs` |
| **S8** | 16 (8 + 8) | `-2.0 * a` for a vector or matrix; the factoring's sum other than 1.0 | `glsl.rs:1681` (`w > 1`), `nir_fs.rs:1491` | VS: `fneg(fadd(a, a))` at fp16; FS keeps the `ffma(-2, a, -ref)`; the factoring's f32 sum kept as a constant, the `-1.0` column an `fneg` | `litfirst_mat_vs`, `litfirst_mat_fs`, `alg_constmat_fs` |

174 + 124 + 64 + 48 + 47 + 41 + 19 + 16 = **533**. Order: S1, S2, S3, S4,
S5, S6, S7, S8 (S6 after S4 for `fma_matmul_const_vs`). S1 to S3 share
`glsl.rs` `neg_products` and `Nir::alu`'s `fneg` rules, so they are three
sessions in that order, not one; S4 to S7 touch the vertex walk's `ffma`
catch-all (`nir.rs:4781`) and so are serial among themselves, but S4 may go
first in parallel with S1 to S3 (different arms).

**Forecast (HOST-ONLY).** Cases that **pass the compile** after their step
(no other refusal) are the ones the site decides alone: S4's 48 (the
`constant_expressions` library cases, whose partner is the isOk harness that
compiles today), S5 to S7's 107 (15 random). S1 to S3's cases are mostly random shaders
(330 of the 533): they move to their next refusal (`Next` in
`mali-deqp-families.md`), and how many then pass is not forecast here. Nothing here is a board result; the
number to rerun is `tools/mali/deqp-host-measure.sh` (constprod-ref
section 1) with the three texts as the filter.

## 5. Overlap with other items

- **arith-lower-1 / mali-arith-lowering.md.** Its row 7 (48 cases) is the
  **fragment** `acos/asin/atan/atan2/faceforward/refract` of varyings
  (`op_angle_*_fs`), the same fp32 chains as S5/S7 in the vertex walk. The
  expansion in `builtin_call.rs` is shared: whichever of arith step 7 and S5
  lands first fixes the chain's `ffma` order; the other reuses it. They edit
  different walks (`nir_fs.rs` against `nir.rs:4781`) but both touch
  `builtin_call.rs`: land them serially and rebase. Arith step 1b (fp32
  transcendentals of a lane, `FRCP`/`FRSQ`) must not change `Frsq`/`Frcp` in
  `alu_sized` (S4's folds depend on `Frsq`'s correctly rounded fold).
  `foldcall_*_fs` are `ArithConstant` here and not in arith's table.
- **negzero-lower / mali-deqp-negzero.md.** `litfirst_vs` (`-0.0 * b`) is the
  zero sign of a **product** (`fmul(a, ±0.0)` is `0.0`, :277-278); constop
  section 10.6(e)(2) tells negzero not to fold a product's zero sign through the
  compare rewrite. S1 owns the `neg_products` side (the literal `±0.0` times a
  value is `+0.0`); negzero-lower owns the compare rewrites and the
  initialiser folds (`glsl.rs:1841-1912`). Both edit `glsl.rs` near
  `neg_products` and `plus_zero` (called from `:177`): serial, S1 first, or
  rebased on whichever is merged.
- **constop / cfold2.** Section 3: S4 owns the route, cfold the values.
- **intval (`IntValue`).** `constchain_fs` and the `foldcall_*_fs` harness
  halves read int uniforms or bool compares: they may compile only after
  the intval steps (`mali-deqp-intval.md`); each reference's FS half compiles
  today per constprod-ref, so the byte-compare is not blocked.
- **posvary.** The `foldcall_*_vs` and `matmul_const_vs` VS halves have **no
  `vs.varying` region**: a constant varying linked as a FAU word of the FS
  (`mali-varying-constants.md`). That exists at the tip; posvary-lower's
  "uniform-only varyings" must keep it.

## 6. glmark2 corpus verdicts at risk (`tests/mali_glmark2_corpus.rs`)

`tests/fixtures/mali_glmark2_verdicts.txt` (97 lines, 62 `ok`) pins the first
refusal per scene and shader; the test is "only move forward". Three rows
carry the `ArithConstant` text: `:17` `shading=phong fs 8a64`, `:59`
`jellyfish vs 08a2`, `:78` `refract fs 3fae`. Each is the first refusal of its
shader: a step that reaches it moves it to its next refusal or to `ok`; the
fixture moves in the same commit (`MALI_GLMARK2_WRITE=1`). By reading: `refract
fs 3fae` is S2 or S3 (`ffma`/`fmul` of constants in the highp walk), `phong`
the highp `Nir::alu` guard `:2414` (S3), `jellyfish vs 08a2` a vertex shader's
negated literal or constant chain (S1 to S3); step 0 names the site. **At
risk the other way** are the 62 `ok` rows: S1 to S3 relax `neg_products`,
`Nir::alu`'s `Fmul` arms and `is_const_product`, which every `ok` program
with a negated product passes through. The tests pinning those rules are the
`negmul_*` family (`negmul_three`, `negmul_highp_lanes*`,
`negmul_mediump_lanes*`, `negmul_vs_lanes`, `negmul_vs_neg`), `gm2_fsmisc_6`
(a literal first in a mediump fragment), `algfold_189_mulone` and
`gm2_arithconst_*`; each step runs them with the corpus verdict test
(`cargo test --test mali_glmark2_corpus`, with the corpus) and a changed `ok`
word is a regression, not a re-pin. S4 touches the `ffma` arms
(`is_mix`, `is_cross`, `is_reflect`, `is_mat_term`) that glmark2's `ok` shaders
use (`gm2_fma_*`, `gm2_arithconst_1`): route only when **all** operands are
constants, leaving every other arm as it is.

## 7. What this design does not know

- The sites are read, not run (section 1): `constchain_loop_vs`,
  `constchain_fs` and `litfirst_array_vs` could reach another site than the
  one named; step 0 decides.
- The case split inside the 41 `matmul` cases (constant against dynamic
  operands) and inside the 47 trig cases between highp and mediump
  (mediump `acos` at the vertex stage is fp32 at both precisions per
  `mali-arith-lowering.md`) is not measured; only the pinned references are.
- Whether S3's `reassociate_constant` transcription is needed or the existing
  `glsl.rs:1456` suffices is decided by the five references, not by this note.
- Mesa's GLSL sources are not vendored (`third_party/mesa-26.2.1` has no
  `builtin_functions.cpp`): the S4 numeric check is the only way to see the
  `atan` body.
- The G310 needs nothing separate: the references are the G610's
  (`mali-g310-vs-g610.md`).

## 8. Step 0 at c0142485

Step 0 of section 4, done without a compiler change. Method: every
`tools/mali/testdata/deqp_constprod_*` reference (31) and the eleven
`deqp_fmaloc_fma_*` ones that `mali-lowering-schedule.md` section 4a gives to
constprod (42 in all) went through `mali_ref::Reference::load` and
`mali_ref::compile_pair`, as `tests/mali_deqp_constprod.rs` does (features
`host-test,gpu-mali,shaderc-online,no-shader-table`). The site is a throwaway
`eprintln!("SITE file:line")` at every `MaliError::{ArithConstant,
FusedMultiplyAdd, AlgebraicForm}` construction in `glsl.rs`, `nir.rs` and
`nir_fs.rs` (the one `ok_or` made lazy), reverted and not committed; the probe
test was not committed either. Each reference printed exactly one `SITE`
line, the one of its error, so no earlier site was passed over. `K1` to `K8`
are the schedule's names for `S1` to `S8` of section 4.

Texts: `A` = "a constant product in a shape Mesa's algebraic rules move or fold
..." (`ArithConstant`), `F` = "a fused multiply-add other than a matrix
product's (a built-in function's) ..." (`FusedMultiplyAdd`), `G` = "a shape
Mesa's algebraic rules rewrite without a reference ..." (`AlgebraicForm`).
Every site below is **measured at c0142485**; "= note" means the file:line of
section 1 is the measured one.

| reference | text | site (measured) | step | vs section 1 |
|---|---|---|---|---|
| `litfirst_vs` | A | `glsl.rs:1681` | K1 | = note |
| `litfirst_array_vs` | A | `glsl.rs:1681` | K1 | = note (a "probe" row: confirmed) |
| `neglit_pair_vs` | A | `glsl.rs:1609` | K2 | = note |
| `neglit_pair_fs` | A | `glsl.rs:1609` | K2 | **differs**: the note says `nir_fs.rs:1433`; the pair is refused earlier, in the GLSL rewrite pass, before the fragment walk |
| `chainlit_vs` | A | `glsl.rs:1681` | K2 | = note |
| `chainlit_fs` | A | `glsl.rs:1681` | K2 | = note |
| `constchain_degrees_vs` | A | `nir.rs:2414` | K3 | = note |
| `constchain_attrib_vs` | A | `nir.rs:2414` | K3 | = note |
| `constchain_loop_vs` | A | `nir.rs:2414` | K3 | = note (a "probe" row: confirmed) |
| `constchain_fs` | A | `nir.rs:2414` | K3 | = note (a "probe" row: confirmed) |
| `reassoc_mixed_vs` | A | `glsl.rs:1492` | K3 | = note |
| `foldcall_reflect_fs` | A | `nir_fs.rs:3803` | K4 | = note |
| `foldcall_acos_fs` | A | `nir_fs.rs:3803` | K4 | = note |
| `fma_foldcall_acos_vs` | F | `nir.rs:4816` | K4 | line only (note `:4781`) |
| `fma_foldcall_atan_vs` | F | `nir.rs:4816` | K4 | line only |
| `fma_foldcall_reflect_vs` | F | `nir.rs:4816` | K4 | line only |
| `fma_foldcall_refract_vs` | F | `nir.rs:4816` | K4 | line only |
| `fma_foldcall_mod_vs` | F | `nir.rs:2649` | K4 | = note |
| `fma_trig_acos_vs` | F | `nir.rs:4816` | K5 | line only |
| `fma_trig_atan2_vs` | F | `nir.rs:4816` | K5 | line only |
| `fma_random_trig_vs` | F | `nir.rs:4816` | K5 | line only |
| `fma_matmul_const_vs` | F | `nir.rs:4816` | K6 (after K4) | line only |
| `fma_matmul_dynamic_vs` | F | `nir.rs:4816` | K6 | line only |
| `fma_matmul_mat_vs` | F | `nir.rs:4816` | K6 | line only |
| `fma_mulassign_vs` | F | `nir.rs:4816` | K6 | line only |
| `fma_geom_reflect_vs` | F | `nir.rs:4816` | K7 | line only |
| `fma_geom_refract_vs` | F | `nir.rs:4816` | K7 | line only |
| `fma_invariance_vs` | F | `nir.rs:4816` | K7 | line only |
| `litfirst_mat_vs` | A | `glsl.rs:1681` | K8 | = note |
| `litfirst_mat_fs` | A | `glsl.rs:1681` | K8 | = note |
| `alg_constmat_fs` | G | `nir_fs.rs:1491` | K8 | = note |
| `fmaloc_fma_asin` | F | `nir.rs:4816` | K5 | not in section 1 |
| `fmaloc_fma_atan` | F | `nir.rs:4816` | K5 | not in section 1 |
| `fmaloc_fma_atan2` | F | `nir.rs:4816` | K5 | not in section 1 |
| `fmaloc_fma_attrmat_vec` | F | `nir.rs:4816` | K6 | not in section 1 |
| `fmaloc_fma_attrmat_unimat` | F | `nir.rs:4816` | K6 | not in section 1 |
| `fmaloc_fma_constmat_vec` | F | `nir.rs:4816` | K6 | not in section 1 |
| `fmaloc_fma_constmat_mat` | F | `nir.rs:4816` | K6 (constant product: after K4's routing, like `fma_matmul_const_vs`) | not in section 1 |
| `fmaloc_fma_const_matmat` | F | `nir.rs:4816` | K4 (constant fold) | not in section 1 |
| `fmaloc_fma_const_asin` | F | `nir.rs:4816` | K4 | not in section 1 |
| `fmaloc_fma_const_reflect` | F | `nir.rs:4816` | K4 | not in section 1 |
| `fmaloc_fma_reflect_float` | F | `nir.rs:4816` | K7 | not in section 1 |

(The step of the eleven `fmaloc_fma_*` rows is by shape, from
`mali-deqp-fmaloc.md` section 3's descriptions; their site is measured.)

What the sites say:

- **One difference of substance:** `neglit_pair_fs` is refused at
  `glsl.rs:1609`, not `nir_fs.rs:1433`. K2's `glsl.rs` change reaches both
  `neglit_pair` references; the `nir_fs.rs:1433` arm (`neg(x) && neg(y)`)
  is not reached by the reference and K2 must pin it with another shape or
  leave it.
- **The other differences are line shifts only:** `nir.rs:4781` of section 1
  is `nir.rs:4816` at this tip (same `AluOp::Ffma => return
  Err(FusedMultiplyAdd)` catch-all); no `file` changed. All 31 + 11
  references are refused at one of eight sites: `glsl.rs` 1492, 1609, 1681;
  `nir.rs` 2414, 2649, 4816; `nir_fs.rs` 1491, 3803.
- **The three "probe" rows are confirmed, none reaches another site:**
  `constchain_loop_vs` and `constchain_fs` are both at `nir.rs:2414`;
  `litfirst_array_vs` at `glsl.rs:1681`. (The site says where the first
  refusal is, not that the shader compiles after K1/K3: a later site may
  appear then, as for every row.)
- **Counts by site:** `nir.rs:4816` 25 (14 + 11 `fmaloc_fma_*`), `glsl.rs:1681`
  6, `nir.rs:2414` 4, `glsl.rs:1609` 2, `nir_fs.rs:3803` 2, and one each
  `glsl.rs:1492`, `nir.rs:2649`, `nir_fs.rs:1491`.

**S3: is the `reassociate_constant` transcription needed?** `glsl.rs` already
has `reassociate_constant` and the non-mixed `reassociate_operands`
(`glsl.rs:1456-1500`). The measured refusal of `reassoc_mixed_vs` is its
`mixed` arm (`glsl.rs:1492`: the reassociation across a change of width),
which the existing rule declines by an error instead of transcribing. So
the existing rule does not suffice for that reference: K3 needs the mixed
branch of `reassociate_constant`, nothing more of the function. The four
`nir.rs:2414` references are another thing: the site is the `is_const_product`
guard of `Nir::alu`, a refusal placed there because the GLSL pass was
presumed to have folded what Mesa folds; their Mesa words keep the constants
(section 1), so for them K3 removes or narrows the guard and does not
transcribe anything. Whether the guard can go without turning a shape that
Mesa folds into "compiled, not Mesa's" is not shown by the sites: it needs
the K3 byte-compare of each of the four, and a guard that stays for the shapes
`glsl.rs:1456` leaves unfolded.

Not done here (outside step 0): no reference was lowered or bytes compared;
no site beyond the first refusal of each reference was measured.
