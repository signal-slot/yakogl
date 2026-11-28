<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the `FusedMultiplyAdd` residue at the integration tip, measured, with Mesa 26.2.1 references

**Status: design** (plan #387, item `fmares-refs`; issue #7). Nothing under
`src/` changed. Measured at the integration tip **`349f989f5392bea65c20e7663289e86481e521e0`**
(2026-11-28). The new references are `tools/mali/testdata/deqp_fmares_*`
(14), pinned by `tests/mali_deqp_fmares.rs`. HOST-ONLY: every count is a
compile or link outcome on this host, never a board result.

`mali-deqp-families.md` section 19.1 ranks `FusedMultiplyAdd` 10th (116
must-pass cases as first refusal at `e9033663`, all vertex). The schedule
(`mali-lowering-schedule.md` 2.7) splits them over constprod K4, K5, K6 and
K7; `mali-kfold-lowering.md` (item `kfold-refs`, at `4d8a0b40`) designed K4's
rest and K5 and named K6 (24) and K7 (19) without a design. This note measures
every case again at the tip, puts each in a step, and draws the references the
K6 and K7 shapes still lacked. Read `mali-constprod-lowering.md` (S4-S7),
`mali-fmaloc-lowering.md` (3.4-3.6, 5a) and `mali-kfold-lowering.md` first;
this note does not repeat their tables.

## 1. How this was measured

- **Host measure:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --out "$PWD/build/measure" --jobs 3
  --chunk 100`, the default case list (the 10,597 board-refused cases of the
  slices `2026-11-15-t6-gles2-S{1..4}-37305608`). The CTS tree is the
  `HOST_DEQP_CTS` entry of `tools/cts/deps.toml` (VK-GL-CTS 93bca01, one
  shallow commit from GitHub, `external/fetch_sources.py`,
  `cts-yakogl-target.patch`), made the way its `build` says. All 10,597 cases
  began; 106 chunk processes exited 1 (dEQP's "not every case passed"), none
  crashed. **4,709** cases carry a host refusal (as at `4d8a0b40`);
  **116** have `FusedMultiplyAdd` as their first refusal, all at the vertex
  stage, all Fail on the board (`37305608`).
- **The tip does not build as committed.** `capi/Cargo.toml` at `349f989f`
  declares the feature `vivante-st` twice (lines 41 and 85; two merges each
  added it), and cargo refuses the workspace manifest: every `cargo` command
  fails, the measure's library build included. The measure and the tests ran
  with the one-line fix `6579a1d0` ("capi: vivante-st is declared once",
  on a connected branch, not yet in the tip) applied to the working tree and
  **not committed** here (the measure's `commit` file says `349f989f-dirty`
  for that reason). It touches no compiler code.
- **Sites:** a scratch copy of the tree (in `$TMPDIR`, not committed) in which
  the two `Err(MaliError::FusedMultiplyAdd)` of `src/shaderc/online/mali/nir.rs`
  print which one returned: `vs_fmod` (`nir.rs:2863`, `neg_floor` is no
  `fneg`) and the vertex walk's `AluOp::Ffma` catch-all (`nir.rs:6776`), the
  catch-all also printing the IR op behind each of the front end's three
  `ffma` sources and the walked lanes. There are no other raising sites at the
  tip. Each case's program (the measure's `programs.tsv`, verbatim from the
  `.qpa`) went through `tests/mali_ref`'s `compile_pair`; every one of the 116
  printed one site and was refused with the case's text.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_fmares_<name> --instrument shim` on the drm-shim
  `tools/mali/drm_shim_ref.sh build` made from the pinned `mesa-26.2.1.tar.xz`
  (sha256 of `third_party/mesa-26.2.1/UPSTREAM`) in
  `$CONNECTED_DEPS_DIR/mesa-shim`; the README's self-check
  (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) was byte-equal. The `.nir.sha`
  files are `tools/mali/nir_refs.sh 'deqp_fmares_*'` (14 written, 0 errors).
  Each pair is the `.qpa` text of its case; each `.license` names it.

## 2. The cases by refusing site and shape

`ffma` sources are the IR ops the catch-all printed for the front end's
`ffma(a, b, c)` (`builtin_call.rs` and the matrix product of `glsl.rs`).

| Site (tip `349f989f`) | `ffma` sources | Shape | Cases | dEQP cases |
|---|---|---|--:|---|
| `nir.rs:2863` `vs_fmod` | - | `mod` of constants | 7 | constant_expressions.builtin_functions.common.mod_{float,vec2,vec3,vec4,vec2_float,vec3_float,vec4_float}_vertex |
| `nir.rs:6776` catch-all | `Fabs`, `ConstF32`, `ConstF32` | `asin`/`acos` Horner term in `\|x\|` | 27 | operator.angle_and_trigonometry.{asin,acos}.{highp,mediump}_{float,vec2,vec3,vec4}_vertex 16; random.trigonometric.vertex.{5,26,28,40,51,61,64,74,77,93} 10; random.all_features.vertex.51 |
| catch-all | `ConstF32`, `Fmul`, `ConstF32` | `atan` polynomial term in `x * x` | 39 | operator.angle_and_trigonometry.{atan,atan2}.{highp,mediump}_{float,vec2,vec3,vec4}_vertex 16; random.trigonometric.vertex.{0,4,10,17,18,22,23,31,39,42,43,45,52,54,56,63,65,76,80,94,98,99} 22; random.all_features.vertex.95 |
| catch-all | `LoadLocal`, `LoadInput`, `Fmul` | `const matN * attribute vecN` (the column a `const` local) | 9 | matrix.mul.const_{lowp,mediump,highp}_mat{2,3,4}_vec{2,3,4}_vertex |
| catch-all | `LoadInput`, `LoadInput`, `Fmul` | `attribute matN * attribute vecN` | 9 | matrix.mul.dynamic_{lowp,mediump,highp}_mat{2,3,4}_vec{2,3,4}_vertex |
| catch-all | `LoadInput`, `LoadUniform`, `Fmul` | `attribute matN * uniform matN` | 3 | matrix.mul.dynamic_highp_mat{2,3,4}_mat{2,3,4}_vertex |
| catch-all | `LoadLocal`, `LoadUniform`, `Fmul` | `res = a_matN; res *= u_in1` | 3 | matrix.mul_assign.dynamic_highp_mat{2,3,4}_vertex |
| catch-all | `Fneg`, `LoadLocal`, `Fmul` | `refract`'s `ffma(-(eta * dot + sqrt k), N, eta * I)` | 17 | operator.geometric.refract.{highp,mediump}_{float,vec2,vec3,vec4}_float_vertex 8; invariance.{highp,mediump,lowp}.subexpression_precision_{highp,mediump,lowp} 9 (`refract`, `distance`, `fract`) |
| catch-all | `Fmul`, `LoadLocal`, `LoadLocal` | scalar `reflect`'s `ffma(fmul(fmul(n, i), -2.0), N, I)` (no `fdot`) | 2 | operator.geometric.reflect.{highp,mediump}_float_vertex |
| | | **Total** | **116** | shaders.operator 42, shaders.random 34, shaders.matrix 24, shaders.invariance 9, shaders.constant_expressions 7 |

The counts per step are those of `mali-kfold-lowering.md` section 2 at
`4d8a0b40` (K4 7, K5 66, K6 24, K7 19): nothing moved between the two tips.
The lowp `attribute mat * uniform mat` shapes are not here: the lowp
`matmul_mat` reference (`deqp_constprod_fma_matmul_mat_vs`, lowp mat3) is
byte-equal at the tip, and the measure has no lowp/mediump `dynamic_*_mat*_mat*`
case with this text.

## 3. Steps

Every case is in a step of the schedule; **no new step (FR1, ...) is needed**:
the two sites and nine source shapes all belong to K4, K5, K6 or K7. K6 and
K7 are split here by the operand that refuses, as sub-steps of one item each.
The fmaloc steps of `mali-fmaloc-lowering.md` section 4 are the same work
(5a, "Which FMA rows belong to constprod"): `fma-matrix` = K6, `fma-const` =
K4 (landed but for `mod`), `fma-trig` = K5, `fma-reflect-float` = K7b.

| Step | Mechanism (one line) | Site | Cases | dEQP groups | References (existing; **new**) |
|---|---|---|--:|---|---|
| **K4** (K4a vs) | in `vs_fmod`, a constant `neg_floor` makes the whole `mod` a constant: fold it through `vs.alu` (unfused f32, then `f32_to_f16` at the mediump reader) | `nir.rs:2863` | 7 | constant_expressions.builtin_functions.common.mod_*_vertex | `kfold_k4_mod_{vec4,vec3_float}_vs`, `constprod_fma_foldcall_mod_vs` |
| **K5** | accept the front end's `asin`/`acos`/`atan`/`atan2` polynomial `ffma`s before the catch-all: fp32 for `asin`/`acos` at every precision, fp16 (`FMA.v2f16`) for mediump `atan`/`atan2`; the selects and sign words with them | `nir.rs:6776` | 66 | operator.angle_and_trigonometry 32; random.trigonometric.vertex 32; random.all_features.vertex 2 | `kfold_k5_*` (6), `constprod_fma_trig_{acos,atan2}_vs`, `constprod_fma_random_trig_vs`, `fmaloc_fma_{asin,atan,atan2}` |
| **K6a** | a `const matN` column (`LoadLocal` of a constant local) in the matrix-product arms: per lane `FMA` with the non-zero entries as FAU words, zero entries folded out | `nir.rs:6776` | 9 | matrix.mul.const_*_mat*_vec*_vertex | `fmaloc_fma_constmat_vec` (highp mat3); **`fmares_k6_const_{highp_mat4_vec4,lowp_mat4_vec4,mediump_mat2_vec2}`** |
| **K6b** | an attribute matrix column (`LoadInput` of a `matN` slot) in the same arms, its lanes the `LD_ATTR_IMM` load's | `nir.rs:6776` | 9 | matrix.mul.dynamic_*_mat*_vec*_vertex | `constprod_fma_matmul_dynamic_vs`, `fmaloc_fma_attrmat_vec` (both highp mat3); **`fmares_k6_dyn_{mediump_mat4_vec4,lowp_mat2_vec2}`** |
| **K6c** | an attribute (or a local copy of one) times a **uniform** matrix: the roles of the arm's splat and column swapped, Mesa's summation order kept | `nir.rs:6776` | 6 | matrix.mul.dynamic_highp_mat*_mat*_vertex 3, matrix.mul_assign.dynamic_highp_mat*_vertex 3 | `fmaloc_fma_attrmat_unimat` (mat4), `constprod_fma_mulassign_vs` (mat2); **`fmares_k6_dyn_highp_{mat2_mat2,mat3_mat3}`, `fmares_k6_mulassign_highp_mat4`** |
| **K7a** | `refract`'s `ffma(fneg(..), N, eta * I)` of vertex lanes (with its `sqrt`/`k < 0` select; `distance` and `fract` in the invariance shaders) | `nir.rs:6776` | 17 | operator.geometric.refract 8; invariance.{highp,mediump,lowp} 9 | `constprod_fma_geom_refract_vs` (highp float), `constprod_fma_invariance_vs` (highp.highp); **`fmares_k7_refract_{highp_vec3,mediump_vec4}`, `fmares_k7_inv_{lowp_lowp,mediump_mediump,highp_lowp}`** |
| **K7b** | scalar `reflect`: `is_reflect` takes `fmul(n, i)` where it wants the `fdot` (`fmaloc-lowering` 3.6) | `nir.rs:6776` | 2 | operator.geometric.reflect.*_float_vertex | `constprod_fma_geom_reflect_vs`, `fmaloc_fma_reflect_float` (both highp); **`fmares_k7_reflect_mediump_float`** |
| | | | **116** | | |

### 3.1 What the new references show: half of K6 and most of K7 are fp16

Every K6/K7 reference before this item was **highp**. The new lowp and
mediump ones show that Mesa computes these vertex products at **fp16**:
`fmares_k6_const_lowp_mat4_vec4` is 6 `FMA.v2f16` (29 `vs.varying` words),
`k6_const_mediump_mat2_vec2` 2 `FMA.v2f16`, `k6_dyn_mediump_mat4_vec4` 8
`V2F32_TO_V2F16` (the attribute columns narrowed) and 8 `FMA.v2f16`,
`k6_dyn_lowp_mat2_vec2` likewise, then `F16_TO_F32` per lane into the
varying store. The same holds for K7: `k7_refract_mediump_vec4` is 14
`FMA.v2f16`, `k7_reflect_mediump_float` 3, `k7_inv_lowp_lowp` and
`k7_inv_mediump_mediump` 20-23 `FMA.v2f16` and 13-16 `FADD.v2f16` in both
the position and the varying program (`fract` is `FROUND.v2f16` and a
subtract), and `k7_inv_highp_lowp` keeps the position chain at fp32 and the
lowp varying chain at fp16. The highp ones (`k6_const_highp_mat4_vec4`,
`k6_dyn_highp_*`, `k6_mulassign_highp_mat4`, `k7_refract_highp_vec3`) are
`FMA.f32` chains as the existing references.

So each of K6 and K7 has two halves: **highp** (K6 12 cases: const 3,
dynamic vec 3, K6c 6; K7 6: refract highp 4, reflect highp 1,
invariance.highp.subexpression_precision_highp 1) needs only the arms; the
**lowp/mediump** half (K6 12, K7 13) also needs the vertex walk's fp16
arithmetic (`V2F32_TO_V2F16` of a loaded lane, `FMA.v2f16`/`FADD.v2f16`
pairs, `F16_TO_F32` at the store), which K5's mediump `atan`/`atan2`
(`kfold_k5_atan_mediump_vec2_vs`, `k5_atan2_mediump_vs`) is the first step to
build. A step that lowers a lowp/mediump product at fp32 is "compiled, not
Mesa's" and must leave it refused.

All 14 new references are refused at the tip with the text
`refusals_pinned` holds, at the catch-all `nir.rs:6776` with the source shape
of their row (scratch probe); none is in `DIFFERS`.

## 4. Dependencies and order

- **K4 before K6** (schedule 2.7): K4's constant arms sit before the
  catch-all where K6a's constant column is tested; K6a must not take a column
  K4 folds whole (`matmul_const`, `fmaloc_fma_constmat_mat` are byte-equal at
  the tip: keep them so).
- **K5 before K7**, and **A7 for K5** (schedule 2.7: A7 fixes the
  `builtin_call.rs` chain order that K5's `asin`/`acos`/`atan` read; A7 has no
  item). K7a's `refract` also reads `builtin_call.rs` (`sqrt`, the `k < 0`
  select), so it follows K5 on the same file.
