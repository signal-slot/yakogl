<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Board results

The newest result of each suite on each board, with the main commit its image
was built from. A new run replaces its row in the same push as the numbers
(the old row stays in git history). The summary in Japanese is
[STATUS.md](STATUS.md); what is open and in which order is
[docs/HANDOFF.md](../../docs/HANDOFF.md).

Run records (`records/<board>/...`, consoles and per-case files) are
gitignored. Every image is booted by the board's own bootloader (U-Boot from
eMMC on the AM62P and the i.MX8MP, the Pi firmware's tryboot on the Pi 4) and
run with `tools/board.sh`.

Earlier measurements are in git history. Until 2026-10-16 this file was the
whole measurement log; code comments that cite a RESULTS.md section by name
or date ("1r", "2026-10-04", "What the 43 MiB are", ...) cite that log, which
is read with `git show ab43a20c5:tools/cts/RESULTS.md`. Paths written as
`ab43a20c5:<path>` name a record removed in the same clean-up, read the same
way.

## ES 2.0: dEQP-GLES2 (stage 1, met)

One main commit, `e9901869a`, on the three boards (2026-10-15/16; issue #870,
closed). Mustpass list: 17,165 cases.

| Board | Pass | Fail | NotSupported | QualityWarning / CompatibilityWarning | GPU resets |
|---|---:|---:|---:|---|---:|
| AM62P, 17 images at 1x (re-run at `a731983d`, 17,371 cases begun: the list plus 206 outside it) | 17,330 | **0** | 34 | 5 / 2 | 0 |
| AM62P, ms4 image (`multisample.*` on the 4x config) | 16 | **0** | 0 | 0 / 0 | 0 |
| AM62P combined (ms4 replaces the 16 1x `multisample.*`) | 17,140 | **0** | 18 | 5 / 2 | 0 |
| i.MX8MP, 29 entries (re-run at `f40a2aba`, 17,371 cases begun: the list plus 206 outside it) | 17,318 | **0** | 34 | 19 (other) | 0 |
| Pi 4, 29 rows (every list case has a result) | 17,126 | **0** | 34 | 3 / 2 | 0 (0 recoveries in 29 records) |

- The 34 NotSupported are the same on the three boards and accepted
  (`docs/es2-cts-failures.md` section C, `tools/cts/es2-accepted.txt`):
  `clip_control` 8, `negative_api.texture.compressedtexsubimage2d_*` 9,
  `multisampled_render_to_texture.readpixels` 1 and `multisample.*` 16 (no
  multisample surface at 1x; the AM62P runs them on its ms4 image).
- The AM62P 1x row is from `a731983d` (2026-10-17,
  `records/am62p/deqp-a731983d/`, 17 images, every case has a result; two
  images lost the serial bridge on the first pass and were run again). It
  replaced the `e9901869a` 1x figure, 17,124 pass on the 17,165-case list. The
  ms4 and combined rows are still from `e9901869a`: the combined 17,140 is that
  17,124 plus the 16 ms4 passes.
- The two CompatibilityWarnings are `flush_finish.flush` and
  `flush_finish.flush_wait`, dEQP's timing check.
- The i.MX8MP's 19 warnings appear on NXP's own driver too.

## Common set (stage 2 input)

The same commit, `e9901869a` (`tools/cts/common-tree.txt`: glmark2 GM, Slint
view off SO and on SV, the AGL cluster CL, gl_test GT).

