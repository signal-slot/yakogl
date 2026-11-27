<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the open constant-fold steps K2, K4's rest, K5 and fix C, measured, with Mesa 26.2.1 references

**Status: design** (plan #387, item `kfold-refs`; issue #7). Nothing under
`src/` changed. Measured at the integration tip **`4d8a0b40`** (2026-11-27).
The references are `tools/mali/testdata/deqp_kfold_*` (24), pinned by
`tests/mali_deqp_kfold.rs`. HOST-ONLY: every count is a compile or link
outcome on this host, never a board result.

Train 2 (`arith-lower-2`) left these steps undone and its prototype diffs are
lost. The steps are those of `mali-constprod-lowering.md` (K2 = its S2,
K4 = S4, K5 = S5; `mali-lowering-schedule.md` 2.7) and fix C of
`mali-deqp-defects.md` section 4. Read those first; this note does not repeat
their reference tables. The shared fold helper is `src/shaderc/online/mesa_fold.rs`
(`mali-libm-fold.md`); it is declared and has one caller today
(`glsl.rs` `Tree::libm_lane`, negzero N2).

## 1. How this was measured

- **Host measure:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --out "$PWD/build/measure" --jobs 3
  --chunk 100` (the `HOST_DEQP_CTS` tree of `tools/cts/deps.toml`, VK-GL-CTS
  93bca01 with `cts-yakogl-target.patch`), the default case list (the 10,597
  board-refused cases of the slices `2026-11-15-t6-gles2-S{1..4}-37305608`).
  All cases began, none crashed; **4,709** carry a host refusal (4,716 at
  `e9033663`, `mali-deqp-families.md` section 19). First refusals of the four
  texts this note is about: `ArithConstant` 353, `FusedMultiplyAdd` 116,
  `Defect(LaneCycle)` 11.
- **Sites:** a scratch copy of the tree (outside the repository, not
  committed) in which every `Err(MaliError::ArithConstant)`,
  `Err(MaliError::FusedMultiplyAdd)` and `Err(MaliError::Defect(Defect::LaneCycle))`
  of `glsl.rs`, `nir.rs` and `nir_fs.rs` prints `file!():line!()` (the two
  `ok_or` made lazy, so only the returned error prints; one-line edits, so the
  line numbers are the tip's). Each of the 462 distinct refusing programs of
  the 480 cases (the `programs.tsv` of the measure, verbatim from the `.qpa`)
  went through `tests/mali_ref`'s `compile_pair`; each printed one site and was
  refused with its case's text. A case is counted at the site of its first
  program refused with its own text.
- **`constant_expressions.builtin_functions`:** all 430 cases (every one is in
  `tools/cts/gles2-mustpass.txt`) were generated from the dEQP templates
  (`es2fShaderConstExprTests.cpp`, `glsShaderConstExprTests.cpp`,
  `glsShaderLibraryCase.cpp`, `gluShaderLibrary.cpp` `genCompareFunctions` at
  93bca01) by a scratch generator that reproduces the `.qpa`-verbatim
  `deqp_constprod_foldcall_{acos,reflect}_fs`,
  `deqp_constprod_fma_foldcall_{mod,atan}_vs` and
  `deqp_cfold2_const_call_other_bvec` pairs byte for byte, and compiled the
  same way: 327 compile, 64 `Arithmetic`, 16 `ArithConstant`, 16 `ConstantFold`,
  7 `FusedMultiplyAdd` (the measure's `constant_expressions` row agrees).
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_kfold_<name> --instrument shim` on the drm-shim
  that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` (sha256 of `third_party/mesa-26.2.1/UPSTREAM`) in
  `$CONNECTED_DEPS_DIR/mesa-shim`; the README's self-check
  (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was byte-equal. Each shader's
  `.license` names its case. No `.nir.sha` was drawn for them.

## 2. The open steps at the tip

| Step | Site (tip `4d8a0b40`) | What refuses | Cases | dEQP groups | Pinned before | New `deqp_kfold_*` |
|---|---|---|--:|---|---|---|
| **K2** neglit | `glsl.rs:1751` (`neg_products`: both factors `fneg(ConstF32)`) | a product of two negated literals | **109** (56 vs / 53 fs) | random.basic_expression 26, scalar_conversion 21, conditionals 16, all_features 15, texture 7, comparison_ops 6, exponential 5, swizzle 5, trigonometric 5; functions.overloading 2; fragment_ops.interaction.basic_shader 1 | `constprod_neglit_pair_{vs,fs}` | `k2_neglit_{expr_vs,trig_vs,conv_fs,texture_fs}` |
| **K2** chain | `glsl.rs:1823` (`neg_products`: `clamped \|\| chain`) | a negated literal times a product | **40** (15 vs / 25 fs) | random.all_features 12, conditionals 12, basic_expression 7, scalar_conversion 6, comparison_ops, swizzle, texture 1 each | `constprod_chainlit_{vs,fs}` | `k2_chain_{cond_vs,swizzle_fs,conv_combined}` |
| **K4** rest, fs | `nir_fs.rs:4650` (any other front-end `ffma` with a `1.0`/`-1.0` operand) | a built-in on constants in the fragment walk | **16** | constant_expressions.builtin_functions: `acos`, `atan_separate`, `reflect`, `refract` x float/vec2/vec3/vec4, `_fragment` | `constprod_foldcall_{acos,reflect}_fs`, `constop_ce_atan_separate_fs` | `k4_{acos_vec2,atan2_vec4,reflect_vec3,refract_float,refract_vec4}_fs` |
| **K4** rest, vs | `nir.rs:2863` (`vs_fmod`: `neg_floor` is no `fneg`, a constant) | `mod` of constants in the vertex walk | **7** | constant_expressions.builtin_functions.common.mod_{float,vec2,vec3,vec4,vec2_float,vec3_float,vec4_float}_vertex | `constprod_fma_foldcall_mod_vs` | `k4_mod_{vec4,vec3_float}_vs` |
| **K5** | `nir.rs:6754` (the vertex walk's `AluOp::Ffma` catch-all) | `asin`/`acos`/`atan`/`atan2` of a vertex value | **66** | operator.angle_and_trigonometry.{asin,acos,atan,atan2}.{highp,mediump}_{float,vec2,vec3,vec4}_vertex 32; random.trigonometric.vertex 32; random.all_features.vertex 2 | `constprod_fma_trig_{acos,atan2}_vs`, `constprod_fma_random_trig_vs`, `fmaloc_fma_{asin,atan,atan2}` (all highp float or random) | `k5_{asin_mediump,atan_mediump_vec2,acos_highp_vec4,atan2_mediump}_vs`, `k5_random_{trig17,allfeat51}_vs` |
| **fix C** | `nir_fs.rs:11787` (`pair_node`'s guard: a pair asked for again while its `vectorise` is open) | the `lane_tree`/`vectorise` cycle, refused by name | **11** | random.scalar_conversion 6 (`.29`, `.74`, `.96`, fragment and combined), basic_expression 2 (`.58`), comparison_ops 2 (`.13`, `.29`), texture 1 (`.83`) | `defects_lanecycle_{fs,reduced}`, `intrnd_dead_bool_cmp_fs`, `misc2_algebraic_random13` | `c_{conv29,cmp29,conv74,expr58}_fs` |

Every case of the table is Fail on the board (`37305608`). In all, K2 149, K4's
rest 23, K5 66, fix C 11: **249** first refusals.

The same nir.rs:6754 catch-all also holds **K6** (24: matrix.mul 21,
matrix.mul_assign 3) and **K7** (19: operator.geometric 10,
invariance.{highp,mediump,lowp} 9), split by the refusing vertex shader's
source (`asin|acos|atan` is K5; `reflect|refract|distance|fract|faceforward`
K7; the rest, all `mat*` products, K6). Not this note's steps.

Every new reference is refused at its row's site with its row's text (the
scratch probe), and `refusals_pinned` holds the texts. None compiles, so none
is in `DIFFERS`.

### 2.1 What else the sites show (for the planner; no step here)

- `ArithConstant` at **`nir.rs:2628`** (`Nir::alu`'s `is_const_product` guard,
  constprod **K3**) is the largest constant-product site at the tip: **176**
  cases (173 vs; Fail 104, NotSupported 72): shaders.indexing.{tmp_array 32,
  varying_array 32, matrix_subscript 24, vector_subscript 18},
  operator.angle_and_trigonometry 16, random 30, invariance 12,
  default_vertex_attrib 16. K3 has no item (schedule 4b).
- `deqp_cfold2_zero_operand_inot` and `deqp_cfold2_const_other_inot` (the
  references of nonfin **NF1**, which the schedule 5.2/5.5 gives to "constprod
  K2") are refused at **`nir.rs:2628`**, the K3 guard, not at K2's
  `glsl.rs` sites: NF1 is K3's, and K2 alone does not lift it.
- `glsl.rs:1830` (`hoist_operand` declines; 6 cases, functions.datatypes
  mat2/mat3/mat4 vertex and fragment, = `constprod_litfirst_mat_{vs,fs}`, K8),
  `nir_fs.rs:7153` (4, matrix.{add,sub}.const_highp_mat{3,4}_float_fragment),
  `glsl.rs:1633` (2, random.all_features.vertex.17/.94, the `mixed`
  reassociation, `constprod_reassoc_mixed_vs`, K3).
- Line shifts since `mali-constprod-lowering.md` section 8 (`c0142485`), same
  arms: `glsl.rs:1609` is `:1751`, `:1681` is `:1823`, `:1492` is `:1633`;
  `nir.rs:2414` is `:2628`, `:2649` is `:2863`, `:4816` is `:6754`;
  `nir_fs.rs:3803` is `:4650`. The `nir_fs.rs` twin of K2's rule
  (`:1881`, `Fmul` of two `fneg`s in the fragment walk) is reached by no case.
- K4's landed part: `constprod_fma_foldcall_{acos,atan,reflect,refract}_vs`
  and `fmaloc_fma_const_{asin,reflect,matmat}` are byte-equal at the tip (the
  vertex arm `nir.rs:6748`, "of three constants, the fold of a built-in on
  constants", `b5ed279e`); of the builtin_functions cases, all 28
  `acos/asin/atan_separate/atan_combined/reflect/refract/faceforward_*_vertex` compile.

## 3. Mesa's folded bits, per step, and the `mesa_fold` entry point it needs

`tests/mali_deqp_kfold.rs` `folded_bits` computes each constant below with the
helper (included by path) and finds it among the reference's words.

### K2: products of negated literals and literal chains

All seven new references are **global initialisers of literals only**
(`float h = (-8.0 * -1.5)`, `const float f = 16.0 * 0.25 * -0.25`,
`float(61.75) * float(0.5) * -0.25`, `float(5.5) - -5.0 * -1.0`,
`float(1.5) - -0.25 * -2.0`, `float(0) * float(-1) * -0.5`,
`-2.0 * float(float(0.25))`). Mesa's GLSL IR folds them whole
(`ir_constant_expression.cpp`, f32 `a * b` left to right, unfused; by reading),
so no product of them reaches NIR: the value is a FAU word or an immediate of
its reader (`k2_chain_conv_combined`'s fragment program is a constant
colour: `MOV`s of FAU words, no ALU). Which word holds which folded value
in the other six was not traced lane by lane; the byte-compare decides.
The existing `neglit_pair_*` /
`chainlit_*` are the run-time forms (`func(in0) * func(-0.5, -2.0)`,
`x * y * -c`).

- **Entry point: none.** The fold is plain binary32 multiplication in source
  order, which every Rust `f32` `*` gives; the only rounding to add is the
  existing `mesa_fold::f32_to_f16` where a mediump fragment shader reads the
  constant. The step is: in `glsl.rs` `neg_products`, a product whose factors
  are all `ConstF32` (through `fneg`) is folded to one `ConstF32` before the
  `(Some, Some)` refusal (`:1751`) and before the `chain` refusal (`:1823`);
  `fmul(fneg(a), fneg(b))` of non-constants is `fmul(a, b)` (constprod S2);
  `x * y * -c` takes `x * c` first (`negmul_three`).

### K4's rest: built-ins on constants

| Reference(s) | Built-in, constants | Mesa's word | How Mesa folds it | Entry point |
|---|---|---|---|---|
| `k4_reflect_vec3_fs`, `k4_refract_{float,vec4}_fs`, `constprod_foldcall_reflect_fs` | `reflect(..).x`, `refract(.., 0.5).x` | `-1.0`, the immediate `0x3F800000.neg` of `FADD.f32 r1, 0x3F800000.neg, u2.w0.neg` | GLSL IR body, f32 (`I - 2 * dot(N, I) * N`; `eta * I - (eta * dot + sqrt(k)) * N`), exact | none (exact f32; `fold_glsl(Sqrt)` for `refract`'s root) |
| `k4_acos_vec2_fs`, `constprod_foldcall_acos_fs` | `acos(1.0)` | `+0.0`: no word, `FCMP_OR.f32.ge.m1 r0, r0^, u2.w0.abs, 0x0` compares the reference alone | `acosf(1.0)` = 0 | `fold_glsl(Acos)` (by reading, not measured: the front end's expansion gives 0 too, `pi/2 - pi/2` of one constant) |
| `k4_atan2_vec4_fs`, `constop_ce_atan_separate_fs` | `atan(-1.0, -1.0)` | FAU `c016cbe4` (`FADD.f32 r1, u3.w0, r1^.neg`) | GLSL IR's **double** `atan2`, rounded once | **`fold_glsl(Atan2, ..)`** of the whole call: the op-by-op fold of our front end's expansion is `c016cbd6` (`mali-deqp-constop.md` 10.2 R7), so routing alone gives other bits |
| `k4_mod_{vec4,vec3_float}_vs`, `constprod_fma_foldcall_mod_vs` | `mod(87.65, 3.7)` into a mediump varying | `IADD_IMM.i32 r0, 0x0, #0x411A411A` in the fragment shader, no `vs.varying` region | `x - y * floorf(x / y)` unfused in f32, then `_mesa_float_to_float16_rtne` | `f32_to_f16` of the plain f32 expression (fused or unfused give `411a` alike, `mali-libm-fold.md` 3.2) |

So K4's rest is two sub-steps:

- **K4a (19 cases: 12 fragment `acos`/`reflect`/`refract` + 7 vertex `mod`).**
  The route, as constprod section 3 says: in `nir_fs.rs` before the `:4650`
  guard, a lane whose three `ffma` operands are all `Val::Const` is the fold's
  (the fragment twin of `nir.rs:6748`, unfused `a * b + c`); in `nir.rs`
  `vs_fmod`, a constant `neg_floor` makes the whole `mod` a constant (fold it
  through `vs.alu`). No new fold arithmetic. Un-ignores `k4_acos_vec2_fs`,
  `k4_reflect_vec3_fs`, `k4_refract_{float,vec4}_fs`, `k4_mod_*`,
  `constprod_foldcall_{acos,reflect}_fs`, `constprod_fma_foldcall_mod_vs`.
- **K4b (4 cases: `atan_separate_*_fragment`).** The bits need the whole call
  folded by `mesa_fold::fold_glsl(Atan2, y, x)` while the call is still one
  (the front end's expansion in `builtin_call.rs` is shared with the V3D and
  PowerVR back ends, so the fold must not change their words; a Mali-side
  recognition of the expansion's shape on constant operands, or a constant
  call kept whole for the Mali build, are the two ways). If neither is done,
  K4a's route must leave these four refused (a fold of the expansion would be
  "compiled, not Mesa's"): `k4_atan2_vec4_fs` and
  `constop_ce_atan_separate_fs` stay ignored.

