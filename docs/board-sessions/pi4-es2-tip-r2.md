<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES2 whole tree at a25712f2 (r2)

Commit under test: `a25712f2` (integration tip). Rows 0-28, all 29 images,
every one ran on the Pi 4 with the console read over the TCP serial bridge
(`serial_capture.py`, probed free first; the address is not recorded).

Result: **17,165 of 17,165 mustpass cases have a result.**
TALLY: 17,118 pass / 9 fail / 34 NotSupported / 3 QualityWarning /
1 CompatibilityWarning (`TOTAL` line below; expected if all 52 passed:
17,127 / 0 / 35 / 3). No crash, no GPU reset (`recoveries: 0` in every row).

## Images

Built fresh at this tip with `tools/cts/pi4-deqp-images.sh` (29 images, rc=0;
the banner says `build from 0950761`, which is a25712f2 plus an earlier
docs-only version of this record; zephyr `af17c0c8aa2a`). The old set
`pi4-4451f136` was not reused: `git diff --name-only 4451f136 HEAD` over the
image paths lists `src/gl/framebuffer.rs`.

## How the run went

- Rows 0-22 ran in one `board.sh pi4 tree`. The harness stopped that job at its
  time limit during T5; the board then stayed off the network for some minutes
  and rebooted by itself, and a staged image and `tryboot.txt` of the stopped
  run were left in /boot/firmware (board.sh refused: "a run that did not clean
  up"). I removed those two files (`yakogl-T5.bin`, `tryboot.txt`) and resumed.
- A second interruption (worker restart) hit row U1 the same way, with the same
  cleanup. Rows T5 and U1 therefore have a `not_back` record plus a clean rerun;
  the console of the stopped attempts also reads complete. Counts below use the
  clean rerun of each.
- Rows 23-28 and the final tally were produced by `TREE_ROWS` resumes into the
  same record directory. The tally script reads all records, so the first
  attempts of T5 and U1 appear as extra verdict lines.

## Verdict lines and TALLY (as `board.sh pi4 tally` printed them)

```
== per run (the verdict line, recomputed from each run's files)
board.sh pi4 AL rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 D rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=296s
board.sh pi4 E rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 F1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=296s
board.sh pi4 FB1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=296s
board.sh pi4 FB2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 FF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_1 resets=0 record=absent back_after=297s
board.sh pi4 H rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_3 resets=0 record=absent back_after=309s
board.sh pi4 LA rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 MISC rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=303s
board.sh pi4 NS rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 RT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=307s
board.sh pi4 S1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=300s
board.sh pi4 S2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=293s
board.sh pi4 S3 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 S4 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=299s
board.sh pi4 S5f rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=307s
board.sh pi4 S5v rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=305s
board.sh pi4 SMALL rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=314s
board.sh pi4 T1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 T2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=317s
board.sh pi4 T3 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 T4 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 T5 rc=5 verdict=not_back boot=tryboot row boot_verdict=not_back result=none resets=0 record=absent back_after=?s
board.sh pi4 T5 rc=5 verdict=not_back boot=tryboot row boot_verdict=not_back result=none resets=absent record=absent back_after=?s
board.sh pi4 T5 rc=5 verdict=not_back boot=tryboot row boot_verdict=not_back result=none resets=absent record=absent back_after=?s
board.sh pi4 T5 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 TF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=303s
board.sh pi4 U1 rc=5 verdict=not_back boot=tryboot row boot_verdict=not_back result=none resets=0 record=absent back_after=?s
board.sh pi4 U1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=296s
board.sh pi4 VA rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 DR rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 XT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
== per record (deqp-uart-tally.py)
records/pi4/deqp-a25712f2/20261012-1218-AL-b5b20d38/console.txt: deqp: done: 57 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 57 cases begun, 57 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1223-D-d3f94171/console.txt: deqp: done: 305 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 305 cases begun, 305 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1228-E-371ac8de/console.txt: deqp: done: 453 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 453 cases begun, 453 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1233-F1-9d63c812/console.txt: deqp: done: 1923 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1923 cases begun, 1923 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1238-FB1-8ddf032e/console.txt: deqp: done: 392 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 392 cases begun, 392 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1243-FB2-63f8c1a6/console.txt: deqp: done: 319 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 319 cases begun, 319 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1248-FF-73384feb/console.txt: deqp: done: 4 passed, 0 failed, 0 not supported, 1 warnings, complete=1 | deqp: summary: 5 cases begun, 5 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1253-H-ee4fa25d/console.txt: deqp: done: 651 passed, 0 failed, 0 not supported, 3 warnings, complete=1 | deqp: summary: 654 cases begun, 654 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1259-LA-f41ac6e2/console.txt: deqp: done: 19 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 19 cases begun, 19 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1304-MISC-94b1bfd5/console.txt: deqp: done: 58 passed, 0 failed, 16 not supported, 0 warnings, complete=1 | deqp: summary: 74 cases begun, 74 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1309-NS-3fd581b4/console.txt: deqp: done: 633 passed, 0 failed, 9 not supported, 0 warnings, complete=1 | deqp: summary: 642 cases begun, 642 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1314-RT-339e90e6/console.txt: deqp: done: 200 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 200 cases begun, 200 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1320-S1-17474351/console.txt: deqp: done: 2949 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 2949 cases begun, 2949 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1325-S2-124b8fc1/console.txt: deqp: done: 1910 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1910 cases begun, 1910 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1330-S3-2ff2e15e/console.txt: deqp: done: 2188 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 2188 cases begun, 2188 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1335-S4-c6e7edff/console.txt: deqp: done: 2027 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 2027 cases begun, 2027 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1340-S5f-b1e265ca/console.txt: deqp: done: 100 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 100 cases begun, 100 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1346-S5v-a06ceaac/console.txt: deqp: done: 100 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 100 cases begun, 100 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1351-SMALL-23264272/console.txt: deqp: done: 227 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 227 cases begun, 227 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1356-T1-61393b06/console.txt: deqp: done: 224 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 224 cases begun, 224 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1401-T2-5423d69b/console.txt: deqp: done: 248 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 248 cases begun, 248 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1407-T3-821520ca/console.txt: deqp: done: 298 passed, 6 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 304 cases begun, 304 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1412-T4-3f108f37/console.txt: deqp: done: 66 passed, 3 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 69 cases begun, 69 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1417-T5-dc11339c/console.txt: deqp: done: 7 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 7 cases begun, 7 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1425-T5-dc11339c/console.txt: deqp: done: 7 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 7 cases begun, 7 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1430-TF-c98923cb/console.txt: deqp: done: 20 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 20 cases begun, 20 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1435-U1-203e4229/console.txt: deqp: done: 1267 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1267 cases begun, 1267 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1441-U1-203e4229/console.txt: deqp: done: 1267 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 1267 cases begun, 1267 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1446-VA-b0524d48/console.txt: deqp: done: 571 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 571 cases begun, 571 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1451-DR-22e73ab2/console.txt: deqp: done: 101 passed, 0 failed, 0 not supported, 0 warnings, complete=1 | deqp: summary: 101 cases begun, 101 with a result, 0 without
records/pi4/deqp-a25712f2/20261012-1457-XT-1bc0002e/console.txt: deqp: done: 3 passed, 0 failed, 9 not supported, 0 warnings, complete=1 | deqp: summary: 12 cases begun, 12 with a result, 0 without
== recoveries per record (the OSAL's diag line; GPU reset lines)
20261012-1218-AL-b5b20d38    recoveries: 0; GPU reset lines: 0
20261012-1223-D-d3f94171     recoveries: 0; GPU reset lines: 0
20261012-1228-E-371ac8de     recoveries: 0; GPU reset lines: 0
20261012-1233-F1-9d63c812    recoveries: 0; GPU reset lines: 0
20261012-1238-FB1-8ddf032e   recoveries: 0; GPU reset lines: 0
20261012-1243-FB2-63f8c1a6   recoveries: 0; GPU reset lines: 0
20261012-1248-FF-73384feb    recoveries: 0; GPU reset lines: 0
20261012-1253-H-ee4fa25d     recoveries: 0; GPU reset lines: 0
20261012-1259-LA-f41ac6e2    recoveries: 0; GPU reset lines: 0
20261012-1304-MISC-94b1bfd5  recoveries: 0; GPU reset lines: 0
20261012-1309-NS-3fd581b4    recoveries: 0; GPU reset lines: 0
20261012-1314-RT-339e90e6    recoveries: 0; GPU reset lines: 0
20261012-1320-S1-17474351    recoveries: 0; GPU reset lines: 0
20261012-1325-S2-124b8fc1    recoveries: 0; GPU reset lines: 0
20261012-1330-S3-2ff2e15e    recoveries: 0; GPU reset lines: 0
20261012-1335-S4-c6e7edff    recoveries: 0; GPU reset lines: 0
20261012-1340-S5f-b1e265ca   recoveries: 0; GPU reset lines: 0
20261012-1346-S5v-a06ceaac   recoveries: 0; GPU reset lines: 0
20261012-1351-SMALL-23264272 recoveries: 0; GPU reset lines: 0
20261012-1356-T1-61393b06    recoveries: 0; GPU reset lines: 0
20261012-1401-T2-5423d69b    recoveries: 0; GPU reset lines: 0
20261012-1407-T3-821520ca    recoveries: 0; GPU reset lines: 0
20261012-1412-T4-3f108f37    recoveries: 0; GPU reset lines: 0
20261012-1417-T5-dc11339c    recoveries: 0; GPU reset lines: 0
20261012-1425-T5-dc11339c    recoveries: 0; GPU reset lines: 0
20261012-1430-TF-c98923cb    recoveries: 0; GPU reset lines: 0
20261012-1435-U1-203e4229    recoveries: 0; GPU reset lines: 0
20261012-1441-U1-203e4229    recoveries: 0; GPU reset lines: 0
20261012-1446-VA-b0524d48    recoveries: 0; GPU reset lines: 0
20261012-1451-DR-22e73ab2    recoveries: 0; GPU reset lines: 0
20261012-1457-XT-1bc0002e    recoveries: 0; GPU reset lines: 0
== mustpass: 17165 cases in these records' case filters (0 outside them, not run, left out); 17165 with a result (17367 case lines seen)
group                           pass   fail   n.s.     QW  other   none
attribute_location                57      0      0      0      0      0
buffer                            49      0      0      0      0      0
clip_control                       0      0      8      0      0      0
clipping                         602      0      0      0      0      0
color_clear                       19      0      0      0      0      0
debug_marker                       3      0      0      0      0      0
default_vertex_attrib             40      0      0      0      0      0
depth_range                       28      0      0      0      0      0
depth_stencil_clear               11      0      0      0      0      0
dither                            20      0      0      0      0      0
draw                             101      0      0      0      0      0
fbo                              709      0      0      0      0      0
flush_finish                       4      0      0      0      1      0
fragment_ops                    1923      0      0      0      0      0
implementation_limits             16      0      0      0      0      0
info                               6      0      0      0      0      0
lifetime                          33      0      0      0      0      0
light_amount                      19      0      0      0      0      0
multisample                        0      0     16      0      0      0
multisampled_render_to_texture       0      0      1      0      0      0
negative_api                     237      0      9      0      0      0
polygon_offset                    10      0      0      0      0      0
prerequisite                       3      0      0      0      0      0
rasterization                     49      0      0      3      0      0
read_pixels                        8      0      0      0      0      0
shader_api                        38      0      0      0      0      0
shaders                        10101      0      0      0      0      0
state_query                      396      0      0      0      0      0
texture                          798      9      0      0      0      0
uniform_api                     1267      0      0      0      0      0
vertex_arrays                    571      0      0      0      0      0
TOTAL                          17118      9     34      3      1      0
== not Pass, by name
NotSupported dEQP-GLES2.functional.clip_control.depth_mode_one_to_one
NotSupported dEQP-GLES2.functional.clip_control.depth_mode_zero_to_one
NotSupported dEQP-GLES2.functional.clip_control.errors
NotSupported dEQP-GLES2.functional.clip_control.face_culling
NotSupported dEQP-GLES2.functional.clip_control.initial
NotSupported dEQP-GLES2.functional.clip_control.modify_get
NotSupported dEQP-GLES2.functional.clip_control.origin
NotSupported dEQP-GLES2.functional.clip_control.viewport_bounds
CompatibilityWarning dEQP-GLES2.functional.flush_finish.flush
NotSupported dEQP-GLES2.functional.multisample.common_edge_big_quad
NotSupported dEQP-GLES2.functional.multisample.common_edge_small_quads
NotSupported dEQP-GLES2.functional.multisample.common_edge_viewport_quad
NotSupported dEQP-GLES2.functional.multisample.constancy_alpha_to_coverage
NotSupported dEQP-GLES2.functional.multisample.constancy_both
NotSupported dEQP-GLES2.functional.multisample.constancy_both_inverted
NotSupported dEQP-GLES2.functional.multisample.constancy_sample_coverage
NotSupported dEQP-GLES2.functional.multisample.constancy_sample_coverage_inverted
NotSupported dEQP-GLES2.functional.multisample.depth
NotSupported dEQP-GLES2.functional.multisample.num_samples_line
NotSupported dEQP-GLES2.functional.multisample.num_samples_polygon
NotSupported dEQP-GLES2.functional.multisample.proportionality_alpha_to_coverage
NotSupported dEQP-GLES2.functional.multisample.proportionality_sample_coverage
NotSupported dEQP-GLES2.functional.multisample.proportionality_sample_coverage_inverted
NotSupported dEQP-GLES2.functional.multisample.sample_coverage_invert
NotSupported dEQP-GLES2.functional.multisample.stencil
NotSupported dEQP-GLES2.functional.multisampled_render_to_texture.readpixels
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_invalid_size
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_invalid_target
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_level_max_cube
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_level_max_tex2d
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_neg_level_cube
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_neg_level_tex2d
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_neg_offset
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_neg_wdt_hgt
NotSupported dEQP-GLES2.functional.negative_api.texture.compressedtexsubimage2d_offset_allowed
QualityWarning dEQP-GLES2.functional.rasterization.interpolation.basic.lines
QualityWarning dEQP-GLES2.functional.rasterization.interpolation.projected.lines
QualityWarning dEQP-GLES2.functional.rasterization.interpolation.projected.lines_wide
Fail dEQP-GLES2.functional.texture.mipmap.cube.basic.linear_linear
Fail dEQP-GLES2.functional.texture.mipmap.cube.basic.linear_nearest
Fail dEQP-GLES2.functional.texture.mipmap.cube.bias.linear_linear
Fail dEQP-GLES2.functional.texture.mipmap.cube.bias.linear_nearest
Fail dEQP-GLES2.functional.texture.mipmap.cube.projected.linear_linear
Fail dEQP-GLES2.functional.texture.mipmap.cube.projected.linear_nearest
Fail dEQP-GLES2.functional.texture.size.cube.15x15_l8
Fail dEQP-GLES2.functional.texture.size.cube.15x15_rgb888
Fail dEQP-GLES2.functional.texture.size.cube.15x15_rgba8888
```

## Per row (the clean run of each image; all names the image ran)

"Warn" is dEQP's warning count (QualityWarning / CompatibilityWarning; dEQP
counts them as pass). crash: none in any row.

