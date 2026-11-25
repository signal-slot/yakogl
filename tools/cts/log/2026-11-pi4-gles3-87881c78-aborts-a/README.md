<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 S03a S07 T17a TF1a TF1b TF2a TF2b TF2c at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-aborts-a`: the eight images whose dEQP run aborted on a refused draw at `b1b5b845` (`../2026-11-pi4-gles3-b1b5b845-aborts/`), rebuilt from `87881c78` (on origin/main) with the case lists `tools/cts/pi4-gles3-tree.txt` defines now (no re-split), and run on the Raspberry Pi 4 (V3D 4.2, tryboot). Item board-aborts-a of plan #399, issue #1261. `docs/notes/v3d-refusals.md` says origin/main removed these aborts: `6d2e7d0d` (S03a, S07), `455c3bb5` (T17a), `6581f22b` and `b7c1484e` (TF1a-TF2c). This is that check on the board.

| image | cases | Pass | Fail | Refused | no result | verdict |
|---|---|---|---|---|---|---|
| S03a | 1141 | 276 | 0 | 0 | 865 | INCOMPLETE (the photo deadline came first, 276 cases in 209 s; no abort, no refused draw) |
| S07 | 4 | 4 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| T17a | 71 | 71 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| TF1a | 184 | 184 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| TF1b | 189 | 189 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| TF2a | 177 | 177 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| TF2b | 189 | 189 | 0 | 0 | 0 | DONE, all passed, complete=1 |
| TF2c | 60 | 60 | 0 | 0 | 0 | DONE, all passed, complete=1 |

Seven of the eight aborts are gone: S07, T17a, TF1a, TF1b, TF2a, TF2b and TF2c ran to `complete=1` with every case passing and no refused draw. S03a no longer aborts on its first case (the tile-memory refusal of `precision.abs.highp_fragment.vec2`): it passes 276 cases in a row, but each shader precision case takes 0.1-4.6 s, so the 210 s record deadline ends the run first; the remaining 865 cases need a split for time (the tree re-split item), not a V3D fix.

0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run; the PM watchdog ended each image (`reset at ~250 s`) and Linux answered again after each.

Method. The stage 1 builds are those of `docs/es3/board-recipe-pi4.md` section 2 (dEQP at 93bca01, gles2 and gles3). The eight images were built with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> S03a S07 T17a TF1a TF1b TF2a TF2b TF2c` from a worktree of `87881c78` outside the repository (each `rc=0 build=87881c78`; every console prints `v3d_deqp: build 87881c78`). As in the other Pi 4 sessions the leased serial console holds the one-client UART bridge, so `tools/board.sh pi4`'s steps were done by hand in its order, one image at a time: the image installed as `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines, both read back (the image's sha256 equal to the build's), `sudo reboot "0 tryboot"`, the console read up to the end line, Linux back, the image removed and the previous `tryboot.txt` put back, read back equal.

The console files are condensed: they keep the boot lines, the build stamp, the filter, the last case line, dEQP's totals, the `deqp: done` / `deqp: summary` lines, the image's end block and its case-order lines; the per-case lines (every one `Pass (Pass)`) are not kept, and there are no `.tally.txt` files. The counts above are the `deqp: done` and end lines of each console.
