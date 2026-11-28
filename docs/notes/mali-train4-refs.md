<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Train 4: the Mesa 26.2.1 reference of every schedule 5.5a step

Plan #387 item `train4-refgaps`, 2026-11-28, integration tip `ad2b6cc9`. The
lowering schedule (`docs/notes/mali-lowering-schedule.md` 5.5a, "Train 4: fixed
work order") has 15 commits. This note checks that each step has a Mesa 26.2.1
reference that pins its shape, names it, and says which shapes were missing and
how they were drawn. cf-dead and the reordered lanes (commits 1, 2) are left out:
`cfdead-refs` is in review. No `src/` file and no existing design, test or
reference was touched.

## 1. Result

One gap, ten references. Every other step already has the references its design
names, and the designs' own tests pin them (section 3).

| gap | step | what | now |
|---|---|---|---|
| the vertex half of nine `load_mat3` `_vertex` cases, and `linkage.varying_type_mat3` | commit 4, load_mat3 (`mali-highpfs-lowering.md` L3) | the design drew `l3_{mat3_to_mat3,float_to_mat3,vec3x3_to_mat3}_vs` and says "Each case's vertex shader (the conversion, fp32) is not pinned here" and "`linkage.varying_type_mat3` was not drawn" (section 7) | drawn: ten `deqp_t4gap_highpfs_l3_*` references, all refused with `Defect(IrIndex)` at the tip |

The 18 `load_mat3` cases are 11 `_vertex` ones (their fragment half is the same
`mat3` compare shader, which `l3_mat3_to_mat3_vs` already pins), 6 fragment ones
(`l3_*_fs`, drawn) and the linkage case. A `_vertex` case is the pair of its own
conversion vertex shader and that fragment shader, and the commit's byte-compare
of "the case" has to cover the vertex words too: they are the new references.

## 2. The ten new references

Names are `tools/mali/testdata/deqp_t4gap_highpfs_l3_<case>_vs` (the linkage
one has no `_vs`); the test is `tests/mali_deqp_t4gap.rs`, one `#[ignore]`d
test per reference and the tables `state_pinned` and `refusals_pinned`.

| reference (after `deqp_t4gap_highpfs_l3_`) | dEQP case (`dEQP-GLES2.functional.shaders.`) | vertex shader |
|---|---|---|
| `mat2_to_mat3_vs` | `conversions.matrix_to_matrix.mat2_to_mat3_vertex` | `attribute mat2 in0; out0 = mat3(in0)` |
| `mat4_to_mat3_vs` | `conversions.matrix_to_matrix.mat4_to_mat3_vertex` | `attribute mat4 in0` |
| `bool_to_mat3_vs` | `conversions.scalar_to_matrix.bool_to_mat3_vertex` | `bool in0 = bool(a_in0)` |
| `int_to_mat3_vs` | `conversions.scalar_to_matrix.int_to_mat3_vertex` | `int in0 = int(a_in0)` |
| `ivec3_ivec3_ivec3_to_mat3_vs` | `conversions.matrix_combine.ivec3_ivec3_ivec3_to_mat3_vertex` | three `ivec3(a_inN)` |
| `vec2_ivec2_float_float_int_bool_bool_to_mat3_vs` | `conversions.matrix_combine.vec2_ivec2_float_float_int_bool_bool_to_mat3_vertex` | seven inputs, four converted |
| `bool_float_int_vec2_bool_bvec2_float_to_mat3_vs` | `conversions.matrix_combine.bool_float_int_vec2_bool_bvec2_float_to_mat3_vertex` | seven inputs, five converted |
| `bool_bvec2_int_vec4_bool_to_mat3_vs` | `conversions.matrix_combine.bool_bvec2_int_vec4_bool_to_mat3_vertex` | five inputs, four converted |
| `float_bvec4_ivec2_bool_bool_to_mat3_vs` | `conversions.matrix_combine.float_bvec4_ivec2_bool_bool_to_mat3_vertex` | five inputs, four converted |
| `varying_type_mat3` | `linkage.varying_type_mat3` | `attribute mat3 in0; varying mediump mat3 var; var = in0;`; fragment `out0 = var` |

State at the tip: **all ten are refused**, with "Valhall back end defect: an
invariant of the pipeline did not hold (IrIndex)" (the fragment walk's load of
the third `mat3` column, the L3 defect). That the refusal sits in the fragment
half is what the existing `l3_*` pins show; whether the vertex halves compile
alone was not measured (no vertex-only test exists). The `vec3_vec3_vec3` vertex
case is `deqp_highpfs_l3_vec3x3_to_mat3_vs` (existing).

