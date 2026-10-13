<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-r1c: AM62P dEQP-GLES2 images 8-17, ms4 and gl_test at `40dd2107` (console on port-33)

Device results at commit `40dd2107` (main). The images were built from
`9a8cc09`, which is `40dd2107` plus this record's first version (documents
only), so the code is main's. Date: 2026-10-12. Boot: `uboot-mmc`, one
install, board returned to Linux after every row.

## Result in short

- Console proven on **port-33**; every row below was captured on it.
- Images 8-17: 7,064 mustpass cases, **7,023 pass / 0 fail / 34 NotSupported /
  5 QualityWarning / 2 CompatibilityWarning**. All 10 images complete (every
  case begun has a result), 0 GPU resets.
- ms4 image: **16 pass / 0 fail**.
- gl_test default list: **50 of 50 pass**; case 171 alone: **pass**.
- Against the last full tree (`docs/es2-cts-failures.md` section A): no
  failure and the same non-pass names; the 45 `texture.vertex.2d.*` and the 2
  ms4 failures of the `32778b57` tree are gone on the board.

## Step 1: console proof

Console port used: **port-33** (the bridge's port wired to main_uart0 / Linux
ttyS2). Port-32 (mcu_uart0) was not used. The farm resource registration was
not changed.

H2 probe: a reader on port-33 from the host (25 s) while, over ssh to the
board, `stty -F /dev/ttyS2 115200; echo YK-PROBE-S2 > /dev/ttyS2` ran (ssh rc
0). The reader printed, verbatim:

```
4 b'YK-P'
13 b'ROBE-S2\r\n'
```

H3: `stty -F /dev/ttyS2 -a | head -1` printed `speed 115200 baud; rows 24;
columns 80; line = 0;`, and ttyS2 resolves to `2800000.serial` (main_uart0).

`command -v gawk` on the host: not found. `tools/board.sh` has a bash fallback
for `console.txt`; every row's `console.txt` was filled and tallied.

## Environment and build

Shared Zephyr workspace and SDK 1.0.1 from the deps dir; `DEQP_WORK` = the
AM62P GLES2 stage-1 output (`deqp-stage1-am62p-gles2`); firmware blob
`pvr_fw_blob.h` in `driver/src/fw/` (git-ignored); `PCO_SHC` and
`GLSL2USC_CACHE` set. Nothing was missing except gawk.

Images 8-17 came from `tools/cts/am62p-deqp-images.sh` with `TREE_ROWS=7-16`
(run in two parts: rows 7-10 first, the rest after the first run was stopped at
its time limit), all `rc=0`, tag `r1c`:

```
img8 rc=0 bin=b2a0169dc022c939
img9 rc=0 bin=3ea00a54903fec5d
img10 rc=0 bin=581a694d2c5c8319
img11 rc=0 bin=f6a6a70c816e8e67
img12 rc=0 bin=cf7d45764efb99b4
img13 rc=0 bin=929cb84981eaa4db
img14 rc=0 bin=47bcee343cff33fd
img15 rc=0 bin=5e688794d6dea969
img16 rc=0 bin=de19fd7b871347d3
img17 rc=0 bin=cabcb299b19799fa
img1 rc=0 bin=69a01ee6206f129e      (ms4 table, label MS4)
```

gl_test: the `GT` row of `tools/cts/common-tree.txt` built by
`tools/cts/common-images.sh am62p ... GT`; case 171 alone built by hand with
`west build` as `gl_test` plus `-DCONFIG_GL_TEST_CASES="171"` and the same
am62p arm options (image sha256 `3a9371dbac073db7...`). Runs: `TREE_IMAGE_DIR`
with `TREE_ROWS=7-16`/`TREE_TAG=r1c` for images 8-17, then the ms4 table, then
GT, then case 171, each with `tools/board.sh am62p tree`.

## Per-image lines (verbatim)

Runner verdict lines:

```
board.sh am62p img8-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=129s
board.sh am62p img9-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=175s
board.sh am62p img10-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=104s
board.sh am62p img11-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=108s
board.sh am62p img12-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=164s
board.sh am62p img13-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=117s
board.sh am62p img14-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=109s
board.sh am62p img15-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=171s
board.sh am62p img16-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=93s
board.sh am62p img17-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=272s
board.sh am62p MS4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=79s
board.sh am62p GT rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=101s
board.sh am62p G171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=76s
```

dEQP tally per console (`deqp-uart-tally.py`):

```
img8  (324):  deqp: done: 324 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 324 cases begun, 324 with a result, 0 without
img9  (452):  deqp: done: 452 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 452 cases begun, 452 with a result, 0 without
img10 (80):   deqp: done: 80 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 80 cases begun, 80 with a result, 0 without
img11 (1060): deqp: done: 1060 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1060 cases begun, 1060 with a result, 0 without
img12 (882):  deqp: done: 882 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 882 cases begun, 882 with a result, 0 without
img13 (1267): deqp: done: 1267 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1267 cases begun, 1267 with a result, 0 without
img14 (711):  deqp: done: 711 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 711 cases begun, 711 with a result, 0 without
img15 (1173): deqp: done: 1173 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1173 cases begun, 1173 with a result, 0 without
img16 (743):  deqp: done: 734 passed, 0 failed, 9 not supported, 0 warnings, complete=1 | deqp: summary: 743 cases begun, 743 with a result, 0 without
img17 (427):  deqp: done: 395 passed, 0 failed, 25 not supported, 7 warnings, complete=1 | deqp: summary: 427 cases begun, 427 with a result, 0 without
MS4   (16):   deqp: done: 16 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 16 cases begun, 16 with a result, 0 without
```

