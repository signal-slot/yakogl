<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-es2-final-b: Pi 4 dEQP-GLES2 whole tree at the integration tip

Date: 2026-10-12 22:59 to 2026-10-13 01:34 (JST). Raspberry Pi 4 (V3D 4.2),
backend tryboot. Tree base `40dd2107fa2c` (main); the images were built from
`6451057`, which is `40dd2107fa2c` plus this record only (the build string in
every image is `V3D_BUILD=6451057`; Zephyr `af17c0c8aa2a`).

Result: **0 Fail, 0 crash, 0 reset.** All 29 images ran to `verdict=done`, all
17,165 mustpass cases have a result. The 25 cube cases of `fe5e8e73` all pass.

## Recipe

- Images: `tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <img>`
  (29 images, rc=0 each; T5 built with `texture-2048.conf` by the script).
- Run: `TREE_IMAGE_DIR=<img> tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt <rec>/full`.
- Tally: `tools/board.sh pi4 tally <rec>/full`.
- The first run was stopped by the worker's two-hour background limit during
  row 23 (T5); that attempt left a staged image on the board's boot partition
  and two partial record directories. The board had already rebooted into
  Linux. I removed the staged files (boot partition) and moved the two partial
  T5 records aside (not into the tally), then resumed with `TREE_ROWS=23-28`
  into the same record directory. Rows 0-22 come from the first run, rows
  23-28 from the second. All 29 rows ran at the same image set.

## Per-image counts

"Cases" are the result lines the image printed (its filter also selects names
outside the mustpass list, so the sum is 17,367 lines for 17,165 mustpass
cases). Pass / Fail / NotSupported / Warning are the image's own counters
(`deqp: done`). Crash is 0 everywhere (`resets=0`, `boot_verdict=complete`).

| Row | Image | sha256 prefix | Cases | Pass | Fail | NotSupported | Warning | Crash |
|---|---|---|---|---|---|---|---|---|
| 0 | AL | 1d57528c | 57 | 57 | 0 | 0 | 0 | 0 |
| 1 | D | fdedd321 | 305 | 305 | 0 | 0 | 0 | 0 |
| 2 | E | 8bf0d58c | 453 | 453 | 0 | 0 | 0 | 0 |
| 3 | F1 | b3227fd8 | 1923 | 1923 | 0 | 0 | 0 | 0 |
| 4 | FB1 | 16ca5d96 | 392 | 392 | 0 | 0 | 0 | 0 |
| 5 | FB2 | 38174109 | 319 | 319 | 0 | 0 | 0 | 0 |
| 6 | FF | 51c1c95d | 5 | 4 | 0 | 0 | 1 (Compatibility) | 0 |
| 7 | H | cd5aa88d | 654 | 651 | 0 | 0 | 3 (Quality) | 0 |
| 8 | LA | 4716efa6 | 19 | 19 | 0 | 0 | 0 | 0 |
| 9 | MISC | 2292675d | 74 | 58 | 0 | 16 | 0 | 0 |
| 10 | NS | 51c2b20b | 642 | 633 | 0 | 9 | 0 | 0 |
| 11 | RT | 224f4b1b | 200 | 200 | 0 | 0 | 0 | 0 |
| 12 | S1 | 1e2f2490 | 2949 | 2949 | 0 | 0 | 0 | 0 |
| 13 | S2 | db5e14b4 | 1910 | 1910 | 0 | 0 | 0 | 0 |
| 14 | S3 | b8c57045 | 2188 | 2188 | 0 | 0 | 0 | 0 |
| 15 | S4 | 36434ec2 | 2027 | 2027 | 0 | 0 | 0 | 0 |
| 16 | S5f | 268b8d6d | 100 | 100 | 0 | 0 | 0 | 0 |
| 17 | S5v | 810cb3f9 | 100 | 100 | 0 | 0 | 0 | 0 |
| 18 | SMALL | fb67bb4f | 227 | 227 | 0 | 0 | 0 | 0 |
| 19 | T1 | 9781b63e | 224 | 224 | 0 | 0 | 0 | 0 |
| 20 | T2 | 0b229d18 | 248 | 248 | 0 | 0 | 0 | 0 |
| 21 | T3 | 8395ab36 | 304 | 304 | 0 | 0 | 0 | 0 |
| 22 | T4 | 572fb41a | 69 | 69 | 0 | 0 | 0 | 0 |
| 23 | T5 | 56ef7063 | 7 | 7 | 0 | 0 | 0 | 0 |
| 24 | TF | 042737f4 | 20 | 20 | 0 | 0 | 0 | 0 |
| 25 | U1 | f509b99d | 1267 | 1267 | 0 | 0 | 0 | 0 |
| 26 | VA | e147fbe0 | 571 | 571 | 0 | 0 | 0 | 0 |
| 27 | DR | 3caac930 | 101 | 101 | 0 | 0 | 0 | 0 |
| 28 | XT | a7ed92dd | 12 | 3 | 0 | 9 | 0 | 0 |
| | whole tree | | 17,367 | 17,329 | 0 | 34 | 4 | 0 |