What the train-4 commit 4 (load_mat3) does with each pin: when it lifts the
`IrIndex`, run the test, move each reference's `STATE` row to `Ok(true)` if
byte-equal (or `Ok(false)` if it compiles with other words, then name the first
differing instruction in the test's `#[ignore]` text), and drop the
`#[ignore]` of each that is `Ok(true)`, in the same commit. `refusals_pinned`
names a reference that now compiles. A vertex-half difference belongs to the
conversion lowering (`mali-intval*` / `mali-boolpix`), not to L3: say so in the
commit and leave that reference `Ok(false)`.

## 3. How they were drawn

* Sources: `data/gles2/shaders/conversions.test` and `linkage.test` of
  VK-GL-CTS 93bca01 (the `VK-GL-CTS-gles3` clone of the dependency cache; the
  gles2 `data` tree read from its git objects, the clone's sparse checkout holds
  only gles3). A scratch script, not committed, expanded each case's `both ""` /
  `vertex ""` / `fragment ""` text with the ShaderLibrary template exactly as
  the committed `deqp_highpfs_l3_*` and `deqp_boolpix_*` references show it:
  attribute declarations (`float` for bool and int, `vecN` for bvecN and ivecN,
  named `a_inN`, with a `T inN = T(a_inN);` setup line per converted input,
  at the template's indent for the first and column 0 for the next), `varying
  mat3 out0;`, `gl_Position = dEQP_Position;`, and the `mat3` `isOk` fragment
  shader of `deqp_highpfs_l3_float_to_mat3_vs`. The linkage pair follows
  `deqp_boolpix_lk_uniform_struct` (`${VERTEX_DECLARATIONS}`,
  `${FRAGMENT_DECLARATIONS}`, `${FRAGMENT_OUTPUT}`).
* The multi-line setup of the five multi-input `matrix_combine` cases (no
  committed reference has two converted inputs) follows `specializeVertexShader`'s
  one `setup << ...;\n` per input as read from the template; it was not
  compared with a dEQP `.qpa` log. A case whose real text differs only in that
  indent compiles to the same words (whitespace does not change a compile).
* Mesa: `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_t4gap_<name> --instrument shim`
  on the G610 drm-shim (`tools/mali/drm_shim_ref.sh build`, Mesa 26.2.1, built by
  `deqp_ref.sh` into `$CONNECTED_DEPS_DIR/mesa-shim` when absent;
  `gl_ref.py --uniform-values`). To check the shim, `deqp_highpfs_l3_float_to_mat3_vs`
  was redrawn first and its `expect.json` came out equal but for the trace name.
* Regeneration check: each of the ten was drawn again from its committed
  `.vert`/`.frag` with the command above into a scratch prefix; every
  `expect.json` is equal to the committed one but for the `trace` line (the
  prefix). Re-run:

  ```text
  for e in tools/mali/testdata/deqp_t4gap_*.expect.json; do b=${e%.expect.json}
    tools/mali/deqp_ref.sh $b.vert $b.frag "$TMPDIR/$(basename $b)" --instrument shim; done
  ```
  `nir_refs.py` was not needed: no shape has an explicit attribute binding or
  `--mat-columns` (the `mat3` varyings are plain).
* Run the test:
  `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_t4gap`
  (2 pass, 10 ignored).

## 4. Every step and its references

Commit numbers are 5.5a's. "(new)" is a `deqp_t4gap_*` reference of this item.
The test file that pins each is the design's own (a `LOWERED` / `OPEN_STEPS` /
`STATE` table); no existing test or reference was edited.

| Commit | Step | References |
|--:|---|---|
| 1, 2 | cf-dead; reordered lanes | **left out** (`cfdead-refs`, in review) |
| 3 | audit F1 (+ D1 residue) | `deqp_audit_f1_*` (10), `deqp_audit_d1_conv_*` (5), `tests/mali_deqp_audit.rs` |
| 4 | load_mat3 | `deqp_irindex_load_mat3`, `deqp_highpfs_l3_{mat3_to_mat2_fs,mat3_to_mat4_fs,vec3x3_to_mat3_fs,float_mat3_fs,mat3_to_mat3_vs,float_to_mat3_vs,vec3x3_to_mat3_vs}`; **new** the ten of section 2 |
| 5 | fix C | `deqp_defects_lanecycle_{fs,reduced}`, `deqp_kfold_c_{conv29_fs,cmp29_fs,expr58_fs,conv74_fs}` (the Mesa words the schedule marked "still to draw" are drawn by `kfold-refs`) |
| 6 | mat4_pair_unread | `deqp_irindex_mat4_pair_unread`, `deqp_highpfs_m4_*` (5) |
| 7 | step 4a's pair trees, D1 pair residue | `deqp_arith_*` storeout/op shapes, `deqp_highpfs_pt_*` (21), `deqp_audit_d1_conv_*` |
| 8 | highp fragment shapes | `deqp_highpfs_hp_*` (17) (HF1, HF2) |
| 9 | K2 | `deqp_constprod_{neglit_pair_*,chainlit_*}`, `deqp_kfold_k2_*` |
| 10 | K4 | `deqp_constprod_fma_foldcall_*`, `deqp_kfold_k4_*` |
| 11 | K5 | `deqp_constprod_fma_{trig_*,random_trig}_vs`, `deqp_kfold_k5_*` |
| 12 | T1 | `deqp_fmaloc_tc_cube_{face,const,rnd_sub}`, `tests/mali_deqp_fmaloc.rs` |
| 13-15 | loops A0 .. LS2-vs | `deqp_loops_*`, `deqp_loops2_*` (207 files incl. the 80 of `mali-loops2-lowering.md`), `deqp_loops3_*`, `deqp_loops4_*`, `deqp_vsloop_*`, `deqp_vsloop2_*`, `deqp_idxloc_*` (18, `mali-deqp-idxloc.md`) |

Gaps looked for and not found: the loops design's LP5 line "the other 10 have none"
predates its own 80 references (`OPEN_STEPS`), and idxloc's 156 cases
share its 18 shapes; K5, K4's rest and fix C have `kfold-refs`' references;
T1 and F1/D1 were drawn by `fmaloc-lower-1` and `audit-ref`. K5 still waits for A7
(no reference to draw). `deqp_highpfs_l3_*` names its own fragment shapes
`_fs` and its vertex ones `_vs`; the new ones continue that.

## 5. Not done here

* The G310 check (`tools/mali/g310_check.sh 'deqp_t4gap_*'`) was not run;
  `mali-g310-vs-g610.md` holds for the earlier 869 references and the ten
  shapes are plain fp32 conversions.
* `tests/mali_deqp_t4gap.rs` is not in a `tools/check.sh` row; the
  `mali-deqp-tests-runner` item (run every `tests/mali_deqp_*` with its header's
  feature set) is what runs it.
* At the tip `capi/Cargo.toml` has `vivante-st` twice (lines 41 and 85), so
  `cargo` cannot load the workspace; the test was run with line 85 removed
  and the file restored. Not touched by this item; someone owning `capi/` should fix it.