| Board | glmark2-es2 | Slint SO / SV | Cluster | gl_test | GPU resets |
|---|---|---|---|---|---:|
| AM62P | 289, 33 scenes, 0 failed, 0 GL errors | probes pass | 75.2 fps, 46.2 Mpx/s at 1024x600 | 50 of 50 | 0 |
| i.MX8MP | 33 scenes, 0 failed, **15 or more GL errors** (#814); no console, so no score (43 at `a63fcc5b`, 2026-10-14) | probes pass | probes pass, no fps (no console) | 49 of 49 run, case 16 skipped (#8) | 0 where counted |
| Pi 4 | 188, 33 scenes, 0 failed, 0 GL errors | probes pass | 139.4 fps, 192.8 Mpx/s at 1920x720 | 12 of 12 | 0 |

- The AM62P row is from `1d54a4eb` (2026-10-16, `records/am62p/common-1d54a4eb/`:
  GM `20261016-1915-GM-a54c5bf1`, SO `-1925-SO-6cafbfdc`, SV `-1928-SV-904fa4f0`,
  CL `-1930-CL-6e108434`, GT `-1933-GT-f70582c4`); all five rows done. The
  glmark2 score fell from 462 (`fb9107af`) to 289 with 0 failed scenes and 0
  GL errors; the cause is not investigated.
- AM62P glmark2 is slow in terrain (1.47 s a frame), shadow (0.38 s) and
  refract (2.08 s): GL-call time on the CPU, a GPU `glGenerateMipmap` path
  for the PowerVR is open (HANDOFF "Known defects").
- Pictures are not checked per scene yet: the tally judges console error
  lines only (`docs/notes/stage2-bench-gap.md`).
- i.MX8MP glmark2 GL errors (#814): the cause is the GC7000's 64 MiB GPU
  carve-out, from which every GC allocation (the board layer's 800x600 window
  and 1280x800 screen copy included) is taken. A diagnostic run from branch
  `b421b447` (main `ab43a20c` plus the 240 MiB carve-out of the glmark2
  image's overlay and the first-GL-errors instrumentation), `board-stage.sh
  imx8mp common` twice, read GM `594b120e fc3f002a 6184003f 00258320`: score
  42, 33 ran, 0 failed, **0 GL errors**, in both runs; SO / SV / CL probes
  pass; GT 48 of 49 (case 44, #6) then 49 of 49. Not a row of main: the row
  above is replaced by a run from main once the change lands.

## ES 3.0: dEQP-GLES3 (stage 4)

Mustpass list: 42,810 cases. Images report ES 3.0 (`REPORT_ES2` off).

**AM62P, whole tree** on `ad2b5ed` (main `e51de0af` plus docs; the same
`src/`, `driver/` and `capi/` as main `a63fcc5b`), 2026-10-14:

| Pass | Fail | NotSupported | QualityWarning | No result |
|---:|---:|---:|---:|---:|
| 36,552 | 2,880 | 2,195 | 185 | 998 |

- Every case that is not Pass: `docs/es3/am62p-gles3-notpass.tsv`.
- The 998 without a result: image 7 stops at
  `shaders.builtin_functions.precision.clamp.highp_fragment.vec3`
  (`ResourceError`, 987 cases after it), and 11 slow `texture.filtering.3d`
  cases did not start. Images 20 and 21 hang the GPU in
  `texture.filtering.3d` (`docs/notes/pvr-3d-mipmap-hang.md`).
- Rows re-run later replace their groups (2026-10-14, 0 resets):
  on `cd738b1a` transform_feedback 882 / 882, ubo 1,099 / 1,099,
  occlusion_query 62 / 62, `fbo.blit.conversion` 325 pass / 0 fail / 112
  NotSupported, `fragment_ops.scissor` 26 / 26; on `4e62d171`
  `fragment_ops.blend.fbo_srgb` 1,152 / 1,152.

**AM62P, gl_test ES 3.0 cases** on `cd738b1a`: 196 `occl`, 200 `intrt`, 201
`usamp`, 202 `blitconv`, 203 `tfcap`, 204 `srgb` and 205 `mappbo` all pass, 0
resets.

**Pi 4, first images** on `cd738b1a` (2026-10-14; `tools/cts/pi4-gles3-tree.txt`,
91 images; recipe `docs/es3/board-recipe-pi4.md`): 7 of 91 images (A1, B1,
C1, D1, FB1, FB2, FB3), 3,807 cases, 2,798 pass / 744 fail / 265
NotSupported, 0 resets; `buffer` and `clipping` pass in full. Failing cases:
`docs/es3/pi4-gles3-fails.tsv`. gl_test: 202, 203 and 205 pass; 196 `occl`
(all results 0) and 204 `srgb` (render half refused) fail.

**i.MX8MP**: not run on the board (recipe `docs/es3/board-recipe-imx8mp.md`).
