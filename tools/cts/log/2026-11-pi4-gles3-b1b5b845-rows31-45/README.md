<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 32-46 (S11 S12 S13 S14 S15 S16 S17 S18 S19 S20 S21 S22 S23 S24 S25) at b1b5b845

commit b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-rows31-45`: rows 32-46 of `tools/cts/pi4-gles3-tree.txt` (15 images: S11 S12 S13 S14 S15 S16 S17 S18 S19 S20 S21 S22 S23 S24 S25).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 32 | S11 | 756 | 0 | 0 | 0 | 39.2 s | 0 | none | DONE, all passed |
| 33 | S12 | 1216 | 0 | 0 | 0 | 55.1 s | 0 | none | DONE, all passed |
| 34 | S13 | 957 | 0 | 0 | 0 | 42.3 s | 0 | none | DONE, all passed |
| 35 | S14 | 594 | 0 | 0 | 0 | 25.5 s | 0 | none | DONE, all passed |
| 36 | S15 | 972 | 0 | 0 | 0 | 40.8 s | 0 | none | DONE, all passed |
| 37 | S16 | 719 | 1 | 0 | 0 | 31.7 s | 0 | none | DONE |
| 38 | S17 | 716 | 0 | 0 | 0 | 34.3 s | 0 | none | DONE, all passed |
| 39 | S18 | 864 | 0 | 0 | 0 | 43.3 s | 0 | none | DONE, all passed |
| 40 | S19 | 716 | 0 | 0 | 0 | 34.8 s | 0 | none | DONE, all passed |
| 41 | S20 | 666 | 0 | 0 | 0 | 34.2 s | 0 | none | DONE, all passed |
| 42 | S21 | 680 | 0 | 0 | 0 | 32.4 s | 0 | none | DONE, all passed |
| 43 | S22 | 768 | 0 | 0 | 0 | 38.1 s | 0 | none | DONE, all passed |
| 44 | S23 | 866 | 0 | 0 | 0 | 39.0 s | 0 | none | DONE, all passed |
| 45 | S24 | 1126 | 0 | 0 | 0 | 54.9 s | 0 | none | DONE, all passed |
| 46 | S25 | 689 | 0 | 0 | 0 | 26.2 s | 0 | none | DONE, all passed |

Run 2026-11-15 on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-s5 of plan #320 (issue #1261): session 5a, rows 31-45 (S11..S25) of tools/cts/pi4-gles3-tree.txt; session 3 had left no record of them. 12,306 mustpass cases in fifteen images, all with a result: 12,305 pass, 1 fail. Every image ended complete=1; 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets (pi4-console-scan.py). The only Fail is S16 shaders.metamorphic.synthetic.variant_3 (Program(Select(OutOfScratch)) at shader build). The longest runs are S12 55.1 s and S24 54.9 s of the 210 s record deadline: no image overran, nothing was re-split. Linux was back 296-323 s after each reboot (cycles.txt). S11 back_after is approximate (the first ssh answer after the reboot was not timed to the second; about 305 s). After S22 the first clean-up ssh was refused while the board was still coming up; the clean-up was repeated at once and read back (staged image removed, tryboot.txt one newline); the run itself was complete.
Method: gles3 stage 1 (docs/es3/board-recipe-pi4.md section 2: VK-GL-CTS 93bca01, Zephyr SDK 1.0.1, 4 cores) and the 30 images of this session built with DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh from a checkout at b1b5b845 (every .built line rc=0 build=b1b5b845). The board runner could not capture the console (the leased serial console holds the UART bridge), so the steps of tools/board.sh pi4 were done by hand, one image at a time: staging to /boot/firmware and tryboot.txt read back (sha256, text), sudo reboot "0 tryboot", the console through the leased serial console (serial_expect on the end line, serial_read into spill files, converted by a script, never copied by hand), Linux back (a new boot_id), staged image removed and tryboot.txt put back, read back. config.txt untouched.

(The table's "row" column counts from 1; the session's `TREE_ROWS` were 31-45, counted from 0 in the 94-image table.)
