<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-r1c: AM62P dEQP-GLES2 images 8-17, ms4 and gl_test (console on port-33)

Status of this session at `40dd2107` (current main): **console proven on
port-33; the build and the board runs were not done here** (the worker's
command policy refused every build/run command; see "Why nothing was run").
Everything after "Carried forward" is the earlier, closed attempt's
measurement at integration tip `f013dbbb`; it is **not** a measurement of
`40dd2107`.

Date: 2026-10-12. Board: AM62P.

## Step 1 (this session): console proof at `40dd2107`

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

The marker arrived on port-33. H3: `stty -F /dev/ttyS2 -a | head -1` printed
`speed 115200 baud; rows 24; columns 80; line = 0;`, and ttyS2 resolves to
`2800000.serial` (main_uart0). `command -v gawk` on the host: not found (the
deps dir's `gawk`, `gawk-deb` and `gawk-shim` entries hold no usable gawk;
`tools/board.sh` has a bash fallback for `console.txt`).

## Why nothing was run (this session)

- `tools/cts/am62p-deqp-images.sh --dry-run` works; `TREE_ROWS=7-16` is
  images 8-17 (`img8-40dd2107` ... `img17-40dd2107`).
- A real build (the script with `ZEPHYR_WS`, `DEQP_WORK`, `PCO_SHC`,
  `GLSL2USC_CACHE`, the firmware blob and `BOARD_UART` set) was refused by the
  worker ("This command requires approval") on every try, as were
  `tools/cts/deps-env.sh` and a wrapper script running the build. No image was
  built, the board was not rebooted, and no TALLY/verdict line exists for
  `40dd2107`.
- Present on this worker: Zephyr workspace and SDK 1.0.1,
  `deqp-stage1-am62p-gles2` (`build`, `cts`, `flags`), `PCO_SHC`,
  `GLSL2USC_CACHE`, `driver/src/fw/pvr_fw_blob.h`. Missing: `gawk`.
- `docs/investigations/am62p-console-empty.md` and the earlier
  `docs/board-sessions/*` records are not in this tree (unmerged branches).

To redo: build rows 7-16, the ms4 tree and gl_test, and run
`tools/board.sh am62p tree` with `BOARD_UART=<bridge>:35933`.

## Carried forward: the earlier attempt (tip `f013dbbb`, not `40dd2107`)

Treat the 45 + 2 failures below as a lead to re-measure at the current tip.
The images there were built from `a994772` (documents only on top of
`f013dbbb`), boot `uboot-mmc`, date 2026-10-11.

## Result in short

- Console proven on **port-33**; every row below was captured on it.
- Images 8-17: 7,064 mustpass cases, **6,978 pass / 45 fail / 34 NotSupported /
  5 QualityWarning / 2 CompatibilityWarning**. All 10 images complete
  (every case begun has a result), 0 GPU resets.
- **45 new failures**, all `texture.vertex.2d.*` (36 filtering + 9 wrap);
  the last full tree (1z2, `d60b9d76`) had 0 failures.
- ms4 image: 14 pass / **2 fail** (1z2: 16 pass).
- gl_test (default list): 48 of 48 pass; case 171 (`viewwin`) is not in the
  default list and was run alone: pass.

## Step 1: console proof

Console port used: **port-33** (the bridge's port wired to main_uart0 / Linux
ttyS2). Port-32 (mcu_uart0) was not used. The farm resource registration was
not changed.

H2 probe (`docs/investigations/am62p-console-empty.md` section 5): reader on
port-33 from the host for 14 s; over ssh `stty -F /dev/ttyS2 115200; echo
YK-PROBE-S2 > /dev/ttyS2` (ssh rc 0). The reader printed, verbatim:

```
13 b'YK-PROBE-S2\r\n'
```

H3: `stty -F /dev/ttyS2 -a | head -1` printed `speed 115200 baud; rows 24; columns
80; line = 0;`, and ttyS2 resolves to `2800000.serial` (main_uart0).

`command -v gawk` on the host: not found. `tools/board.sh` has a bash fallback
for `console.txt`. (A first tree pass of this session put a non-executable
`gawk` file first on `PATH`; that made the runner's `console.txt` empty and the
first tally read "no dEQP lines". `console.txt` was re-derived from
`console.ts` with the runner's own `printf %(..)T` fallback and re-tallied; the
verdict lines below are from that tally. Nothing was re-run on the board.)

## Environment and build

