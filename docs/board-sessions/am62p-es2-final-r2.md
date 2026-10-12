<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES2 whole tree from origin/main (final-r2)

Device results. Date: 2026-10-12. Boot: `uboot-mmc`.

All 17 images of `tools/cts/am62p-full-tree.txt` and the ms4 image of
`tools/cts/am62p-ms4-tree.txt` were built from the **origin/main tip
`63efed8e`** (Merge pull request #144, work/issue-128; `git fetch origin`
worked) in a detached worktree, by `tools/cts/am62p-deqp-images.sh`, and run
by `tools/board.sh am62p tree`.

## Result in short

- **0 failures** in the whole tree and in the ms4 image.
- Full tree at 1x (17 images): 17,165 mustpass cases, **17,125 pass / 0 fail /
  34 NotSupported / 5 QualityWarning / 1 CompatibilityWarning**. Every image
  is complete (every case begun has a result), 0 GPU resets.
- ms4 image: **16 pass / 0 fail**. The 16 `multisample.*` cases are
  NotSupported at 1x and pass at 4x, so the combined total is
  **17,141 pass / 0 fail / 18 NotSupported / 5 QualityWarning / 1
  CompatibilityWarning** (17,165 cases).
- The 9 `shaders.invariance.*.common_subexpression_*` failures are **gone**
  (image 1 covers them: 1512 / 1512 pass).
- The 45 `texture.vertex.2d.*` failures are **gone** (image 9: 452 / 452
  pass).
- Not run: gl_test cases 198, 197 and 171 (time; see "Not run").

## Console port

The lease named bridge port 32. That port is wired to mcu_uart0 and Zephyr
prints on main_uart0, as `docs/investigations/am62p-console-empty.md`
explains. The first two images were run on it and both gave `verdict=empty`
(`back_after=427s`, 0 console bytes). I stopped that run. The tree was then
run with the console on the **bridge's port 33** (main_uart0), the port that
am62p-es2-r1c proved. The farm registration was not changed. The two failed
attempts and the first, refused ms4 try were moved out of the tree
directory; the tally still prints a `scan_failed` row for them
(`img1` and `MS4`, `back_after=?s`) because their directories stay in the
ms4 record. They are not results.

One more event: the stopped run left a one-shot image staged on the board's
eMMC. `tools/board.sh am62p disarm` removed it before the real run. Image 8
had its console capture fail once (`scan_failed`, no console files, 423 s).
It was moved aside and rerun alone (`TREE_ROWS=7`), and it passed.

## Build

Zephyr workspace, SDK 1.0.1, the AM62P stage-1 tree, `PCO_SHC`,
`GLSL2USC_CACHE` and `PVR_FW_BLOB` all came from the deps directory
(`tools/cts/deps-env.sh --check am62p`: every entry `ok`). The firmware header
was copied to `driver/src/fw/` (git-ignored, not committed). All builds:

```
img1 rc=0 bin=8350bf0fa562ed0a    img10 rc=0 bin=897aa48d4848de93
img2 rc=0 bin=58d82d68761d1abe    img11 rc=0 bin=138d1755231b09b5
img3 rc=0 bin=08e6fd4ff7b8ad1c    img12 rc=0 bin=1f70ad3259616095
img4 rc=0 bin=75dd5f1cab3c3907    img13 rc=0 bin=b30d153d807bb8cf
img5 rc=0 bin=c9705530f9329f53    img14 rc=0 bin=4cee07cf717d2d5e
img6 rc=0 bin=e1d839b5a6d72ae7    img15 rc=0 bin=11eb9c7343c63ced
img7 rc=0 bin=20745943cd98a791    img16 rc=0 bin=5a3e23c2a2922b6e
img8 rc=0 bin=11d036f71205700c    img17 rc=0 bin=52f23c67a6c7e129
img9 rc=0 bin=69a691094880780a    ms4 img1 rc=0 bin=3fae43ba8cafb66d
```

## Device verdict lines (verbatim, `board.sh am62p tally`)

Full tree:

```
board.sh am62p img1-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=111s
board.sh am62p img2-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=121s
board.sh am62p img3-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=104s
board.sh am62p img4-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=123s
board.sh am62p img5-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=168s
board.sh am62p img6-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=130s
board.sh am62p img7-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=140s
board.sh am62p img8-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=130s
board.sh am62p img9-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=173s
board.sh am62p img10-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=100s
board.sh am62p img11-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=126s
board.sh am62p img12-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=173s
board.sh am62p img13-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=117s
board.sh am62p img14-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=99s
board.sh am62p img15-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=167s
board.sh am62p img16-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=92s
board.sh am62p img17-63efed8e rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=279s
```

ms4 (the one result row; the tally also prints the `scan_failed` row of the
refused first try, see "Console port"):

```
board.sh am62p MS4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=69s
```

Whole-tree TALLY (`board.sh am62p tally`, full tree):

```
group                           pass   fail   n.s.     QW  other   none
TOTAL                          17125      0     34      5      1      0
== mustpass: 17165 cases in these records' case filters (0 outside them, not run, left out); 17165 with a result (17371 case lines seen)
```