## Whole-tree TALLY (`board.sh pi4 tally`, scoped to the mustpass list)

```
== mustpass: 17165 cases in these records' case filters (0 outside them, not run, left out); 17165 with a result (17367 case lines seen)
TOTAL                          17127      0     34      3      1      0
                               pass   fail   n.s.     QW  other   none
```

17,127 pass / 0 fail / 34 NotSupported / 3 QualityWarning / 1
CompatibilityWarning / 0 without a result = 17,165.

## Verdict lines the runner printed

All 29 rows have the form (here AL, FF, H, XT; the others differ only in
`back_after` 295-307 s and `result=legacy:v3d_deqp_0`):

```
board.sh pi4 AL rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 FF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_1 resets=0 record=absent back_after=300s
board.sh pi4 H rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_3 resets=0 record=absent back_after=299s
board.sh pi4 XT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
```

`legacy:v3d_deqp_N` is the image's warning count (FF 1, H 3). Image
counters, for example: `deqp: done 4 passed, 0 failed, 0 not supported, 1
warnings` (FF), `deqp: done 651 passed, 0 failed, 0 not supported, 3 warnings`
(H), `deqp: done 3 passed, 0 failed, 9 not supported, 0 warnings` (XT).

## Every Fail by name

None.

## Cases that are not Pass (38, all with a result)

NotSupported (34):
- `clip_control.{depth_mode_one_to_one,depth_mode_zero_to_one,errors,face_culling,initial,modify_get,origin,viewport_bounds}` (8)
- `multisample.{common_edge_big_quad,common_edge_small_quads,common_edge_viewport_quad,constancy_alpha_to_coverage,constancy_both,constancy_both_inverted,constancy_sample_coverage,constancy_sample_coverage_inverted,depth,num_samples_line,num_samples_polygon,proportionality_alpha_to_coverage,proportionality_sample_coverage,proportionality_sample_coverage_inverted,sample_coverage_invert,stencil}` (16)
- `multisampled_render_to_texture.readpixels` (1)
- `negative_api.texture.compressedtexsubimage2d_{invalid_size,invalid_target,level_max_cube,level_max_tex2d,neg_level_cube,neg_level_tex2d,neg_offset,neg_wdt_hgt,offset_allowed}` (9)

QualityWarning (3):
`rasterization.interpolation.basic.lines`,
`rasterization.interpolation.projected.lines`,
`rasterization.interpolation.projected.lines_wide`.

CompatibilityWarning (1): `flush_finish.flush`.

(All names are under `dEQP-GLES2.functional.`.)

## Comparison with `fe5e8e73` (docs/es2-cts-failures.md section A)

`tools/cts/tree-diff.py` does not exist at this tip; compared by hand.

| | `fe5e8e73` (2026-10-01) | this tree (`6451057`) |
|---|---|---|
| Pass | 17,075 | 17,127 |
| Fail | 25 | 0 |
| NotSupported | 62 | 34 |
| QualityWarning | 3 | 3 |
| CompatibilityWarning | 0 | 1 |

- The 25 cube cases (T1 18, T3 6, TF 1) all pass: T1, T3 and TF show 0 Fail.
- NotSupported fell from 62 to 34: the 27
  `fbo.completeness.attachment_combinations` cases and
  `shaders.fragdata.invalid_assign_to_1` are Pass now; `debug_marker.*` is Pass
  (XT: 3 pass). The 34 left are the same classes as in the section A "Current"
  entry (`clip_control` 8, `multisample` 16,
  `multisampled_render_to_texture.readpixels` 1,
  `negative_api.texture.compressedtexsubimage2d_*` 9).
- Against the section A "Current" composed result (17,126 / 0 / 34 / 5
  warnings), this is the first single-commit tree: one more Pass and one
  warning fewer (that entry had 2 CompatibilityWarnings, this tree has 1;
  the `flush_finish` timing warning is intermittent, per the earlier notes).
- The F1 image ran with no lost result line in this capture (1,923 of 1,923).

Remaining for stage 1 on the Pi 4: nothing that fails. The 34 NotSupported
and 4 warnings are the same classes as before and are not failures.