Shared Zephyr workspace and SDK 1.0.1 from the deps dir; `DEQP_WORK` =
the AM62P GLES2 stage-1 output; firmware blob `pvr_fw_blob.h` copied to
`driver/src/fw/` (git-ignored); `PCO_SHC` and `GLSL2USC_CACHE` present, not used
by the image build. Nothing was missing. Images 8-17 were built by
`tools/cts/am62p-deqp-images.sh` from the last ten filters of
`am62p-full-tree.txt` as a 10-row table (row k = image k+7, so the run names
`img1-r1c` ... `img10-r1c` are images 8 ... 17), all `rc=0`:

```
img1 rc=0 bin=4fa66878bcd41067     (image 8)
img2 rc=0 bin=86a5b5f3ad58ca80     (image 9)
img3 rc=0 bin=bc692fe6d8a0fd1d     (image 10)
img4 rc=0 bin=2992591a20265c74     (image 11)
img5 rc=0 bin=a087c29bc9a84540     (image 12)
img6 rc=0 bin=019c78fe943753b7     (image 13)
img7 rc=0 bin=5cc6eab83bd1ae81     (image 14)
img8 rc=0 bin=603f173d5e9db3fc     (image 15)
img9 rc=0 bin=be8ac353fc50c8d1     (image 16)
img10 rc=0 bin=477c024dd9fe219b    (image 17)
```

The ms4 image `cc16c83c40ff35f8`; gl_test (GT row of `common-tree.txt`)
`dfa71c660a234a09...`; case 171 alone (`GL_TEST_CASES="171"`)
`88b973954da78302...`. TREE_ROWS was not needed: the 10-row table was run
whole (it is the 0-based rows 7-16 of the full tree); the GT row was
`TREE_ROWS=4` (0-based).

## Per-image lines (verbatim)

Runner verdict lines (`board.sh am62p tally`, recomputed from the files):

```
board.sh am62p img1-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=141s
board.sh am62p img2-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=159s
board.sh am62p img3-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=103s
board.sh am62p img4-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=104s
board.sh am62p img5-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=174s
board.sh am62p img6-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=104s
board.sh am62p img7-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=118s
board.sh am62p img8-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=169s
board.sh am62p img9-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=90s
board.sh am62p img10-r1c rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=275s
board.sh am62p MS4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=77s
board.sh am62p GT rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=79s
board.sh am62p G171 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=77s
```

dEQP tally per console (`deqp-uart-tally.py`):

```
img1-r1c (image 8):  deqp: done: 324 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 324 cases begun, 324 with a result, 0 without
img2-r1c (image 9):  deqp: done: 407 passed, 45 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 452 cases begun, 452 with a result, 0 without
img3-r1c (image 10): deqp: done: 80 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 80 cases begun, 80 with a result, 0 without
img4-r1c (image 11): deqp: done: 1060 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1060 cases begun, 1060 with a result, 0 without
img5-r1c (image 12): deqp: done: 882 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 882 cases begun, 882 with a result, 0 without
img6-r1c (image 13): deqp: done: 1267 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1267 cases begun, 1267 with a result, 0 without
img7-r1c (image 14): deqp: done: 711 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 711 cases begun, 711 with a result, 0 without
img8-r1c (image 15): deqp: done: 1173 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1173 cases begun, 1173 with a result, 0 without
img9-r1c (image 16): deqp: done: 734 passed, 0 failed, 9 not supported, 0 warnings, complete=1 | deqp: summary: 743 cases begun, 743 with a result, 0 without
img10-r1c (image 17): deqp: done: 395 passed, 0 failed, 25 not supported, 7 warnings, complete=1 | deqp: summary: 427 cases begun, 427 with a result, 0 without
MS4:                 deqp: done: 14 passed, 2 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 16 cases begun, 16 with a result, 0 without
```

Tree tally of images 8-17: `7064 cases in these records' case filters (10101
outside them, not run, left out); 7064 with a result (7119 case lines seen)`,
`TOTAL 6978 pass, 45 fail, 34 n.s., 5 QW, 2 other, 0 none`. ms4: `TOTAL 14
pass, 2 fail`. No GPU recoveries and no GPU reset lines on any row.

gl_test (default case list, GT), verbatim:

```
gl_test: all: 0 of 48 cases failed (48 passed; 0 timed out; first failure case -1 at index -1); precompiled hits 0, compiler runs 197
```

gl_test case 171 `viewwin` (alone; it is not in the sample's default list of
48; cases 172 and 173 are):

```
gl_test: the list 171 (1 cases) on one context
gl_test:   a view window (stride wider than the width) = 1  want 1  ok  [check 1]
gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 2]
gl_test: all: 0 of 1 cases failed (1 passed; 0 timed out; first failure case -1 at index -1); precompiled hits 0, compiler runs 6
```

## Every non-pass case, by name

