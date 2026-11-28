<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the `ConstantFold`, `NegZeroShared`, `AlgebraicForm` and `Rebalance` residue, measured, with Mesa 26.2.1 references

**Status: design** (plan #387, item `constres-refs`; issues #20, #7). Nothing
under `src/` changed. Measured at the integration tip
**`349f989f5392bea65c20e7663289e86481e521e0`** (2026-11-28; it contains
train 3a). The new references are `tools/mali/testdata/deqp_constres_*`,
pinned by `tests/mali_deqp_constres.rs`. Mesa paths are relative to `src/` of
the 26.2.1 tree.

Section 19.1 of `docs/notes/mali-deqp-families.md` (at `e9033663`) counts the
four texts as the first refusal of 102 must-pass cases: `ConstantFold` 34,
`NegZeroShared` 30, `AlgebraicForm` 28 and `Rebalance` 10. At this tip they
are **108**. The design steps were CF-B/C/D (`mali-deqp-cfold2.md` 4), N3
(`mali-deqp-negzero.md` 4), K8 (`mali-constprod-lowering.md` S8) and A8
(`mali-arith-lowering.md` step 8), and none had an item. This note puts each of
the 108 cases in one step row (section 3) and gives an order (section 4).

## 1. How this was measured

- **Host measure:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" --jobs 3 --chunk
  100`. This is section 19's command. VK-GL-CTS is at `93bca018`, fetched at
  that commit alone, with `external/fetch_sources.py` run and
  `tools/cts/cts-yakogl-target.patch` applied by `git apply`. All 10,597 cases
  of the slices `2026-11-15-t6-gles2-S{1..4}-37305608` began. 106 processes
  exited 1 and none crashed. 4,709 cases carry a host refusal (4,716 at
  `e9033663`). HOST-ONLY: these are compile and link outcomes only.
- **Workaround, not part of this change:** at `349f989f`, `capi/Cargo.toml`
  defines the feature `vivante-st` twice (lines 41 and 85; `110080eb` and an
  earlier merge each added it), so cargo cannot load the workspace. The
  measure and the tests ran with the second definition removed in the
  working tree, as several unlanded branches already do
  (`connected/01a21f4f-...`). Nothing of that is committed here.
- **The cases:** every case whose `host_refusal` is one of the four texts.
  Each has exactly one program in `programs.tsv`, and all 108 are Fail on the
  board (`37305608`). By stage: `ConstantFold` fs 40; `NegZeroShared` vs 18 /
  fs 12; `AlgebraicForm` vs 1 / fs 27; `Rebalance` vs 8 / fs 2.
- **Refusal sites:** a scratch probe outside the tree, in a detached worktree
  that was thrown away. It wrapped every construction of the four errors in
  `glsl.rs`, `nir.rs` and `nir_fs.rs` so that it records `file!():line!()`
  (line numbers unchanged), and compiled each case's program through
  `tests/mali_ref`'s `compile_pair`. Every case's text matched the measure's.
  For `NegZeroShared` the probe also logged the operation at which the -0.0
  fold (`Tree::fold_value`) first makes a "may be -0.0" lane from operands
  that are not -0.0, and where it gives up (`depth > 12`, an op it does not
  know, a `libm_lane` result that is not finite or not representable in
  fp16).
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_constres_<name> --instrument shim`. The shim is the
  drm-shim that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` (sha256 `c47e81bd...`, as in
  `third_party/mesa-26.2.1/UPSTREAM`) into `$CONNECTED_DEPS_DIR/mesa-shim`.
  The README's self-check (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was
  byte-equal. Each pair is the case's program, verbatim from the measure's
  `.qpa`. Its `.vert.license` and `.frag.license` name the case.
- **Sibling results:** the unlanded result commits named by the plan were
  merged on the tip in a scratch worktree, one at a time. `8180095c`,
  `2c0c6596` (lower-vsconst) and `81b3ce19` (lower-mixprec3) merge cleanly.
  `13259474`, `cbb6b45b` and `e8327e42` conflict with the tip itself and were
  left out. Section 5 lists what the three change. Caveat for anyone
  repeating this: the worktrees share `CARGO_TARGET_DIR`
  (`~/.cache/yakogl/ci-target`), and cargo then reuses one tree's test
  binary for another, because a path package's metadata hash leaves out the
  path. Build each tree in its own target directory.

## 2. The four texts at the tip

| Text (`MaliError`) | Section 19.1 (`e9033663`) | Tip | Sites (cases) |
|---|--:|--:|---|
| `ConstantFold` | 34 | **40** | `nir.rs:2424` 16, `nir_fs.rs:7761` 18, `nir_fs.rs:8027` 6 |
| `NegZeroShared` | 30 | **30** | `glsl.rs:2342` 30 (`Tree::neg_zero_compares`) |
| `AlgebraicForm` | 28 | **28** | `nir_fs.rs:1978` 25, `nir_fs.rs:1963` 2, `nir.rs:2481` 1 |
| `Rebalance` | 10 | **10** | `glsl.rs:1399` 6, `glsl.rs:1503` 2, `glsl.rs:1379` 2 |
| **Total** | 102 | **108** | |

`ConstantFold` gained the 6 `shaders.invariance.*.common_subexpression_{0,3}`
cases. They met `FsUniform` (`uniform mediump vec4 u_color`) at `e9033663`,
which train 3a lowered. The other 102 are the same cases as section 19.1's.

The partner shader of 106 of the 108 compiles at the tip (the measure's
`next_family` is empty). For `conditionals.combined.46` and `.54` it is
refused with the `gl_FragColor` lane text.

## 3. Per step: every case, its mechanism and references

"Refs" are the references that pin the step's shape. The older ones are
named in full. The new ones are `deqp_constres_<name>`, abbreviated as
`constres_<name>`. Each reference listed is refused at the tip at its row's
site. Case names drop `dEQP-GLES2.functional.`.

| Step | Text, site | Cases | Mechanism (what Mesa does) | dEQP cases | Refs |
|---|---|--:|---|---|---|
| **CF-B** | `ConstantFold`, `nir.rs:2424` (the constant arm's `Fsin..Fpow` refusal) | **16** | `log2`, `log`, `pow` and `atan(y_over_x)` of constants. The GLSL front end folds them with libm at fp32, or for `atan` at double (`ir_expression_operation.py`). Fold the call in `glsl.rs`, before its expansion, through `mesa_fold::fold_glsl` (`Libm::Log2`, `Log`, `Pow`, `Atan`). The pinned bits are `0x40d49a78`, `0x406f64b9`, `0x40ccfc0f` and `0x3f8db70d` (cfold2 3). At this tip `atan_combined` refuses at this site too, not at the `iand` that cfold2 recorded. | `constant_expressions.builtin_functions.exponential.{log2,log,pow}_{float,vec2,vec3,vec4}_fragment` (12), `.trigonometry.atan_combined_{float,vec2,vec3,vec4}_fragment` (4) | `deqp_cfold2_const_call_transcendental_{log2,pow,atan}` (existing) |
| **CF-D** | `ConstantFold`, `nir_fs.rs:7761` (mediump `rcp` of a constant not exact in fp16) | **18** | Mesa lowers `fdiv` to `fmul(a, frcp(b))` and folds both at fp16, rounding each to nearest even (`const_other_rcp16`'s `0x37fe`). Round `1/c` with `mesa_fold::f32_to_f16` instead of refusing. In the random shaders the look-up or value that reads it is often dead, and Mesa's program is a constant colour. The three `matrix.div.const_highp_*` cases, whose constants are declared highp, refuse at the same fp16 site. lower-vsconst `2c0c6596` (step D) makes `constres_cfd_div_highp_mat4` byte-equal and compiles all three: 15 are left after it. | `matrix.div.const_{highp,mediump,lowp}_mat{2_mat2,3_mat3,4_mat4}_fragment` (9); `random.trigonometric.fragment.{3,10,41,47,49,54,56,85}` (8); `random.all_features.fragment.40` (1) | `deqp_cfold2_const_other_rcp16`, `_zero_operand_rcp16` (existing); `constres_cfd_div_highp_mat4`, `_cfd_trig_fragment_3`, `_cfd_allfeat_fragment_40` |
| **CR-1** (new) | `ConstantFold`, `nir_fs.rs:8027` (`fdot` of constant lanes, "the front end's fold that did not fold") | **6** | `dot(v_unrelated, vec4(1.0))`, where the link has made `v_unrelated` a constant (the invariance VS writes constants). Mesa propagates the varying and folds the dot: the FS is 7 words, the colour from `u_color`'s FAU words and the folded lane. Fold the fragment walk's `fdot2/3/4` of constant lanes at fp32 and narrow the result with `mesa_fold::f32_to_f16`. nir_opcodes' `fdot` evaluates in float and converts the result once. Refuse only a result outside fp16's range. | `invariance.{highp,mediump,lowp}.common_subexpression_{0,3}` | `constres_cdot_cse0_highp`, `_cdot_cse3_mediump` |
| **N3** | `NegZeroShared`, `glsl.rs:2342` | **19** | The compare reads a lane that the -0.0 fold leaves as "a constant of unknown sign". The lane comes from an `Fmul` (or `Fadd`, `Ffma`) of a negative or zero constant and a value built from int and bool constants that the fold does not evaluate: `0 * float(m)` with `int m = int(float(1.0) * float(1))`, `-0.0 * float(u)` with `const int u = int(-2.0)`, ivec compares, `int(bool)`. Mesa's front end and NIR fold these to plain constants, so no -0.0 reaches the compare. In `Tree::fold_value`, evaluate the int and bool operations (`I2f`/`B2f` of a computed int or bool, `F2i`, int `+ - *`, the compares, `inot`) through the existing `nir::fold_int`. That is cfold2 step A's route, so no second integer folder is written. Also propagate a global's single constant store. | `random.conditionals.vertex.{17,33,56,66,94}`, `.fragment.{17,56}`, `.combined.{10,17,27,29,46,48,54,98}`; `random.all_features.vertex.{53,71,87}`, `.fragment.37` | `constres_n3_cond_vertex_17`, `_n3_cond_fragment_17`, `_n3_cond_combined_10`, `_n3_allfeat_vertex_87` (`deqp_negzero_mainplain_*` compile, or refuse with arithmetic, at the tip) |
| **CR-2** (new; N2's rest) | `NegZeroShared`, `glsl.rs:2342` | **9** | The probe finds `Frcp` and `Fsign` origins. These come from the GLSL expansions of `atan`, `asin`, `acos` and `tan` of constants, which the fold sees op by op: `libm_of` maps only `sqrt`, `inversesqrt`, `exp2`, `log2`, `sin` and `cos`. In `fragment.32`, `.62` and `.78`, `libm_lane` also gives up on a `sin` or `log2` result that is not representable in mediump fp16. Mesa folds the whole call in the front end. Give the -0.0 fold the call's value from `mesa_fold::fold_glsl` (`Tan`, `Asin`, `Acos`, `Atan`, `Exp`, `Log`, `Pow`), which is the same entry as CF-B. | `random.all_features.fragment.{5,32,62,78,87,90}`, `.vertex.{18,40,73}` | `constres_nzb_allfeat_fragment_5`, `_nzb_allfeat_vertex_40` |
| **CR-3** (new) | `NegZeroShared`, `glsl.rs:2342` | **2** | The constant chains of the random globals are deeper than `fold_value`'s cap (`depth > 12`, "unknown"). Memoize per value (`folds` already caches whole reaches) and raise or drop the cap. `fragment.78` hits the cap too, but is counted under CR-2. | `random.all_features.fragment.{71,85}` | `constres_nzd_allfeat_fragment_71` |
| **CR-4** (new; K8's rule on computed factors) | `AlgebraicForm`, `nir_fs.rs:1978` (`shared_factor`) | **25** | `fadd(fmul(a, b), fmul(a, c))` becomes `fmul(a, fadd(b, c))` when `a` is a computed factor, not a constant (`nir_opt_algebraic.py:246`, `~fadd`). The matrix-times-float harness sums `res[0][1] + res[1][1]`. Mesa adds the matrix lanes first and multiplies once: `FADD.v2f16 r0, r0^.h11, r1^.h11`, then `FMA.v2f16 r0, r2^.h00, r0^.h00, 0x0.neg` (`constres_sf_mul_dynamic_mediump_mat2`). K8 covers the same rule with constant addends, and `deqp_constprod_alg_constmat_fs` is byte-equal at the tip. This is the factoring the code's own comment calls "not lowered" (`deqp_irindex_mat3_uniform_conv`). | `matrix.mul.{dynamic,uniform}_{lowp,mediump}_mat{2,3,4}_float_fragment` (12), `matrix.mul.uniform_highp_mat3_float_fragment` (1), `matrix.div.{dynamic,uniform}_{lowp,mediump}_mat{2,3,4}_float_fragment` (12) | `constres_sf_mul_dynamic_mediump_mat2`, `_sf_mul_uniform_lowp_mat4`, `_sf_div_dynamic_mediump_mat3`, `_sf_mul_uniform_highp_mat3` |
| **CR-5** (new) | `AlgebraicForm`, `nir_fs.rs:1963` (`neg_sum_const` whose sum folds to 0.0) | **2** | The reassociation of `#a + -(b + #c)` (`nir_fs.rs:1951-1963`) reaches constants whose fp16 sum is 0.0, a form the code refuses as "has no reference". The random shader's colour is a chain of constant products and sums over the varying lanes. Mesa's FS is 17 words (`FADD_IMM.v2f16`, `FMA.v2f16`, FAU `43004300 30003000 c340c340 b800bd00 45004500`), and the reference pins that. With lower-vsconst and lower-mixprec3 merged, the reference meets `Defect(LaneCycle)` instead (kfold fix C, train 4), so CR-5 goes after fix C. | `random.basic_expression.{fragment,combined}.4` | `constres_nsz_basic_expression_fragment_4` |
| **CR-6** (new) | `AlgebraicForm`, `nir.rs:2481` | **1** | `iadd(bcsel(c, #x, #y), #d)` becomes `bcsel(c, x + d, y + d)` (`nir_opt_algebraic.py:3430-3432`). The `i2f` then reads two float constants: `CSEL.u32.eq` selects FAU `u6.w1`/`u7.w0` (-3.0, -4.0, the negated `c`), and no `S32_TO_F32` follows. This is the "AlgebraicForm" case of cfold2 4.1. Fold `x + d` and `y + d` with `nir::fold_int`. | `random.conditionals.vertex.8` | `deqp_constrnd_int_bits_vs` (existing); `constres_bcd_cond_vertex_8` |
| **A8** | `Rebalance`, `glsl.rs:1399` (6), `:1503` (2), `:1379` (2) | **10** | GLSL's `do_rebalance_tree` (`compiler/glsl/opt_rebalance_tree.cpp`) reassociates chains of 3 or more `+` or `*`. It skips invariant assignments (`rebalance_invariant`). Sites: `visit`'s `Reduction::Unknown` chain at `:1399` (one use); the span's sequence check (`nord != nids`) at `:1503`; a chain of several uses at `:1379` (`Use::Many`). | `functions.misc.multi_arg_float_{fragment,vertex}`, `invariance.{highp,mediump,lowp}.common_subexpression_1`, `random.all_features.vertex.{14,35,54,84}`, `.fragment.24` | `deqp_misc2_rebalance_{funcsum_fs,invariant,random_fs,random_vs}`, `deqp_arith_other_x_vs_mediump` (existing); `constres_rb_allfeat_vertex_14`, `_rb_allfeat_vertex_54`, `_rb_invariance_lowp_cse1` |
| | | **108** | | | |

Not assigned here: **CF-C** (an infinity the next operation absorbs) has no
case at this tip. `deqp_cfold2_zero_operand_rcp_inf` compiles, and the
`rcp_inf`/`rsq_inf` random references stop at the FMA and arithmetic texts.
**NF3** (train 3b) and the **kfold K steps** (train 4) take none of the 108 as
a first refusal. Their sites (`nir.rs` undef NaN; `glsl.rs:1751`/`:1823`,
`nir_fs.rs:4650`, `nir.rs:2863`/`:6754`) are not among section 2's.
**K8** proper (`-2.0 * a`, `litfirst_mat_*`) is refused with the constant-product
text, not one of these four.

All folds use the one constant folder, as `mali-libm-fold.md` and the
schedule's row on built-in folds require. Built-ins (CF-B, CR-2) go through
`mesa_fold::fold_glsl`. fp16 narrowing (CF-D, CR-1) goes through
`mesa_fold::f32_to_f16`. Integer and boolean folds (N3, CR-6) go through
`nir::fold_int`, the existing integer route of cfold2 step A; `mesa_fold.rs`
has no integer fold. No step writes a new folder.

## 4. Order

The steps are ranked by the cases each frees, then by cost. Within the four
texts each case is counted once. A freed case passes only once its later
refusals are gone too. The forecast is HOST-ONLY and an upper bound: the
partner compiles for 106 of 108, but what the refusing shader meets next is
not measured, except for section 5's.

| # | Step | Cases | Files | Needs | First un-ignore |
|--:|---|--:|---|---|---|
| 1 | CR-4 | 25 | `nir_fs.rs` (the `Fadd` arm) | none | `constres_sf_mul_dynamic_mediump_mat2` |
| 2 | N3 | 19 | `glsl.rs` (`Tree::fold_value`) | `nir::fold_int` reachable from `glsl.rs` | `constres_n3_cond_vertex_17` |
| 3 | CF-D | 18 (15 after lower-vsconst) | `nir_fs.rs` (`:7761`) | lower-vsconst first (it takes the 3 highp) | `deqp_cfold2_const_other_rcp16` (`0x37fe`) |
| 4 | CF-B + CR-2 | 16 + 9 | `glsl.rs` (call fold before expansion; `libm_of`) | one session: the same `fold_glsl` entry | `deqp_cfold2_const_call_transcendental_log2`, then `constres_nzb_allfeat_vertex_40` |
| 5 | A8 | 10 | `glsl.rs` rebalance | arith step 8's own reference | `deqp_misc2_rebalance_funcsum_fs` |
| 6 | CR-1 | 6 | `nir_fs.rs` (`:8027`) | none | `constres_cdot_cse0_highp` |
| 7 | CR-3 | 2 | `glsl.rs` | after N3 and CR-2 (their lanes then settle) | `constres_nzd_allfeat_fragment_71` |
| 8 | CR-5 | 2 | `nir_fs.rs` (`:1963`) | kfold fix C (train 4) | `constres_nsz_basic_expression_fragment_4` |
| 9 | CR-6 | 1 | `nir.rs` (`:2481`) | none | `deqp_constrnd_int_bits_vs` |

None of the four texts is at the 100 line, and all four together are 108.
Steps 1 to 4 (CR-4, N3, CF-D, CF-B + CR-2) clear 87 of the 108 (84 if
lower-vsconst has taken CF-D's highp three). All of them are compiler-train
work under `src/shaderc/online/mali/**`, so they belong in a train after 3b,
3c and 4, as one commit per step.

## 5. With the unlanded results merged

On the tip with `8180095c`, `2c0c6596` and `81b3ce19` merged, the 21 new
references and the 108 case programs give:

- `matrix.div.const_highp_mat{2_mat2,3_mat3,4_mat4}_fragment` compile, and
  `constres_cfd_div_highp_mat4` is byte-equal. That is lower-vsconst's step D
  ("fold a mediump shader's highp constant trees at fp32").
- `random.trigonometric.fragment.{3,10}` (and `constres_cfd_trig_fragment_3`)
  move to the fragment arithmetic text.
- `random.basic_expression.{fragment,combined}.4` (and
  `constres_nsz_basic_expression_fragment_4`) move to `Defect(LaneCycle)`.
- Nothing else moves.

`tests/mali_deqp_constres.rs` takes these as each row's alternatives: a text,
or `EQUAL` (a byte-equal compile). Its `refusals_pinned` therefore passes at
the tip and with them merged (both were run). It fails when a reference
moves anywhere else.

## 6. The test

`tests/mali_deqp_constres.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_constres`.

- 21 reference tests, all `#[ignore]` (refused at the tip).
  `references_parse` checks the files, the `.license` pair, the trace name and
  `FS_WORDS`. `refusals_pinned` holds the whole text and the alternatives.
- At `349f989f`: 2 passed, 21 ignored. With `--ignored`, each of the 21
  fails only with "the pair is refused" and its pinned text.
- The step that lifts a reference moves it to `LOWERED` or `DIFFERS` in the
  same commit and updates section 3.

## 7. Issue comment text (for #20; agents do not post it)

> constres (`ConstantFold`, `NegZeroShared`, `AlgebraicForm`, `Rebalance`):
> the host measure at `349f989f` finds 108 cases with one of these texts
> first (40 / 30 / 28 / 10; all Fail on the board). A site probe puts each at
> one site. The steps are CR-4 factoring of a shared computed factor 25
> (`matrix.mul`/`div` float fragment), N3 int/bool constants in the -0.0 fold
> 19, CF-D mediump `rcp` of a constant 18 (3 taken by lower-vsconst), CF-B
> built-ins on constants 16, A8 rebalance 10, CR-2 `atan`/`asin`/`tan` of
> constants in the -0.0 fold 9, CR-1 a constant fragment dot 6, CR-3 the
> fold's depth cap 2, CR-5 a zero constant sum 2 and CR-6 the select
> distribution 1. 21 new Mesa 26.2.1 references are in
> `tools/mali/testdata/deqp_constres_*`, pinned by
> `tests/mali_deqp_constres.rs`. Details: `docs/notes/mali-constres-lowering.md`.