| Label | Pass | Fail | NotSupported | Warn | crash |
|---|---:|---:|---:|---:|---:|
| AL | 57 | 0 | 0 | 0 | 0 |
| D | 305 | 0 | 0 | 0 | 0 |
| E | 453 | 0 | 0 | 0 | 0 |
| F1 | 1923 | 0 | 0 | 0 | 0 |
| FB1 | 392 | 0 | 0 | 0 | 0 |
| FB2 | 319 | 0 | 0 | 0 | 0 |
| FF | 4 | 0 | 0 | 1 | 0 |
| H | 651 | 0 | 0 | 3 | 0 |
| LA | 19 | 0 | 0 | 0 | 0 |
| MISC | 58 | 0 | 16 | 0 | 0 |
| NS | 633 | 0 | 9 | 0 | 0 |
| RT | 200 | 0 | 0 | 0 | 0 |
| S1 | 2949 | 0 | 0 | 0 | 0 |
| S2 | 1910 | 0 | 0 | 0 | 0 |
| S3 | 2188 | 0 | 0 | 0 | 0 |
| S4 | 2027 | 0 | 0 | 0 | 0 |
| S5f | 100 | 0 | 0 | 0 | 0 |
| S5v | 100 | 0 | 0 | 0 | 0 |
| SMALL | 227 | 0 | 0 | 0 | 0 |
| T1 | 224 | 0 | 0 | 0 | 0 |
| T2 | 248 | 0 | 0 | 0 | 0 |
| T3 | 298 | 6 | 0 | 0 | 0 |
| T4 | 66 | 3 | 0 | 0 | 0 |
| T5 | 7 | 0 | 0 | 0 | 0 |
| TF | 20 | 0 | 0 | 0 | 0 |
| U1 | 1267 | 0 | 0 | 0 | 0 |
| VA | 571 | 0 | 0 | 0 | 0 |
| DR | 101 | 0 | 0 | 0 | 0 |
| XT | 3 | 0 | 9 | 0 | 0 |