Tree tally of images 8-17: `mustpass: 7064 cases in these records' case
filters (10101 outside them, not run, left out); 7064 with a result (7119 case
lines seen)`, `TOTAL 7023 pass, 0 fail, 34 n.s., 5 QW, 2 other, 0 none`. ms4:
`TOTAL 16 pass, 0 fail`. "GPU reset lines: 0" and "no recoveries line" on
every row.

gl_test, default list (GT), verbatim:

```
gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 208
```

gl_test case 171 `viewwin` alone, verbatim:

```
gl_test: the list 171 (1 cases) on one context
gl_test:   a view window (stride wider than the width) = 1  want 1  ok  [check 1]
gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 2]
gl_test:   eglTerminate                 = 1  want 1  ok  [check 21]
gl_test: all: 0 of 1 cases failed (1 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 6
```

(All 21 checks printed `ok`; the line count of the full output is elided
here.) The earlier record of this item counted 48 cases in the default list;
this tree's sample lists 50, which is the difference.

## Every non-pass case, by name

**Fail: none** (images 8-17 and ms4).

**NotSupported (34):**

- `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors,
  face_culling, initial, modify_get, origin, viewport_bounds}` (8)
- `multisample.{common_edge_big_quad, common_edge_small_quads,
  common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both,
  constancy_both_inverted, constancy_sample_coverage,
  constancy_sample_coverage_inverted, depth, num_samples_line,
  num_samples_polygon, proportionality_alpha_to_coverage,
  proportionality_sample_coverage, proportionality_sample_coverage_inverted,
  sample_coverage_invert, stencil}` (16; "No multisample buffers" at the 1x
  config, all 16 pass in the ms4 image)
- `multisampled_render_to_texture.readpixels` (1)
- `negative_api.texture.{compressedtexsubimage2d_invalid_size,
  compressedtexsubimage2d_invalid_target, compressedtexsubimage2d_level_max_cube,
  compressedtexsubimage2d_level_max_tex2d, compressedtexsubimage2d_neg_level_cube,
  compressedtexsubimage2d_neg_level_tex2d, compressedtexsubimage2d_neg_offset,
  compressedtexsubimage2d_neg_wdt_hgt, compressedtexsubimage2d_offset_allowed}` (9)

(all under `dEQP-GLES2.functional.`)

**QualityWarning (5):** `rasterization.interpolation.basic.{line_strip,
line_strip_wide, lines}`, `rasterization.interpolation.projected.{lines,
lines_wide}`.

**CompatibilityWarning (2):** `flush_finish.flush`, `flush_finish.flush_wait`
(dEQP's timing check).

## Against docs/es2-cts-failures.md section A

Section A's AM62P full tree is `d60b9d76`: 17,141 pass / 0 fail / 19
NotSupported / 5 warnings (17,125 / 0 / 35 / 5 at the 17 images' 1x config,
ms4 16 pass). The summary table and "Status after plan #1's fixes" record a
later `32778b57` tree with 54 failures (45 `texture.vertex.2d.*`, 9
invariance) and the ms4 image's 2 failures, with fixes `a6fc6f14`, `e17fb6a7`
and `a4f64950`.

| | `d60b9d76` | this session (images 8-17, ms4) |
|---|---|---|
| fail | 0 | **0** |
| NotSupported (1x, images 8-17 part) | the same 34 names | 34, same names |
| QualityWarning | 5 | 5, same names |
| flush_finish warnings | none in `d60b9d76` (flips between runs) | 2 CompatibilityWarning |
| ms4 multisample | 16 pass | 16 pass |

So the `texture.vertex.2d.*` fix (`a6fc6f14`) and the ms4 fix (`a4f64950`) are
now **confirmed on the board at main**, for the part of the tree run here.
Images 1-7 (shaders; the invariance fix `e17fb6a7` lives there) are in
`am62p-es2-r1a.md`, not here, so no whole-tree count is given.

## Not determined here

- The two `flush_finish` CompatibilityWarnings are dEQP's timing check and
  flipped between earlier runs; one run only.
- Whole-tree numbers need images 1-7 at the same commit.

## Environment facts

- Port-33 serial path proven; no hostnames, addresses or serial numbers are
  recorded here. (The `board.sh status` output carries the TFTP server's
  address in the U-Boot environment; it was left out.)
- `driver/src/fw/pvr_fw_blob.h` is needed (git-ignored); the worker had it.
- A first background run of the builds was stopped by the harness's 30-minute
  limit after images 8-11; the rest were built with a 2-hour limit. Each
  image takes about 7 minutes to build; each board row 1.5-4.5 min, the board
  session (10 images, ms4, GT, case 171) about 35 min. The board was left in
  Linux.
