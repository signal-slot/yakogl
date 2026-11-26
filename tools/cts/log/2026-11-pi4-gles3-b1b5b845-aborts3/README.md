<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 25-128 (S03a1 S03a2 S03a3 S03a4 S03a5 S03a6 S03a7 TF1a1 TF1b1 TF2a1 TF2b1) at b1b5b845

commit: b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-aborts3`: rows 25-128 of `tools/cts/pi4-gles3-tree.txt` (11 images: S03a1 S03a2 S03a3 S03a4 S03a5 S03a6 S03a7 TF1a1 TF1b1 TF2a1 TF2b1).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 25 | S03a1 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 26 | S03a2 | 0 | 0 | 0 | 1 | 0.3 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 27 | S03a3 | 0 | 0 | 0 | 1 | 0.3 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 28 | S03a4 | 0 | 0 | 0 | 1 | 0.4 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 29 | S03a5 | 0 | 0 | 0 | 1 | 0.4 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 30 | S03a6 | 0 | 0 | 0 | 1 | 0.4 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 31 | S03a7 | 0 | 0 | 0 | 1 | 0.4 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 111 | TF1a1 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 116 | TF1b1 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 123 | TF2a1 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |
| 128 | TF2b1 | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | STOPPED after 1 cases (dEQP ended the session early) |

## Refusing cases

| image | case | cause | issue |
|---|---|---|---|
| S03a1 | dEQP-GLES3.functional.shaders.builtin_functions.precision.abs.highp_fragment.vec2 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a2 | dEQP-GLES3.functional.shaders.builtin_functions.precision.abs.highp_fragment.vec3 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a3 | dEQP-GLES3.functional.shaders.builtin_functions.precision.cosh.highp_fragment.vec2 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a4 | dEQP-GLES3.functional.shaders.builtin_functions.precision.floor.lowp_vertex.vec3 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a5 | dEQP-GLES3.functional.shaders.builtin_functions.precision.max.mediump_fragment.vec4 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a6 | dEQP-GLES3.functional.shaders.builtin_functions.precision.pow.lowp_vertex.vec3 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| S03a7 | dEQP-GLES3.functional.shaders.builtin_functions.precision.smoothstep.lowp_vertex.vec4 | yakogl/v3d: draw refused: a target larger than the tile memory is sized for (2048x2048), or empty | - |
| TF1a1 | dEQP-GLES3.functional.transform_feedback.array.interleaved.lines.highp_mat3 | yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write) | - |
| TF1b1 | dEQP-GLES3.functional.transform_feedback.array.separate.lines.highp_mat3 | yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write) | - |
| TF2a1 | dEQP-GLES3.functional.transform_feedback.basic_types.interleaved.lines.lowp_mat4 | yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write) | - |
| TF2b1 | dEQP-GLES3.functional.transform_feedback.basic_types.separate.lines.highp_mat4 | yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write) | - |

Run 2026-11-26 on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item aborts2-prep of plan #320 (issue #2, formerly #1261): the eleven labels of tools/cts/pi4-gles3-tree.txt (141 images) that had no tally in any b1b5b845 session directory, the cut remainders the earlier rounds added to the tree but did not run. S03a1 is the one-case image of S03a's abort trigger and S03a2-S03a7 hold the 1,140 builtin_functions.precision cases behind it; TF1a1, TF1b1, TF2a1 and TF2b1 are the one-case images of the abort triggers of TF1a, TF1b, TF2a and TF2b. The tree is unchanged by this session.
Result. All eleven images stopped at their first case (v3d_deqp: build b1b5b845 STOPPED EARLY by dEQP (complete=0; later cases never began): 1 cases, P 0 F 0 R 1 N 0 other 0), each on a draw the V3D back end refused (the Refusing cases table above; plan #184's src/v3d). The four transform-feedback images are one-case images, so every case of theirs has a result (ResourceError). S03a1 likewise. S03a2-S03a7 each stopped on the same refusal as S03a1 (a target larger than the tile memory is sized for, 2048x2048), at their first cases abs.highp_fragment.vec3, cosh.highp_fragment.vec2, floor.lowp_vertex.vec3, max.mediump_fragment.vec4, pow.lowp_vertex.vec3 and smoothstep.lowp_vertex.vec4: of their 1,140 mustpass cases 6 have a result and 1,134 never began (table.txt: 1,145 cases in the eleven filters, 11 with a result, all ResourceError; none column: shaders 1,134, transform_feedback 0). As the item asks, the aborted images keep their tallies as they are and the tree was not re-split: this refusal stops the first case of every precision image it reaches (docs/es3/board-recipe-pi4.md section 6: a cut only helps where the cases behind the trigger can pass), so the 1,134 wait for the back end to take the target.
Run times, GPU recoveries and hangs (tools/cts/pi4-console-scan.py over the eleven consoles): 11 image runs, 11 finished, 0 without a finish line; runs of 0.2-0.4 s (153-425 ms), record at 0 s after boot; 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end; its faults 1 per image is the refused case's ResourceError. Every image armed the PM watchdog (reset ~250 s after boot) and Linux was back 298-307 s after each reboot (cycles.txt).
Method. The dependency cache that held the earlier images was gone, so the gles3 stage 1 of recipe section 2 was built again from a worktree of b1b5b845 (VK-GL-CTS 93bca01, Zephyr SDK 1.0.1; cts-build-zephyr: libraries in <WORK>/build, 453 ninja steps, 151 s) and the eleven images with DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh <the tip's pi4-gles3-tree.txt, an untracked copy> <dir> <labels> from that worktree: each rc=0 build=b1b5b845 with the check-deqp-images conformance line, RAM 161,844-161,848 KB (61.74 %); pi4-gles3-preflight.sh read all eleven as built from b1b5b845. The CTS checkout is a depth-1 fetch of 93bca01, with no tag in reach, so the dEQP banner of these consoles reads 'dEQP Core 93bca01861b0e3ef3c387027a9791e6d065f900c (0x93bca018)' where the earlier b1b5b845 consoles read 'vulkan-cts-...-555-g93bca018': the same CTS commit. tools/board.sh pi4 tree was started for the first image (TREE_ROWS=24) and refused before touching the board (rc=3: the UART bridge takes one client and the leased serial console held it), so its steps were done by hand in its order, one image at a time, S03a1 alone first: prechecks over ssh (no staged image, no tryboot.txt backup, no config-linux.txt, no GL client running, twice the image free on /boot/firmware), the image installed as /boot/firmware/yakogl-<LABEL>.bin with pi4_config's tryboot.txt lines, both read back, sudo reboot "0 tryboot", the console read up to the end line, Linux back (a new boot id), the image and tryboot.txt removed and read back (the card had no tryboot.txt before the session, so none is left after it; config.txt untouched). Each console file holds the bytes from the firmware's first line to the image's end line, CRs and NULs dropped; the byte count with the CRs was checked equal to the serial range it came from. tools/cts/console-redact.py found nothing to rewrite.
