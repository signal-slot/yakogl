<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 28-109 (S03a3a S03a3b S03a3c S03a4a S03a4b S03a5a S03a5b S03a6a S03a6b S03a6c S03a6d S03a7a S03a7b T14a1 T14aa T14aa1) at 625b9f5c

commit: 625b9f5c8fc1435fed9e4bf53f67619b9149e6b5

Session `2026-11-pi4-gles3-625b9f5c-aborts2`: rows 28-109 of `tools/cts/pi4-gles3-tree.txt` (16 images: S03a3a S03a3b S03a3c S03a4a S03a4b S03a5a S03a5b S03a6a S03a6b S03a6c S03a6d S03a7a S03a7b T14a1 T14aa T14aa1).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 28 | S03a3a | 83 | 0 | 0 | 0 | 121.5 s | 0 | none | DONE, all passed |
| 29 | S03a3b | 81 | 0 | 0 | 0 | 48.2 s | 0 | none | DONE, all passed |
| 30 | S03a3c | 26 | 0 | 0 | 0 | 57.1 s | 0 | none | DONE, all passed |
| 31 | S03a4a | 85 | 0 | 0 | 0 | 58.4 s | 0 | none | DONE, all passed |
| 32 | S03a4b | 87 | 18 | 0 | 0 | 35.4 s | 0 | none | DONE |
| 33 | S03a5a | 68 | 0 | 0 | 0 | 53.3 s | 0 | none | DONE, all passed |
| 34 | S03a5b | 104 | 18 | 0 | 0 | 81.4 s | 0 | none | DONE |
| 35 | S03a6a | 45 | 0 | 0 | 0 | 56.4 s | 0 | none | DONE, all passed |
| 36 | S03a6b | 12 | 0 | 0 | 0 | 40.6 s | 0 | none | DONE, all passed |
| 37 | S03a6c | 12 | 0 | 0 | 0 | 41.3 s | 0 | none | DONE, all passed |
| 38 | S03a6d | 121 | 0 | 0 | 0 | 58.6 s | 0 | none | DONE, all passed |
| 39 | S03a7a | 88 | 0 | 0 | 0 | 47.0 s | 0 | none | DONE, all passed |
| 40 | S03a7b | 84 | 18 | 0 | 0 | 64.5 s | 0 | none | DONE |
| 107 | T14a1 | 0 | 1 | 0 | 0 | 1.0 s | 0 | none | DONE |
| 108 | T14aa | 15 | 0 | 0 | 0 | 23.1 s | 0 | none | DONE |
| 109 | T14aa1 | 1 | 0 | 0 | 0 | 1.8 s | 0 | none | DONE, all passed |

Run 2026-11-27 from 05:00 to 06:26 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item board1 of plan #398 (issue #2, formerly #1261), one cycle per image, no retry. The images are built from 625b9f5c, origin/main's tip as this task's snapshot holds it (git fetch origin failed on the worker, no access), with that commit's own tools/cts/pi4-gles3-tree.txt (151 images): the tree is unchanged by this session.
Which labels. At 87881c78 the refusals of the b1b5b845 aborts round were gone (../2026-11-pi4-gles3-87881c78-aborts-a/, -tex1/, -tex2/): S07, T17a (whose 71 cases hold today's T17a1, T17aa, T17aaa, T17aaaa), TF1a, TF1b, TF2a, TF2b and TF2c (whose cases hold today's TF1a..TF2caa labels) and T05c, T05ca-T05cc ran to the end with every case a result, and S03a passed its first 276 cases and then met its 210 s record deadline (INCOMPLETE, 937 mustpass cases from precision.cosh.lowp_vertex.vec3 on without a result). This session runs the labels of plan #398 that still had cases with no result at any commit after those fixes: S03a3a to S03a7b (S03a3a's first 13 cases overlap the last results of 87881c78's S03a) and T14a1, T14aa, T14aa1, which no session has run since b1b5b845 (they need texture-2048.conf). S03a1, S03a2a and S03a2b are not run (their cases have results in 87881c78's S03a); nor are the TF, T17 and T05c labels.
Result. No image aborted. All sixteen ended complete=1 with 0 cases without a result, and every verdict line is rc=0 verdict=done ... resets=0 (cycles.txt). Of the 967 mustpass cases in the sixteen filters, 912 pass, 55 fail, 0 NotSupported, 0 other and 0 never began (table.txt). The 54 shaders Fails are the mat4, mat4x2 and mat4x3 cases of builtin_functions.precision.matrixcompmult, outerproduct and transpose, at every precision and stage, each 'Shader compilation failed at glsBuiltinPrecisionTests.cpp:4967' (18 each in S03a4b, S03a5b and S03a7b). The tile-memory refusal (a target larger than 2048x2048) that stopped every precision image at b1b5b845 did not happen once.
The T14 refusal moved. texture.units.8_units.only_cube.2 (T14a1), refused at b1b5b845 for lack of GPU memory, now runs with texture-2048.conf (112 MiB carve-out, 32 MiB of tiled copies): a Fail, 'Image comparison failed' (its console's cube dump of unit 2 reads NaN and inf face values). T14aa's 15 mustpass cases pass (its 6 Fails, all_units.mixed.4/5/7 and only_2d/only_2d_array/only_3d .7, are outside the mustpass) and T14aa1's all_units.only_cube.0 passes. Carve-out high-water 106295 KiB of 114172 (T14aa); 0 allocations refused for lack of GPU memory in any image. Refusing cases: none in this session.
With this session, 966 of the 1,978 mustpass cases that never began at b1b5b845 have a result at 625b9f5c (950 in S03a3a-S03a7b, 16 in T14aa and T14aa1). The other 1,012 have one at 87881c78: 190 of S03a2a and S03a2b (and 13 more of S03a3a's) in that commit's S03a, 70 of T17a, 752 of transform_feedback. So every one of the 1,978 now has a result, at one of the two commits.
Method. The gles3 stage 1 of recipe section 2 was built on this worker from a depth-1 fetch of VK-GL-CTS 93bca01 (cts-build-zephyr: libraries in <WORK>/build, 453 ninja steps), then DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> <the sixteen labels> from a clean worktree of 625b9f5c: sixteen rc=0 build=625b9f5c lines with the check-deqp-images conformance line, RAM 166,476 KB (63.51 %) for the S03a images and 232,044 KB (88.52 %) for the three texture-2048.conf images; tools/cts/pi4-gles3-preflight.sh --commit 625b9f5c... --rows 27-39,106-108: all ok. The leased serial console holds the Pi 4's one-client UART bridge, so tools/board.sh pi4's steps were done by hand in its order, one image at a time, T14a1 second after S03a3a: prechecks over ssh (no staged image, no tryboot.txt, no tryboot.txt.yk-orig, no config-linux.txt, no GL client running, twice the image free on /boot/firmware), the image written as /boot/firmware/yakogl-<LABEL>.bin and tryboot.txt with pi4_config's lines, both read back (sha256, text), sudo reboot "0 tryboot", Linux back with a new boot id, the image and tryboot.txt removed and read back (the card had no tryboot.txt before the session, none after it; config.txt untouched). Each console file holds the bytes from the firmware's first line to the image's end line, CRs and NULs dropped, converted by a script from the serial reads; the byte count with the CRs restored was checked equal to the serial range. Each run's verdict line is the one tools/board.sh pi4 tally recomputes from that run's files (a meta.txt with name, boot=tryboot, back=1 and the reboot and back times, and its console), the redacted board record kept under the gitignored records/pi4/. Linux was back 297-308 s after each reboot. tools/cts/console-redact.py ran over the consoles.
