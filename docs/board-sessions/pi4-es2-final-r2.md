<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-final-r2: Pi 4 dEQP-GLES2 whole tree from the origin/main tip

Date: 2026-10-12. Images built from `origin/main` tip `d56e79e5`
(`d56e79e532031b6c75d05b453b7fc71e8b71b190`, "Merge pull request #182 from
signal-slot/work/issue-8") in a git worktree. The images report
`V3D_BUILD=d56e79e5`, zephyr `af17c0c8aa2a`. All 29 rows of
`tools/cts/pi4-full-tree.txt` (rows 0-28, 17165 mustpass names) ran on the
Pi 4 (V3D) through the serial bridge console.

Result: 17165 of 17165 mustpass names have a result. **0 Fail, 0 crash,
0 timeout, 0 without a result.** 17127 Pass, 34 NotSupported,
3 QualityWarning, 1 CompatibilityWarning. Every row ended `complete=1`, with
0 GPU recoveries and 0 GPU reset lines.

## Verdict lines printed by the device run

Every row printed `verdict=done boot=tryboot row boot_verdict=complete
result=legacy:v3d_deqp_0 resets=0 record=absent`, for example:

```
board.sh pi4 XT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=307s
```

The whole-tree tally printed by `board.sh pi4 tree` at the end of the run:

```
== mustpass: 17165 cases in these records' case filters (0 outside them, not run, left out); 17165 with a result (17367 case lines seen)
TOTAL                          17127      0     34      3      1      0
                               (pass   fail   n.s.     QW  other   none)
```

## Per row (rows 0-28)

Each line is the image's own `deqp: done` counter. Cases begun = cases with a
result in every row.

| Row | Name | Pass | Fail | NotSupported | Warnings |
|---|---|---|---|---|---|
| 0 | AL | 57 | 0 | 0 | 0 |
| 1 | D | 305 | 0 | 0 | 0 |
| 2 | E | 453 | 0 | 0 | 0 |
| 3 | F1 | 1923 | 0 | 0 | 0 |
| 4 | FB1 | 392 | 0 | 0 | 0 |
| 5 | FB2 | 319 | 0 | 0 | 0 |
| 6 | FF | 4 | 0 | 0 | 1 (CompatibilityWarning) |
| 7 | H | 651 | 0 | 0 | 3 (QualityWarning) |
| 8 | LA | 19 | 0 | 0 | 0 |
| 9 | MISC | 58 | 0 | 16 | 0 |
| 10 | NS | 633 | 0 | 9 | 0 |
| 11 | RT | 200 | 0 | 0 | 0 |
| 12 | S1 | 2949 | 0 | 0 | 0 |
| 13 | S2 | 1910 | 0 | 0 | 0 |
| 14 | S3 | 2188 | 0 | 0 | 0 |
| 15 | S4 | 2027 | 0 | 0 | 0 |
| 16 | S5f | 100 | 0 | 0 | 0 |
| 17 | S5v | 100 | 0 | 0 | 0 |
| 18 | SMALL | 227 | 0 | 0 | 0 |
| 19 | T1 | 224 | 0 | 0 | 0 |
| 20 | T2 | 248 | 0 | 0 | 0 |
| 21 | T3 | 304 | 0 | 0 | 0 |
| 22 | T4 | 69 | 0 | 0 | 0 |
| 23 | T5 | 7 | 0 | 0 | 0 |
| 24 | TF | 20 | 0 | 0 | 0 |
| 25 | U1 | 1267 | 0 | 0 | 0 |
| 26 | VA | 571 | 0 | 0 | 0 |
| 27 | DR | 101 | 0 | 0 | 0 |
| 28 | XT | 3 | 0 | 9 | 0 |
| | whole tree | 17127 | 0 | 34 | 3 + 1 |

## Failing cases

None. The case-by-case tally printed no Fail line.

Not Pass (38 cases), none a failure:

- NotSupported, 34: `functional.clip_control.*` (8: `depth_mode_one_to_one`,
  `depth_mode_zero_to_one`, `errors`, `face_culling`, `initial`, `modify_get`,
  `origin`, `viewport_bounds`); `functional.multisample.*` (16, row MISC:
  `common_edge_big_quad`, `common_edge_small_quads`, `common_edge_viewport_quad`,
  `constancy_alpha_to_coverage`, `constancy_both`, `constancy_both_inverted`,
  `constancy_sample_coverage`, `constancy_sample_coverage_inverted`, `depth`,
  `num_samples_line`, `num_samples_polygon`, `proportionality_alpha_to_coverage`,
  `proportionality_sample_coverage`, `proportionality_sample_coverage_inverted`,
  `sample_coverage_invert`, `stencil`);
  `functional.multisampled_render_to_texture.readpixels` (1);
  `functional.negative_api.texture.compressedtexsubimage2d_*` (9:
  `invalid_size`, `invalid_target`, `level_max_cube`, `level_max_tex2d`,
  `neg_level_cube`, `neg_level_tex2d`, `neg_offset`, `neg_wdt_hgt`,
  `offset_allowed`).
- QualityWarning, 3: `functional.rasterization.interpolation.basic.lines`,
  `functional.rasterization.interpolation.projected.lines`,
  `functional.rasterization.interpolation.projected.lines_wide`.
- CompatibilityWarning, 1: `functional.flush_finish.flush`.

All names are prefixed `dEQP-GLES2.`.

## Diff against pi4-es2-r1-tip.md

`pi4-es2-r1-tip.md` (integration branch, commit `ce60c5e2`) is a blocked note
with no per-case results, so a per-case diff against it is empty. The last
numbers on file are `fe5e8e73`: 25 Fail, all cube cases
(`docs/es2-cts-failures.md` section A; T1 18, T3 6, TF 1 at sweep 30). At
`d56e79e5` rows T1 (224), T3 (304) and TF (20) are all Pass with 0 Fail, so
the 25 cube failures are gone. The 38 non-Pass cases above were not compared
with `fe5e8e73` per case; that diff was not made here
(`tools/cts/tree-diff.py` is not in this tip).

## How the run went

- Build: `tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-d56e79e5`,
  29 images, all `rc=0`. `DEQP_WORK` was the cached `rpi_4b` dEQP stage 1.
  `MESA_V3D_TREE` was not needed by this script.
- Run: `board.sh pi4 tree` with the Pi 4 console on the serial bridge, about
  5 minutes a row. The first invocation was stopped by the worker's 2-hour
  limit while row T5 (23) was on the board; rows 0-22 were finished. The
  board came back to Linux by itself. The stale files that run had staged on
  the boot partition (the T5 image and its one-shot boot file) were removed
  by hand, and the two aborted T5 record directories were moved aside, because
  `board.sh` refuses to reuse them. Rows 23-28 were then run with
  `TREE_ROWS=23-28` into the same record directory, and the final tally above
  covers all 29 rows. Row T5 ran to completion once, from a fresh start.
- Records stay under the gitignored `records/` of the build worktree and are
  not committed.