### K5: `asin`/`acos`/`atan`/`atan2` of a vertex value

No constant is folded at run time; Mesa's words are NIR's expansions
(`nir_builtin_builder.c`) with their coefficients as FAU words:

- **`asin`, `acos` are fp32 at both precisions** (`nir_asin` promotes):
  `FMA.f32 r, x.abs, c, ..` Horner in `|x|` with `build_asin`'s
  `-0.0187293 0.0742610 -0.2121144 1.5707288` (`bc996e30 3d981627 be593484
  3fc90da4`) and `M_PI_2f` (`3fc90fdb`), `FADD.f32 r, 0x3F800000, x.neg.abs`,
  `FRSQ.f32` then `FRCP.f32` (the root), `FCMP_OR.f32.lt.f1` x2 and `FADD` for
  the sign; `acos` ends `FMA.f32 r, s.neg, p, u.w1` (`pi/2 - ..`).
  `k5_asin_mediump_vs` and `k5_acos_highp_vec4_vs` (per lane, 35 and 62
  words).
- **mediump `atan`/`atan2` are fp16** (`k5_atan_mediump_vec2_vs` 49 words,
  `k5_atan2_mediump_vs` 58): the lanes narrowed by `FADD.f32 r.h0, x, 0x0.neg`,
  `FRCP.f16`, `FCMP_OR.v2f16.ge.m1`, `MUX.v2i16` (the `|x| > 1` swap), the
  `nir_atan` polynomial as `FMA.v2f16` with fp16 pairs `a236 2adf af83 3234 b553`
  and `3c00` (the f32 coefficients `bc46c6a5 ..`, rounded by `f32_to_f16`),
  `be48` (-pi/2), `80008000`/`7fff7fff` copysign masks, and for `atan2` the
  constants `74007400` (huge, 16384) and `3e48` (pi/2).
