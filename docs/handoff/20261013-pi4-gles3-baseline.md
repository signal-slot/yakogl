<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gles3-baseline, 2026-10-14, cd738b1a

Pi 4 session on the integration tip. Details, commands and console lines:
`docs/board-sessions/pi4-gles3-baseline.md`; per-case failures:
`docs/board-sessions/pi4-gles3-baseline-fails.tsv`.

## State of main

Pi 4 row, gl_test column: on `cd738b1a` (2026-10-14) gl_test 202 `blitconv`
12/12, 203 `tfcap` 9/9 and 205 `mappbo` 15/15 pass; 196 `occl` fails (all five
queries available, all five results 0) and 204 `srgb` fails (the render half
is refused by name, no sample-half checks ran). 0 MMU faults, 0 recoveries.
Pi 4 dEQP-GLES3, first baseline: 7 of 91 images (A1, B1, C1, D1, FB1, FB2,
FB3) ran, 3,807 cases, 2,798 pass / 744 fail / 265 not supported, every case
with a result, 0 resets. B1 (`buffer`, 695) and C1 (`clipping`, 602) pass in
full.

## Open work

- Pi 4 gl_test 196, 202, 203, 204, 205: done (the "Waiting for a board" item
  for the Pi 4); 196 and 204 are open defects, below.
- Pi 4 dEQP-GLES3: 84 images remain (FB4 is built, `build/` only). Each ran
  in under 30 s of its 240 s window, so the table could be re-split into far
  fewer images.
- The Pi 4 failures seen so far are `fbo.blit.conversion` 261,
  `fbo.completeness.renderable` 105, `fbo.color.*` about 190, `blit.default_framebuffer`
  31, `attribute_location.mixed*` 48, `implementation_limits.*` 11 and
  `draw.random.*` 14: the integer, MRT, 2D-array and layout items of the plan,
  not yet classified case by case.
- `tools/cts/common-images.sh` cannot set `CONFIG_V3D_GL_TEST_CASE(S)`; a
  case option would save editing the Kconfig for every build.

## Known defects

- Pi 4 gl_test 196 `occl`: the V3D occlusion counter reads 0 for every query
  on the board (q0..q4 `available 1`, results 0,0,0,0,0; want 0,1,0,1,1),
  though it counts on the host. Check `OCCLUSION_QUERY_COUNTER`'s enable and
  read path on hardware. Not yet an issue number.
- Pi 4 gl_test 204 `srgb`: the case prints "the render half is the PowerVR's
  (this GPU refuses an sRGB target by name)" and then FAIL, with no checks;
  HANDOFF says the sample half runs on all three boards. Read case 204 in
  `driver/tools/pvrlab/gl_test.c` for the V3D guard.
- `tools/board.sh pi4 tree` run as a session's background job is killed at
  the tool's time limit; it died while FB3 was on the board, left the staged
  `yakogl-FB3.bin` and `tryboot.txt` on `/boot/firmware` (a leftover refuses
  the next run) and printed no tally. Run the tree in rows of at most four, or
  in a job with the longest timeout.

## Next board session

Pi 4: re-run gl_test 196 after the counter is looked at, and 204 after its
guard. Then the remaining tree images: `TREE_IMAGE_DIR=<dir> BOARD_MUSTPASS=<abs
path of gles3-mustpass.txt> tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt
<outdir>` with `TREE_ROWS=<a>-<b>` (rows 0 to 7 ran, row 7 is FB4, which is
built and not run), `DEQP_MODULE=gles3` and `DEQP_WORK` set to the gles3 stage
1 for the image build. One row costs about 5 min 15 s (the run itself under
30 s). Run the five GLES2 regression rows (D, FB1, T1, T3, TF) in the same
session; they did not run here.