## The 52 expected changes

- 27 `fbo.completeness.attachment_combinations.*_tex` (row 4, FB1): **all 27 now pass**
  (FB1: 392 pass, 0 NotSupported; baseline 365 / 27).
- 25 cube cases of issue #12 (**19 now pass, 6 still fail**):
  - 18 in T1 (row 19, `texture.vertex.cube.*`): **all 18 pass** (T1 224 pass, 0 fail; baseline 206 / 18).
  - 1 in TF (row 24, `shaders.texture_functions.vertex.texturecubelod`): **passes** (TF 20 pass, 0 fail; baseline 19 / 1).
  - 6 in T3 (row 21): **still Fail**, T3 298 pass / 6 fail, unchanged:
    `texture.mipmap.cube.basic.linear_linear`, `.basic.linear_nearest`,
    `.bias.linear_linear`, `.bias.linear_nearest`, `.projected.linear_linear`,
    `.projected.linear_nearest`. Per `docs/v3d-noseamless-cube.md` ("Reading a
    failure"), only these six failing means the fragment stage's implicit level
    (FDX/FDY fallback) is not written.

## Every other change against the baseline (fe5e8e73)

- **3 new Fail in T4 (row 22)**, baseline 69 pass / 0 fail, now 66 / 3:
  `texture.size.cube.15x15_l8`, `texture.size.cube.15x15_rgb888`,
  `texture.size.cube.15x15_rgba8888`. Cube textures, not among the 52.
- D (row 1): 305 pass / 0 NotSupported (baseline 304 / 1): the predicted
  `shaders.fragdata.invalid_assign_to_1` now passes.
- FF (row 6): 4 pass + 1 warning, the tally lists it as
  `CompatibilityWarning dEQP-GLES2.functional.flush_finish.flush`
  (baseline: 5 pass). dEQP counts a warning as a pass.
- All other rows equal the baseline, including the 34 NotSupported
  (multisample 16, compressedtexsubimage2d 9, clip_control 8, and
  multisampled_render_to_texture.readpixels 1) and the 3 QualityWarning
  (`rasterization.interpolation.basic.lines`, `.projected.lines`,
  `.projected.lines_wide`).

## Failing cases by name (9)

```
dEQP-GLES2.functional.texture.mipmap.cube.basic.linear_linear
dEQP-GLES2.functional.texture.mipmap.cube.basic.linear_nearest
dEQP-GLES2.functional.texture.mipmap.cube.bias.linear_linear
dEQP-GLES2.functional.texture.mipmap.cube.bias.linear_nearest
dEQP-GLES2.functional.texture.mipmap.cube.projected.linear_linear
dEQP-GLES2.functional.texture.mipmap.cube.projected.linear_nearest
dEQP-GLES2.functional.texture.size.cube.15x15_l8
dEQP-GLES2.functional.texture.size.cube.15x15_rgb888
dEQP-GLES2.functional.texture.size.cube.15x15_rgba8888
```

## For the next run

Do not run the tree as one long background job that the harness can stop: a
stop during a row leaves the board off the network for a few minutes and
leaves a staged image in /boot/firmware. Resume with `TREE_ROWS=<rows>`.
