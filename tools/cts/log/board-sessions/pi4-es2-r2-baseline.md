<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (merge `64e1714ef`, ccfarm `01a12a23-b55e-7253-b8b3-1395860b6b5a`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# pi4-es2-r2 baseline: what the Pi 4 dEQP-GLES2 tree run at the tip is compared against

Date: 2026-10-11. Written on the host at integration tip `89d518ee9451`. Nothing
here ran on the Pi 4. This file is the reference for item `pi4-es2-tip-r2`;
that session adds its own results in `tools/cts/log/board-sessions/pi4-es2-tip-r2.md`.

## Limit: `fe5e8e73` is not in this repository

The last full Pi 4 tree ran at `fe5e8e73` (2026-10-01). This repository's
history starts at `afb1633` (the import of 2026-10-04), so `git log
fe5e8e73..HEAD` fails ("unknown revision"). "Commits since" below means
`git log afb1633..HEAD` (586 commits) plus what the docs say about the
window before the import. Every `fe5e8e73` number comes from the committed
documents named in each section, not from a re-run.

## 1. The last full tree, `fe5e8e73` (sweep 30)

Sources read (read only): `tools/cts/RESULTS.md`, section "Pi 4: sweep 30, the
whole dEQP-GLES2 tree in one consistent sweep (2026-10-01)" and its image
table; `docs/es2-cts-failures.md` sections Summary, A (Raspberry Pi 4), B, C;
`tools/cts/log/board-sessions/pi4-es2-r1.md` and `pi4-es2-r1-tip.md` (both say nothing
ran; they hold no counts, only the expectation below).

Whole tree (mustpass 17,165, every case has a result):

    TALLY at fe5e8e73: 17,075 pass / 25 fail / 62 NotSupported / 3 QualityWarning

All 29 images were built from one commit and run in one sweep. Zephyr
`v4.4.0-9848-gaf17c0c8aa2a`. Row = 0-based position among the non-comment
lines of `tools/cts/pi4-full-tree.txt` (the `TREE_ROWS` number).

Per row. "Mustpass names" is the count in the tree file's comment. "Cases run"
and the four result columns are RESULTS.md's image table; they count what the
image ran, and some filters also run names that are not in
`gles2-mustpass.txt` (202 names in all), so the columns sum to more than
17,165. "QualityWarning" is RESULTS.md's "other" column; dEQP counts it as a
pass, and the Pass column does not include it.

| Row | Label | Case filter (`dEQP-GLES2.` dropped) | Mustpass names | Cases run | Pass | Fail | NotSupported | QualityWarning |
|---:|---|---|---:|---:|---:|---:|---:|---:|
| 0 | AL | `functional.attribute_location.*` | 57 | 57 | 57 | 0 | 0 | 0 |
| 1 | D | `functional.shaders.algorithm.*, functional.shaders.builtin_variable.*, functional.shaders.conditionals.*, ... (13 filters)` | 292 | 305 | 304 | 0 | 1 | 0 |
| 2 | E | `functional.shaders.constants.*, functional.shaders.keywords.*, functional.shaders.scoping.*, ... (4 filters)` | 352 | 453 | 453 | 0 | 0 | 0 |
| 3 | F1 | `functional.fragment_ops.*` | 1923 | 1923 | 1923 | 0 | 0 | 0 |
| 4 | FB1 | `functional.fbo.api.*, functional.fbo.completeness.*` | 390 | 392 | 365 | 0 | 27 | 0 |
| 5 | FB2 | `functional.fbo.render.*` | 319 | 319 | 319 | 0 | 0 | 0 |
| 6 | FF | `functional.flush_finish.*` | 5 | 5 | 5 | 0 | 0 | 0 |
| 7 | H | `functional.rasterization.*, functional.clipping.*` | 654 | 654 | 651 | 0 | 0 | 3 |
| 8 | LA | `functional.light_amount.*` | 19 | 19 | 19 | 0 | 0 | 0 |
| 9 | MISC | `functional.depth_stencil_clear.*, functional.polygon_offset.*, functional.multisample.*, ... (4 filters)` | 70 | 74 | 58 | 0 | 16 | 0 |
| 10 | NS | `functional.negative_api.*, functional.state_query.*` | 642 | 642 | 633 | 0 | 9 | 0 |
| 11 | RT | `functional.shaders.random.texture.*` | 200 | 200 | 200 | 0 | 0 | 0 |
| 12 | S1 | `functional.shaders.operator.*` | 2949 | 2949 | 2949 | 0 | 0 | 0 |
| 13 | S2 | `functional.shaders.matrix.*, functional.shaders.swizzles.*, functional.shaders.conversions.*` | 1910 | 1910 | 1910 | 0 | 0 | 0 |
| 14 | S3 | `functional.shaders.loops.*, functional.shaders.indexing.*, functional.shaders.preprocessor.*, ... (4 filters)` | 2171 | 2188 | 2188 | 0 | 0 | 0 |
| 15 | S4 | `functional.shaders.functions.*, functional.shaders.invalid_implicit_conversions.*, functional.shaders.random.basic_expression.*, ... (9 filters)` | 2012 | 2027 | 2027 | 0 | 0 | 0 |
| 16 | S5f | `functional.shaders.random.all_features.fragment.*` | 98 | 100 | 100 | 0 | 0 | 0 |
| 17 | S5v | `functional.shaders.random.all_features.vertex.*` | 100 | 100 | 100 | 0 | 0 | 0 |
| 18 | SMALL | `info.*, functional.prerequisite.*, functional.color_clear.*, ... (10 filters)` | 227 | 227 | 227 | 0 | 0 | 0 |
| 19 | T1 | `functional.texture.completeness.*, functional.texture.format.*, functional.texture.wrap.*, ... (4 filters)` | 181 | 224 | 206 | 18 | 0 | 0 |
| 20 | T2 | `functional.texture.filtering.*` | 248 | 248 | 248 | 0 | 0 | 0 |
| 21 | T3 | `functional.texture.mipmap.*, functional.texture.specification.*` | 304 | 304 | 298 | 6 | 0 | 0 |
| 22 | T4 | `functional.texture.size.2d.1024x1024_l8, functional.texture.size.2d.1024x1024_l8_mipmap, functional.texture.size.2d.1024x1024_rgb888, ... (26 filters)` | 67 | 69 | 69 | 0 | 0 | 0 |
| 23 | T5 | `functional.texture.size.2d.2048x2048_l8, functional.texture.size.2d.2048x2048_l8_mipmap, functional.texture.size.2d.2048x2048_rgb888, ... (7 filters)` | 7 | 7 | 7 | 0 | 0 | 0 |
| 24 | TF | `functional.shaders.texture_functions.*` | 17 | 20 | 19 | 1 | 0 | 0 |
| 25 | U1 | `functional.uniform_api.*` | 1267 | 1267 | 1267 | 0 | 0 | 0 |
| 26 | VA | `functional.vertex_arrays.*` | 571 | 571 | 571 | 0 | 0 | 0 |
| 27 | DR | `functional.draw.*` | 101 | 101 | 101 | 0 | 0 | 0 |
| 28 | XT | `functional.clip_control.*, functional.debug_marker.*, functional.multisampled_render_to_texture.*` | 12 | 12 | 3 | 0 | 9 | 0 |
| | **sum** | | 17165 | 17367 | 17277 | 25 | 62 | 3 |

Not recorded: a count of the mustpass names only, per row. RESULTS.md gives
the per-row counts above (all names run) and the mustpass-only totals (the
TALLY line). I looked in `tools/cts/RESULTS.md` (sweep 30 section, line
15241 on), `docs/es2-cts-failures.md` section A, and
`tools/cts/imx8mp-full-tree-29460f96.results.tsv` (i.MX8MP only, not Pi 4).
The 202 extra names are not listed by row, so I do not split them.
The per-row numbers of the board session (from `board.sh pi4 tree`) will
also count all names the image ran; compare them with this table, not with
the mustpass-only totals.

The non-passing cases at `fe5e8e73`, as RESULTS.md lists them:

- Fail, 25: all cube cases (section 2).
- QualityWarning, 3 (row 7, H): `rasterization.interpolation.basic.lines`,
  `rasterization.interpolation.projected.lines`,
  `rasterization.interpolation.projected.lines_wide`.
- NotSupported, 62: see section 3.

## 2. Expected changes (52 cases)

Row and image come from `tools/cts/pi4-full-tree.txt`. Every name below is in
`tools/cts/gles2-mustpass.txt` (checked by script). Prefix every name with
`dEQP-GLES2.functional.`.

### 2a. The 25 cube cases of issue #12: Fail -> Pass expected

Why they failed: the V3D filters cube maps seamlessly and dEQP's ES 2.0
context is judged against the non-seamless reference
(`docs/es2-cts-failures.md` section B, rows 3-27). Fixed on the host by
`c3e4146` (design and failing-then-passing test), `ec5ba4f` (implementation:
`LodKey::cube_noseam`, a cube look-up as a 2D-array look-up of the selected
face), `53ef6d7` (review fixes) and `d8ce931` (doc). Nothing proved this on
the board yet. Per image, the sweep-30 tally moves like this if all 25 pass:
T1 (row 19) 206 pass / 18 fail -> 224 / 0; T3 (row 21) 298 / 6 -> 304 / 0;
TF (row 24) 19 / 1 -> 20 / 0.

| # | Case (`dEQP-GLES2.functional.` dropped) | Image | Row |
|---:|---|---|---:|
| 1 | `shaders.texture_functions.vertex.texturecubelod` | TF | 24 |
| 2 | `texture.mipmap.cube.basic.linear_linear` | T3 | 21 |
| 3 | `texture.mipmap.cube.basic.linear_nearest` | T3 | 21 |
| 4 | `texture.mipmap.cube.bias.linear_linear` | T3 | 21 |
| 5 | `texture.mipmap.cube.bias.linear_nearest` | T3 | 21 |
| 6 | `texture.mipmap.cube.projected.linear_linear` | T3 | 21 |
| 7 | `texture.mipmap.cube.projected.linear_nearest` | T3 | 21 |
| 8 | `texture.vertex.cube.filtering.linear_mipmap_linear_linear_clamp` | T1 | 19 |
| 9 | `texture.vertex.cube.filtering.linear_mipmap_linear_linear_mirror` | T1 | 19 |
| 10 | `texture.vertex.cube.filtering.linear_mipmap_linear_nearest_clamp` | T1 | 19 |
| 11 | `texture.vertex.cube.filtering.linear_mipmap_linear_nearest_mirror` | T1 | 19 |
| 12 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_clamp` | T1 | 19 |
| 13 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_mirror` | T1 | 19 |
| 14 | `texture.vertex.cube.filtering.nearest_mipmap_linear_linear_repeat` | T1 | 19 |
| 15 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_clamp` | T1 | 19 |
| 16 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_mirror` | T1 | 19 |
| 17 | `texture.vertex.cube.filtering.nearest_mipmap_nearest_linear_repeat` | T1 | 19 |
| 18 | `texture.vertex.cube.wrap.clamp_clamp` | T1 | 19 |
| 19 | `texture.vertex.cube.wrap.clamp_mirror` | T1 | 19 |
| 20 | `texture.vertex.cube.wrap.clamp_repeat` | T1 | 19 |
| 21 | `texture.vertex.cube.wrap.mirror_clamp` | T1 | 19 |
| 22 | `texture.vertex.cube.wrap.mirror_mirror` | T1 | 19 |
| 23 | `texture.vertex.cube.wrap.mirror_repeat` | T1 | 19 |
| 24 | `texture.vertex.cube.wrap.repeat_clamp` | T1 | 19 |
| 25 | `texture.vertex.cube.wrap.repeat_mirror` | T1 | 19 |

Read a failure with `docs/v3d-noseamless-cube.md` ("Reading a failure"):
all 25 fail = the TMU does not take TMUI as a float face index; only the six
`nearest_mipmap_{nearest,linear}_linear_*` filtering cases fail = the c = 0.5
switch; only the six `texture.mipmap.cube.*` fail = the fragment stage's
implicit level (FDX/FDY fallback not written).

### 2b. The 27 `fbo.completeness.attachment_combinations.<a>_<b>_<c>_tex` cases: NotSupported -> Pass expected

At `fe5e8e73` all 27 were NotSupported (RESULTS.md: "the V3D does not claim
the depth-texture extensions"). The V3D claims `GL_OES_depth_texture` since
2026-10-03 (`tools/cts/STATUS.md`, `docs/HANDOFF.md`; the change is in the
window before the import). `a`, `b`, `c` are each one of `none`, `rbo`,
`tex`. All run in image FB1, row 4 (sweep 30: 365 pass / 27 NotSupported of
392). If all 27 pass, FB1 is 392 pass / 0 NotSupported. If they still fail
the stencil attachment, it is a V3D driver fix (`docs/es2-cts-failures.md`
section C).

| # | Case (`dEQP-GLES2.functional.fbo.completeness.attachment_combinations.` dropped) | Image | Row |
|---:|---|---|---:|
| 1 | `none_none_none_tex` | FB1 | 4 |
| 2 | `none_none_rbo_tex` | FB1 | 4 |
| 3 | `none_none_tex_tex` | FB1 | 4 |
| 4 | `none_rbo_none_tex` | FB1 | 4 |
| 5 | `none_rbo_rbo_tex` | FB1 | 4 |
| 6 | `none_rbo_tex_tex` | FB1 | 4 |
| 7 | `none_tex_none_tex` | FB1 | 4 |
| 8 | `none_tex_rbo_tex` | FB1 | 4 |
| 9 | `none_tex_tex_tex` | FB1 | 4 |
| 10 | `rbo_none_none_tex` | FB1 | 4 |
| 11 | `rbo_none_rbo_tex` | FB1 | 4 |
| 12 | `rbo_none_tex_tex` | FB1 | 4 |
| 13 | `rbo_rbo_none_tex` | FB1 | 4 |
| 14 | `rbo_rbo_rbo_tex` | FB1 | 4 |
| 15 | `rbo_rbo_tex_tex` | FB1 | 4 |
| 16 | `rbo_tex_none_tex` | FB1 | 4 |
| 17 | `rbo_tex_rbo_tex` | FB1 | 4 |
| 18 | `rbo_tex_tex_tex` | FB1 | 4 |
| 19 | `tex_none_none_tex` | FB1 | 4 |
| 20 | `tex_none_rbo_tex` | FB1 | 4 |
| 21 | `tex_none_tex_tex` | FB1 | 4 |
| 22 | `tex_rbo_none_tex` | FB1 | 4 |
| 23 | `tex_rbo_rbo_tex` | FB1 | 4 |
| 24 | `tex_rbo_tex_tex` | FB1 | 4 |
| 25 | `tex_tex_none_tex` | FB1 | 4 |
| 26 | `tex_tex_rbo_tex` | FB1 | 4 |
| 27 | `tex_tex_tex_tex` | FB1 | 4 |

## 3. Every other case that was not a pass at `fe5e8e73`

All are expected unchanged unless a commit is named. The first five lines
are the 35 NotSupported cases that are not in section 2b; the last line is
the 3 QualityWarning cases:

| Cases | Count | Row (image) | Expected | Commit since that may matter |
|---|---:|---|---|---|
| `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors, face_culling, initial, modify_get, origin, viewport_bounds}` | 8 | 28 (XT) | NotSupported, unchanged (EXT_clip_control is not claimed) | none |
| `multisampled_render_to_texture.readpixels` | 1 | 28 (XT) | NotSupported, unchanged | none |
| `multisample.{common_edge_big_quad, common_edge_small_quads, common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both, constancy_both_inverted, constancy_sample_coverage, constancy_sample_coverage_inverted, depth, num_samples_line, num_samples_polygon, proportionality_alpha_to_coverage, proportionality_sample_coverage, proportionality_sample_coverage_inverted, sample_coverage_invert, stencil}` | 16 | 9 (MISC) | NotSupported, unchanged (no 4x config on the Pi 4) | none |
| `negative_api.texture.compressedtexsubimage2d_{invalid_size, invalid_target, level_max_cube, level_max_tex2d, neg_level_cube, neg_level_tex2d, neg_offset, neg_wdt_hgt, offset_allowed}` | 9 | 10 (NS) | NotSupported, unchanged ("No suitable compressed formats found") | none found by a subject search; the merge `9b77870` names `negative_api` (ES 3.0 GL-core work), so read this row's names, do not assume |
| `shaders.fragdata.invalid_assign_to_1` | 1 | 1 (D) | may change to Pass: `docs/es2-cts-failures.md` predicts it (an ES 2.0 context now reports one draw buffer). A prediction, not a measurement | `4e9397c` (`gl_MaxDrawBuffers` = 1, no `GL_MAX_DRAW_BUFFERS` in `report-es2`) |
| `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide` (QualityWarning, counts as a pass) | 3 | 7 (H) | QualityWarning, unchanged (line interpolation) | none |

Check: 8 + 1 + 16 + 9 + 1 = 35; 35 + 27 = 62 NotSupported. Rows 9 (MISC) and
10 (NS) also give 16 and 9, as the table in section 1.

Other changes in the window that could reach the Pi 4 through shared code,
listed in `docs/es2-cts-failures.md` section D (risk H or M for the Pi 4):
`632687b` (sampler-state shader variants and LRU slots), `2b219c5` (32
depth/stencil slots on every GPU), `84b5761` and `17605c3` (default shader
back end moved to the in-process compiler), `4e9397c` (ES 2.0 context
reports), and later GL-core and ES 3.0 work (for example `2673197`
`GL_OES_texture_npot`, `a54bbde` and `289fb9c` ES 2.0 vertex-attribute
refusals). None is a known Pi 4 regression; the Pi 4 has not run a dEQP
image since `fe5e8e73`, so any row that is not equal to section 1 apart
from the 52 cases above is news and goes in the record by name.

Expected whole-tree TALLY if all of section 2 is true and section 3 is
unchanged:

    17,075 + 25 + 27 = 17,127 pass / 0 fail / 35 NotSupported / 3 QualityWarning

(Add 1 pass and take 1 NotSupported away if `invalid_assign_to_1` also
passes.) `docs/v3d-noseamless-cube.md` says 17,100 / 0 / 62 / 3 for the cube
fix alone; the line above also counts the 27.

## 4. Host check at the tip

Commit under test: `89d518ee9451` (integration tip at the start of this item).
The bare command `cargo test --test v3d_cube_noseamless` does not build: the
crate has no default GPU feature ("enable exactly one GPU feature", "no shader
back end"). The test is run the way `tools/check.sh` runs it (lines 326-327
and 356-357), in the V3D build and the V3D ES 2.0 build:

    cargo test --no-fail-fast --no-default-features \
      --features "host-test,gpu-v3d,shaderc-online,no-shader-table" \
      --test v3d_cube_noseamless
    cargo test --no-fail-fast --no-default-features \
      --features "host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2" \
      --test v3d_cube_noseamless

Result: 7 passed, 0 failed, 0 ignored in each build (the 7 tests of
`docs/v3d-noseamless-cube.md` "Host evidence", including
`an_unkeyed_program_is_unchanged`).

The related unit tests that document names (`--lib`, same two feature sets):

    cargo test --no-fail-fast --no-default-features --features "<set>" --lib -- \
      non_seamless a_vertex_key_alone the_cube_key_follows black_stand

Result: 5 passed, 0 failed in each build:
`gl::draw::tests::the_cube_key_follows_the_filters`,
`shaderc::online::backend::tests::a_v3d_cube_program_has_a_non_seamless_variant`,
`shaderc::online::backend::tests::a_vertex_key_alone_relinks_the_base_program`,
and the two `v3d::backend::backend_tests` black stand-in tests. The wider
filter `-- cube` passes 51 (ES 2.0 build, 5 ignored) and 57 (ES 3.0 build)
`--lib` tests.

So the cube fix is still in at the tip, as far as the host can show. The two
things no host test can show (the TMU taking TMUI as a float face index and
the fragment stage's implicit level) are the board's.

## 5. For the board session

- Compare the per-row counts of `board.sh pi4 tree` with the table in
  section 1; the changes allowed without a note are the 52 cases in section 2
  (rows 4, 19, 21 and 24) and, if it passes, `fragdata.invalid_assign_to_1`
  (row 1).
- Record the commit, the verdict lines and the TALLY that `board.sh` printed;
  no addresses, UART paths or credentials.

## 6. Addendum, 2026-10-12: what main records since this file was written

Written on the host at main `40dd2107fa2c`. Nothing here ran on the Pi 4 by
this item. Sections 1 to 5 stay as written: `fe5e8e73` is still the last
single-commit full tree, and the table in section 1 is still the one to
compare against. Since then `tools/cts/RESULTS.md` has gained three Pi 4
records (read only; sections "Pi 4: the whole dEQP-GLES2 tree, 0 fail
(2026-10-10 and 10-11)" and "Pi 4: gl_test on `1cf946a9` and the six cube rows
(2026-10-11)"), and `docs/es2-cts-failures.md` was updated to match.

| Record | What it ran | Result |
|---|---|---|
| tree on `a27d69c0`, 29 images | whole tree | 17,083 pass / 9 fail / 34 NotSupported / 3 QualityWarning / 2 CompatibilityWarning / 34 without a result (all in row 3, F1) |
| rows on `1cf946a9` | rows 11 RT, 16 S5f, 20 T2, 21 T3, 22 T4, 24 TF | 934 mustpass cases, 934 pass, 0 fail, 0 lost |
| F1 rerun on `a27d69c0` | row 3 | the 34 cases without a result pass; capture lost 260 other cases (console bridge drop, not a GPU result) |

Composed result (three records, not one commit): **17,126 pass / 0 fail / 34
NotSupported / 5 warnings (3 QualityWarning, 2 CompatibilityWarning)** of
17,165. A single-commit tree has not run since `fe5e8e73`; that is what
`pi4-es2-tip-r2` adds.

How this changes the expectations of sections 2 and 3:

- Section 2a, 25 cube cases: 19 passed on `a27d69c0` (the 18
  `texture.vertex.cube.*` and `texturecubelod`). The 6 `texture.mipmap.cube.*`
  linear cases and 3 `texture.size.cube.15x15_{l8,rgb888,rgba8888}` (row 22,
  T4; they were not in section 2a and passed at `fe5e8e73`) failed on
  `a27d69c0` and pass on `1cf946a9`. So the board session should expect all 25
  to pass, and also `size.cube.15x15_*` to stay a pass: row 22 is 69 / 69.
- Section 2b, 27 `*_tex` cases: pass on the `a27d69c0` tree. Expected to
  pass.
- Section 3: `shaders.fragdata.invalid_assign_to_1` passes on `a27d69c0`
  (it was only a prediction above). The other 34 NotSupported cases (clip_control 8,
  multisample 16, `multisampled_render_to_texture.readpixels` 1,
  `negative_api.texture.compressedtexsubimage2d_*` 9) are as at `fe5e8e73`.
  New warnings: `functional.flush_finish.flush` and `.flush_wait` read as
  CompatibilityWarning on `a27d69c0` (row 6, FF); they were Pass at
  `fe5e8e73`. dEQP counts them as pass; note them if they appear.
- Expected tally for the tip run, from the composed result: 17,126 pass / 0
  fail / 34 NotSupported / 3 QualityWarning / 2 CompatibilityWarning (they sum
  to 17,165). Section 3's 17,127 pass / 35 NotSupported is superseded, because
  fragdata and the 2 flush cases moved. Compare class by class with the TALLY
  line `board.sh` prints.
- Lost cases: the console bridge can drop lines. `tools/board.sh` marks such a
  run `done_lossy` and `deqp-uart-tally.py` counts Lost cases. A lossy row is a
  rerun, not a failure.

### Host check at main `40dd2107fa2c`

Same two commands as section 4 (the bare `cargo test --test
v3d_cube_noseamless` needs the feature set, as noted there):

    cargo test --no-fail-fast --no-default-features \
      --features "host-test,gpu-v3d,shaderc-online,no-shader-table" \
      --test v3d_cube_noseamless
    cargo test --no-fail-fast --no-default-features \
      --features "host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2" \
      --test v3d_cube_noseamless

Result: 8 passed, 0 failed, 0 ignored in each build (section 4 had 7; the
eighth is `a_fragment_lookup_takes_each_pixels_level_from_the_direction`, the
fragment level fix of `1cf946a9`). `an_unkeyed_program_is_unchanged` passes.
The cube fix is still in at the tip.