- **K5's mediump fp16 vertex arithmetic before the lowp/mediump halves of K6
  and K7** (section 3.1). The highp halves need only K4 (K6) and K5's
  ordering of `builtin_call.rs` (K7).
- K6 and K7 both edit the catch-all's arm list: add each arm **after** the
  existing `is_mat_term`/`is_fmod`/`is_cross`/`is_mix`/`is_reflect` arms
  (`mali-fmaloc-lowering.md` 5, verdicts at risk) and run
  `mali_glmark2_corpus`, `mali_vs_fma`, `mali_deqp_constprod`,
  `mali_deqp_fmaloc` and this test.

Order: **K4 (7) → K5 (66) → K6 highp (12) → K7 highp (6) → K6 lowp/mediump
(12) → K7 lowp/mediump (13)**. K4 and K5 are `mali-train-4`'s (from
`kfold-refs`); K6 and K7 have no item: `schedule-refresh-4` places them.

## 5. Forecast (HOST-ONLY)

| Step | First refusals at the tip | Whose shader then compiles (forecast) |
|---|--:|---|
| K4 | 7 | 7 (`mali-kfold-lowering.md` 4, K4a) |
| K5 | 66 | 32 operator cases; the 34 random ones meet their next text |
| K6 | 24 | 24: the fragment half is dEQP's 7-8-word varying-to-colour shader, which compiles beside the byte-equal `matmul_const`/`matmul_mat` |
| K7 | 19 | 10 operator cases (same fragment harness); the 9 invariance cases not forecast (their fragment shader reads `u_color` and a `dot` of the varying) |

`FusedMultiplyAdd` falls from 116 to 0 after all four; to 43 after K4 and K5
(below the 100 line from K5 on). Upper bounds, no inflow.

## 6. What this note does not know

- The next refusal of each random K5 case and of the invariance cases after
  their step (no compiler change was made here).
- Which entries of a lowp/mediump `const` matrix Mesa keeps as fp16 FAU
  words and which it folds (the zero entries of the highp one are folded
  out); the byte-compare of `k6_const_*` decides, lane by lane.