- **highp `atan`** is the same chain in fp32 (`k5_random_trig17_vs`:
  `FRCP.f32`, `FCMP_OR.f32.ge.m1 0x3F800000 >= |x|`, `CSEL.u32.eq`,
  `IADD_IMM #0xBFC90FDB`, five `FMA.f32` of `bc46c6a5 3d5be101 bdf0555d
  3e468bc1 beaa5476` and `3f7ffea5`, the `80000000` sign).
- `k5_random_allfeat51_vs` mixes the landed `sin`/`cos` (`FSIN_TABLE`,
  `FCOS_TABLE`, arith step 2) with an `acos` chain: K5 is the last refusal of
  its vertex shader.

**Entry point:** none for the run-time chains. The coefficient words are the
double literals of `nir_builtin_builder.c` as `f32` (a Rust `as f32`), and as
fp16 `mesa_fold::f32_to_f16` of that `f32` (`folded_bits` checks both). A lane
of such a chain whose operand is a constant (a `vec4` with one constant lane)
folds through `mesa_fold::fma32` in NIR's op order (`fold_nir(Atan, 1.0)` is
`3f490fa4`, `mali-libm-fold.md` 1.4), never through `alu_sized`'s unfused
rule. The step: the vertex walk accepts the front end's `ffma` terms of
`asin`/`acos`/`atan`/`atan2` (`builtin_call.rs`, as `fma_trig_*` and arith
step 1b's fragment `atan` take them) before the `:6754` catch-all, fp32 for
`asin`/`acos` at every precision, fp16 for a mediump `atan`/`atan2`.

### Fix C: the `lane_tree`/`vectorise` cycle

No fold. Mesa's words for the four new shapes keep **one computed lane** and
make the others constants: `c_conv29_fs` and `c_cmp29_fs` (`vec4(-1, g, 0,
-0.25) * vec4(f, c, e, d)` with `f`, `c` constant false) are 9 words, lanes
x, y, z the constant `+0.0` (`MOV.i32 r0, 0x0`, `MKVEC.v2i16 r1, 0x0.h0, ..`;
lane y is `g * 0.0` of a uniform, folded to `+0.0`) and lane w one
`FMA.v2f16 r0, 0x34000000.neg.h11, d.h00, 0x0.neg` (`-0.25 * d`).
`c_expr58_fs` and `c_conv74_fs` (13 and 17 words) pair a constant half with
a computed one through `MKVEC.v2i16 r, u.w0.h0, x.h0` and one `FMA.v2f16`.
`defects_lanecycle_reduced` instead keeps `2b * 0.0` as a product
(`mali-deqp-defects.md` 3.3): which rule makes Mesa fold `g * 0.0` here and
keep `2b * 0.0` there was not found (the NIR dump of the four pairs, which
`tools/mali/nir_refs.sh` would give, was not drawn). **Entry point: none.**
The step: build a lane of a pair whose other lane is a constant alone,
without going back through `Fs::pair_lane` (`mali-deqp-defects.md` fix C),
and keep `pair_node`'s guard as the backstop; the byte-compare of the eight
references decides which constant lanes the walk may fold.

## 4. Forecast (HOST-ONLY)

| Step | First refusals at the tip | Whose shader then compiles (forecast) | Groups that lift |
|---|--:|---|---|
| K2 | 149 | not forecast: 146 are random shaders, which meet their next text (`k2_neglit_trig_vs` also has an `acos` of a vertex value, K5's); the 2 functions.overloading cases compile if `neglit_pair_*` does (their partner is the isOk harness) | shaders.random.*, functions.overloading.user_func_arguments_* |
| K4a | 19 | **19** (12 fragment cases whose vertex half is dEQP's pass-through; 7 vertex cases whose fragment half is the isOk harness of `fma_foldcall_*_vs`, byte-equal today) | constant_expressions.builtin_functions.{trigonometry.acos, geometric.reflect, geometric.refract}_*_fragment, common.mod_*_vertex |
| K4b | 4 | 4 if `fold_glsl(Atan2)` is reached; else 0 (they stay refused) | constant_expressions.builtin_functions.trigonometry.atan_separate_*_fragment |
| K5 | 66 | **32** (operator.angle_and_trigonometry: the fragment half is dEQP's varying-to-colour shader, 7 words); the 34 random ones meet their next text | operator.angle_and_trigonometry.{asin,acos,atan,atan2}.*_vertex |
| fix C | 11 | not forecast (by the byte-compare of the four new references and `defects_lanecycle_*`); a named refusal at worst | shaders.random.{scalar_conversion,basic_expression,comparison_ops,texture} |

`ArithConstant` falls from 353 to at most 353 - 149 - 16 = **188** after K2
and K4 (still above the 100-case line: K3's 176 at `nir.rs:2628` are the
rest), `FusedMultiplyAdd` from 116 to at most 116 - 7 - 66 = **43** after K4
and K5 (K6 24, K7 19), `Defect(LaneCycle)` to 0 or a named text after fix C.
Upper bounds, no inflow: a case that leaves its text may meet the next one.
The plan's criterion needs K3 as well for `ArithConstant`.

## 5. Order and overlaps

K4a first (two arms, no new arithmetic, 19 cases that compile), then K5 (66;
serial with K6/K7 on the same catch-all and with arith step 7 on
`builtin_call.rs`, constprod section 5), then K2 (149, `glsl.rs`
`neg_products`, which every negated product of glmark2's `ok` shaders passes:
the `negmul_*` and `gm2_fsmisc_6` pins and the corpus verdict test guard it),
then fix C, then K4b. All of them edit `src/shaderc/online/mali/**`; they are
`mali-train-4`'s, after train 3. Lines are the tip's: `lower-train-2`
(plan #367) and `lower-fsint` (plan #402) have not landed and move
`nir_fs.rs` and `nir.rs` lines; the arms are named above so that the
step finds them again.

## 6. What this note does not know

- The next refusal of each random case after its step (no compiler change was
  made here).
- Whether Mesa's G310 words equal these G610 references: not rechecked
  (`tools/mali/g310_check.sh`); `mali-g310-vs-g610.md` found every earlier
  reference equal.
- The NIR of the new pairs (no `.nir.sha`) and the zero-product rule of
  fix C's shapes (section 3).
