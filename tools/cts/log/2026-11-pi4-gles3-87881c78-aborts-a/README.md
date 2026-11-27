<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 at 87881c78: the eight images that aborted at b1b5b845 (S03a S07 T17a TF1a TF1b TF2a TF2b TF2c)

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-aborts-a`, item board-aborts-a of plan #399 (issue #1261). At `b1b5b845` these eight images aborted (`../2026-11-pi4-gles3-b1b5b845-aborts/`); `docs/notes/v3d-refusals.md` says `6d2e7d0d` fixes S03a and S07, `455c3bb5` T17a and `6581f22b` plus `b7c1484e` TF1a to TF2c. Run 2026-11-27 on the Raspberry Pi 4 (V3D 4.2, tryboot), one cycle per image, images built from a worktree of `87881c78` with that commit's own `tools/cts/pi4-gles3-tree.txt` (case counts S03a 1141, S07 4, T17a 71, TF1a 184, TF1b 189, TF2a 177, TF2b 189, TF2c 60).

| image | cases | Pass | Fail | other | complete | run | recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| S07 | 4 | 4 | 0 | 0 | 1 | 6.0 s | 0 | none | DONE, all passed |
| T17a | 71 | 71 | 0 | 0 | 1 | 6.2 s | 0 | none | DONE, all passed |
| TF1a | 184 | 184 | 0 | 0 | 1 | 12.5 s | 0 | none | DONE, all passed |
| TF1b | 189 | 189 | 0 | 0 | 1 | 11.6 s | 0 | none | DONE, all passed |
| TF2a | 177 | 177 | 0 | 0 | 1 | 10.9 s | 0 | none | DONE, all passed |
| TF2b | 189 | 189 | 0 | 0 | 1 | 11.1 s | 0 | none | DONE, all passed |
| TF2c | 60 | 60 | 0 | 0 | 1 | 4.3 s | 0 | none | DONE, all passed |
| S03a | 1141 | 276 | 0 | 1 in progress | - | 209.1 s | 0 | none | INCOMPLETE (the photo deadline at 210 s came first) |

Result. Seven of the eight images no longer abort: each ended `DONE, all passed` with `complete=1` and `N cases begun, N with a result, 0 without`, 0 failed, 0 not supported, 0 refused. So `6d2e7d0d` (S07), `455c3bb5` (T17a) and `6581f22b` plus `b7c1484e` (TF1a, TF1b, TF2a, TF2b, TF2c) remove those aborts on the board. S03a no longer aborts on a refused draw either (its first case, precision.abs.highp_fragment.vec2, passed), but it did not finish: dEQP was still running at the 210 s deadline, 276 cases Pass, case #276 `precision.cosh.lowp_vertex.vec3` in progress, 865 of 1141 cases not begun (1141 - 276). No failure, refusal, GPU recovery, job timeout, hung job or MMU fault in any image. S03a needs a split for time (the cases run at about 0.76 s each, so about 270 fit in the window).

Method. The dEQP stage 1 trees (gles2 `deqp-stage1-rpi_4b` through `deps-env.sh --build`, gles3 `deqp-stage1-rpi_4b-gles3` through `cts-build-zephyr.sh` with `MODULE=gles3`, both and VK-GL-CTS at 93bca01 built for this session: the cache had none) came from the `87881c78` worktree, then `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> S03a S07 T17a TF1a TF1b TF2a TF2b TF2c` gave eight `rc=0 build=87881c78` images. On the board `tools/board.sh pi4`'s steps were done by hand in its order, one image at a time (the leased serial console is a one-client bridge): check that no `yakogl-<LABEL>.bin` and no `tryboot.txt` is there, copy the image to `/boot/firmware/yakogl-<LABEL>.bin`, write `tryboot.txt` (`[all]`, `kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no hdmi timings), read both back (sha256 equal), `sudo reboot "0 tryboot"`, read the console up to the image's end line, wait for Linux, remove the image and `tryboot.txt` (the board had no `tryboot.txt` before; none is left). `cycles.txt` has the sha256 and the 298-305 s each return took.

What the console files hold. Each line a file keeps is exactly what the board printed in this run, CRs dropped; nothing was rebuilt from another run or edited. The serial tool returns the console in 16 KiB pieces into the session, not into a file, so the files were written from the pieces read, and not all of each console was written down:
- `S07.console.txt`: the whole console, from the firmware's first line to the image's end line.
- `T17a.console.txt`, `TF1a.console.txt`: the firmware lines, the start of the run (T17a cases #0-#1, TF1a case #0) and the end (T17a from case #66, TF1a from case #180) as printed; the middle is replaced by a `[condensed: ...]` line, and the long `deqp: case filter` line of T17a (and of TF1a) by a `[condensed: ...]` text inside it.
- `TF1b`, `TF2a`, `TF2b`, `TF2c`, `S03a`: the END of the console only (their second line, a `[condensed: ...]` line, says from where): a few last cases, the verdict lines and the image's own record. The firmware lines and the first cases of these five were not saved.
- In every file the `case order` result lines (`    0 PPP...`) were re-typed from the image's own counters: the board printed them, but they are not a copy of the bytes (the run is as long as the counters say, all `P`; for S03a 276 `P`).
Consequently `*.tally.txt` (made with `tools/cts/pi4-session-tally.sh`, run with this commit's tree and mustpass) counts only the result lines kept and says `Lost in capture` or `MISSING: no summary line` where lines were left out: the board's own counters (`deqp: done`, `deqp: summary`, the end line) are the numbers of the table above. `table.txt` was not made for the same reason.

Not done. The per-case lines of the middle of each console are not in the tree. A rerun that writes the serial log to a file would give them.