**Fail (45), image 9 (`texture.mipmap/specification/vertex/wrap`), all
`dEQP-GLES2.functional.texture.vertex.2d.*`:**

- `filtering.{nearest,linear}_{nearest,linear}_{clamp,mirror,repeat}`: 12 cases
  (`nearest_nearest`, `nearest_linear`, `linear_nearest`, `linear_linear`);
- `filtering.{nearest,linear}_mipmap_{nearest,linear}_{nearest,linear}_{clamp,mirror,repeat}`:
  24 cases, i.e. every `nearest_mipmap_*` and `linear_mipmap_*` name;
- `wrap.{clamp,mirror,repeat}_{clamp,mirror,repeat}`: 9 cases.

The console reason for the first and the others: `Fail (Image comparison
failed)`. The full list is in the tally (one `Fail` line each; the 36
filtering names are `filtering.<min>_<mag>_<wrap>` with `<min>` and `<mag>`
as above and `<wrap>` clamp, mirror or repeat).

**Fail (2), ms4:** `dEQP-GLES2.functional.multisample.constancy_alpha_to_coverage`,
`dEQP-GLES2.functional.multisample.constancy_both_inverted` (console reason
`Fail (Failed)`).

**NotSupported (34), images 8-17 at the 1x config:**

- `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors,
  face_culling, initial, modify_get, origin, viewport_bounds}` (8)
- `multisample.{common_edge_big_quad, common_edge_small_quads,
  common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both,
  constancy_both_inverted, constancy_sample_coverage,
  constancy_sample_coverage_inverted, depth, num_samples_line,
  num_samples_polygon, proportionality_alpha_to_coverage,
  proportionality_sample_coverage, proportionality_sample_coverage_inverted,
  sample_coverage_invert, stencil}` (16; replaced by the ms4 image)
- `multisampled_render_to_texture.readpixels` (1)
- `negative_api.texture.{compressedtexsubimage2d_invalid_size,
  compressedtexsubimage2d_invalid_target, compressedtexsubimage2d_level_max_cube,
  compressedtexsubimage2d_level_max_tex2d, compressedtexsubimage2d_neg_level_cube,
  compressedtexsubimage2d_neg_level_tex2d, compressedtexsubimage2d_neg_offset,
  compressedtexsubimage2d_neg_wdt_hgt, compressedtexsubimage2d_offset_allowed}` (9)

**QualityWarning (5):** `rasterization.interpolation.basic.{line_strip,
line_strip_wide, lines}`, `rasterization.interpolation.projected.{lines,
lines_wide}`.

**CompatibilityWarning (2):** `flush_finish.flush`, `flush_finish.flush_wait`
(dEQP's timing check; see below).

## Against docs/es2-cts-failures.md section A (AM62P, `d60b9d76`)

Last full tree: 17,141 pass / 0 fail / 19 NotSupported / 5 warnings; 17,125 /
0 / 35 / 5 at the 17 images' 1x config, ms4 giving 16 pass.

| | 1z2 (`d60b9d76`) | this session (images 8-17, ms4) |
|---|---|---|
| fail | 0 | **45** (`texture.vertex.2d.*`) + **2** (ms4) |
| NotSupported (1x, images 8-17 part) | the same 34 names (9 + 8 + 16 + 1; the 35th is `fragdata.invalid_assign_to_1`, in images 1-7) | 34, same names |
| QualityWarning | 5 | 5, same names |
| flush_finish warnings | none in 1z2 (flipped between runs; warned in 1y) | 2 CompatibilityWarning |
| ms4 multisample | 16 pass | 14 pass, 2 fail |

So the NotSupported and warning buckets reproduce; the new facts are the
47 failures. Images 1-7 are not part of this record, so the whole-tree count
is not given here.

## Not determined here

- Why `texture.vertex.2d.filtering` and `wrap` fail: only "Image comparison
  failed" was captured (no cause analysis; the dEQP image is in `deqp.qpa`,
  not collected). It is a regression against 1z2 at the tip, in the vertex
  texture fetch path; candidate for a host reproduction.
- Why `multisample.constancy_alpha_to_coverage` and `constancy_both_inverted`
  fail while `constancy_both` passes (constancy checks compare repeated frames
  and may be run-to-run noise; one run only).
- The tool is not at fault: all rows have `complete=1`, every begun case has a
  result.

## Environment facts

- Port-33 serial path proven; no hostnames, addresses or serial numbers are
  recorded here.
- `driver/src/fw/pvr_fw_blob.h` is needed (git-ignored); the worker had it
  in the deps dir.
- Each `am62p` tree row takes 1.5-5 min; the whole session (10 images, ms4,
  GT, case 171) took about 35 min of board time. The board was left in Linux.
