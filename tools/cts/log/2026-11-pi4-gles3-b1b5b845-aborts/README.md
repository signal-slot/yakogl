<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 25-106 (S03a T05ca T05cb T05cc T14a T14b T17a T17b TF1a TF1b TF1c TF2a TF2b TF2c) at b1b5b845

commit: b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-aborts`: rows 25-106 of `tools/cts/pi4-gles3-tree.txt` (14 images: S03a T05ca T05cb T05cc T14a T14b T17a T17b TF1a TF1b TF1c TF2a TF2b TF2c).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 25 | S03a | 0 | 0 | 0 | 1 | 0.8 s | 0 | none | incomplete (no complete deqp summary) |
| 79 | T05ca | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 80 | T05cb | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 81 | T05cc | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 92 | T14a | 0 | 0 | 0 | 1 | 0.8 s | 0 | none | incomplete (no complete deqp summary) |
| 93 | T14b | 78 | 96 | 0 | 0 | 24.5 s | 0 | none | DONE |
| 97 | T17a | 0 | 0 | 0 | 1 | 0.5 s | 0 | none | incomplete (no complete deqp summary) |
| 98 | T17b | 36 | 0 | 144 | 0 | 8.3 s | 0 | none | DONE, none failed |
| 100 | TF1a | 3 | 0 | 0 | 1 | 0.8 s | 0 | none | incomplete (no complete deqp summary) |
| 101 | TF1b | 8 | 0 | 0 | 1 | 0.6 s | 0 | none | incomplete (no complete deqp summary) |
| 102 | TF1c | 66 | 0 | 0 | 0 | 3.8 s | 0 | none | DONE, all passed |
| 104 | TF2a | 20 | 0 | 0 | 1 | 1.7 s | 0 | none | incomplete (no complete deqp summary) |
| 105 | TF2b | 11 | 0 | 0 | 1 | 0.8 s | 0 | none | incomplete (no complete deqp summary) |
| 106 | TF2c | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |

Run 2026-11-22 on the Raspberry Pi 4 (V3D 4.2, tryboot), item pi4-aborts of plan #320 (issue #2, formerly #1261): the cases that never began after dEQP aborted S03-S08 (`../2026-11-pi4-gles3-b1b5b845-rows16-30/`), T05c (`../2026-11-pi4-gles3-b1b5b845-rows61-75/`), T14, T17, TF1 and TF2 (`../2026-11-pi4-gles3-b1b5b845-rows76-90/`), re-split per `docs/es3/board-recipe-pi4.md` section 6: the original labels keep the cases that ran, the never-begun rest is in the 14 new labels here (the tree went from 100 to 114 images). The abort triggers of the original images, confirmed in their consoles: S03-S08 the first precision case of each (tile-memory refusal), T05c etc2_eac_rgba8_cube_npot, T14 texture.units.8_units.only_cube.1, T17 wrap.etc2_eac_rgba8.clamp_clamp_linear_npot, TF1 array.interleaved.lines.highp_ivec4, TF2 basic_types.interleaved.lines.highp_mat4.

**Iteration 1 only.** Three images ran to the end: T14b (texture.vertex, `DONE`, 198 cases, P 90 F 108), T17b (`DONE, none failed`, 180 cases, P 36 N 144) and TF1c (`DONE, all passed`, 66). The other eleven stopped again on a refused draw, each one in its case named here: S03a precision.abs.highp_fragment.vec2 (a target larger than the 2048x2048 tile memory), T05ca/T05cb/T05cc their one ETC2 cube case and T17a wrap.etc2_eac_rgba8.clamp_clamp_linear_pot (a texel format the V3D does not sample), T14a units.8_units.only_cube.2 (out of GPU memory, then no empty job holds the draw), TF1a array.interleaved.lines.highp_mat3, TF1b array.separate.lines.highp_mat3, TF2a basic_types.interleaved.lines.lowp_mat4, TF2b basic_types.separate.lines.highp_mat4, TF2c random_full_array_capture.interleaved.lines.1 (the job builder refused the draw). Of the 2451 mustpass cases in these labels 473 have a result and 1978 still never began (`table.txt`, `none` column: shaders 1140 in S03a, texture 86 in T14a and T17a, transform_feedback 752 in TF1a, TF1b, TF2a, TF2b, TF2c); they need a further re-split and run (each abort leaves the remainder of its label unbegun). The refusals are V3D back-end limits, plan #184 (src/v3d).

0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run (`tools/cts/pi4-console-scan.py`; its 'faults 1' per aborted image is the refused case's ResourceError). The longest run was T14b, 24.5 s; the PM watchdog ended each image (`reset at ~250 s`), and Linux was back 296-302 s after each reboot (`cycles.txt`).

Method. The 14 images were built with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh <re-split pi4-gles3-tree.txt> <dir> <labels>` from a worktree of `b1b5b845` and the gles3 stage 1 of recipe section 2 built from it (each `rc=0 build=b1b5b845`). As in the other `b1b5b845` sessions, the leased serial console holds the Pi 4's one-client UART bridge, so `tools/board.sh pi4`'s steps were done by hand in its order, one image (one GL client) at a time: prechecks over ssh, the image installed as `/boot/firmware/yakogl-<LABEL>.bin` with `pi4_config`'s `tryboot.txt` lines, both read back, `sudo reboot "0 tryboot"`, the console read up to the end line, Linux back, the image removed and `tryboot.txt` put back, read back. The consoles of T14a, T14b, T17a and T17b from the first cycle were lost in a serial reconnect, so those four images ran a second cycle; the files here are from that cycle. Each console file holds the bytes from the firmware's first line to the image's end line, CRs and NULs dropped.