ms4 TALLY:

```
group                           pass   fail   n.s.     QW  other   none
multisample                       16      0      0      0      0      0
TOTAL                             16      0      0      0      0      0
== mustpass: 16 cases in these records' case filters (17149 outside them, not run, left out); 16 with a result (16 case lines seen)
```

## Per image (the image's own counters)

| Image | Passed | Failed | NotSupported | Warnings | Begun / with result |
|---|---|---|---|---|---|
| 1 | 1512 | 0 | 0 | 0 | 1512 / 1512 |
| 2 | 994 | 0 | 0 | 0 | 994 / 994 |
| 3 | 1172 | 0 | 0 | 0 | 1172 / 1172 |
| 4 | 1100 | 0 | 0 | 0 | 1100 / 1100 |
| 5 | 2525 | 0 | 0 | 0 | 2525 / 2525 |
| 6 | 1318 | 0 | 0 | 0 | 1318 / 1318 |
| 7 | 1631 | 0 | 0 | 0 | 1631 / 1631 |
| 8 | 324 | 0 | 0 | 0 | 324 / 324 |
| 9 | 452 | 0 | 0 | 0 | 452 / 452 |
| 10 | 80 | 0 | 0 | 0 | 80 / 80 |
| 11 | 1060 | 0 | 0 | 0 | 1060 / 1060 |
| 12 | 882 | 0 | 0 | 0 | 882 / 882 |
| 13 | 1267 | 0 | 0 | 0 | 1267 / 1267 |
| 14 | 711 | 0 | 0 | 0 | 711 / 711 |
| 15 | 1173 | 0 | 0 | 0 | 1173 / 1173 |
| 16 | 734 | 0 | 9 | 0 | 743 / 743 |
| 17 | 396 | 0 | 25 | 6 | 427 / 427 |
| ms4 | 16 | 0 | 0 | 0 | 16 / 16 |

Images 16 and 17 also hold the QualityWarning / CompatibilityWarning cases
(`flush_finish.flush` is the one CompatibilityWarning counted as "other" in
the tally; the 6 warnings of image 17 are the 5 QualityWarning plus it).

## Failing cases

**None.** Not Pass, by name (all are the known acceptable buckets of
`docs/es2-cts-failures.md`, sections C and the NotSupported table):

- NotSupported, 8: `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one,
  errors, face_culling, initial, modify_get, origin, viewport_bounds}`.
- NotSupported, 16 at 1x (pass in the ms4 image): `multisample.*`.
- NotSupported, 1: `multisampled_render_to_texture.readpixels`.
- NotSupported, 9: `negative_api.texture.compressedtexsubimage2d_{invalid_size,
  invalid_target, level_max_cube, level_max_tex2d, neg_level_cube,
  neg_level_tex2d, neg_offset, neg_wdt_hgt, offset_allowed}`.
- QualityWarning, 5: `rasterization.interpolation.basic.{line_strip,
  line_strip_wide, lines}`, `rasterization.interpolation.projected.{lines,
  lines_wide}`.
- CompatibilityWarning, 1: `flush_finish.flush`.

## Comparison with docs/es2-cts-failures.md

| | `32778b57` tree (2026-10-10, in the doc) | this tree (`63efed8e`) |
|---|---|---|
| pass | 17,070 | 17,141 (full tree + ms4) |
| fail | 54 | **0** |
| NotSupported | 34 | 18 (34 at 1x; 16 pass in ms4) |
| warnings | 7 (5 QW, 2 CW) | 6 (5 QW, 1 CW) |

- The 9 invariance failures and the 45 `texture.vertex.2d.*` failures, the
  54 of the doc, are all gone. The pass count equals the `d60b9d76` tree
  (17,141 pass, 0 fail).
- The doc's 34 NotSupported was counted with the 1x tree; the doc's "19" is
  its ms4 composition. Here the 1x tree has 34 and the ms4 image proves 16
  of them, so 18 remain: 8 `clip_control`, 1 `multisampled_render_to_texture`,
  9 `compressedtexsubimage2d`. The doc's 19 is 9 + 8 + 1 + 1, the last being
  `shaders.fragdata.invalid_assign_to_1`; that case is not in the not-Pass list here (the `shaders` group has 10,101 pass, 0 fail), so
  18 is the same set minus that one.
- Warnings: one CompatibilityWarning fewer than the doc (`flush_finish.flush`
  stays; `flush_wait` was clean). The doc already notes this pair is noisy
  between runs.

## Not run

- gl_test cases 198, 197 and 171 (the step said "if time allows"). The
  session spent its time on the console-port problem and the tree.
- The record directories under `records/` are git-ignored and are not in this
  commit.

## Assumptions

- Console on bridge port 33, as in am62p-es2-r1c, because the leased port 32
  is the dead MCU UART. This uses a port of the same adapter that the lease
  did not name.
- Failed attempts (empty console, image 8 capture loss) were moved out of the
  tree directory so the resumable runner would re-run them; their directories
  are kept under `records/`.
