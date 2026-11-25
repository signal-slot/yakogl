<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 0 and 3-15 (A1 D1 FB1 FB2 FB3 FB4 FO1 FO2 FO3 FO4 FO5 FO6 FR1 FR2) at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-fbo`: rows 0 and 3-15 of `tools/cts/pi4-gles3-tree.txt` (14 images: A1 D1 FB1 FB2 FB3 FB4 FO1 FO2 FO3 FO4 FO5 FO6 FR1 FR2), the images the #1261 V3D changes since `b1b5b845` reach (per-render-target colour write masks and BLEND_ENABLES, size words 44-47/46-47, clears at the attachments' intersection, draw-buffer order, 4096 targets). B1 and C1 (rows 1-2) are not in this session: no V3D change reaches them. Item `board-fbo` of plan #399 (issue #1261).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 1 | A1 | 470 | 2 | 0 | 0 | 28.7 s | 0 | none | DONE |
| 4 | D1 | 367 | 0 | 76 | 0 | 18.7 s | 0 | none | DONE, none failed |
| 5 | FB1 | 573 | 0 | 7 | 0 | 17.5 s | 0 | none | DONE |
| 6 | FB2 | 461 | 3 | 146 | 0 | 28.5 s | 0 | none | DONE |
| 7 | FB3 | 243 | 50 | 36 | 0 | 24.1 s | 0 | none | DONE |
| 8 | FB4 | 126 | 317 | 66 | 0 | 29.8 s | 0 | none | DONE |
| 9 | FO1 | 677 | 0 | 0 | 0 | 30.7 s | 0 | none | DONE, all passed |
| 10 | FO2 | 475 | 0 | 0 | 0 | 22.0 s | 0 | none | DONE, all passed |
| 11 | FO3 | 227 | 0 | 0 | 0 | 12.5 s | 0 | none | DONE, all passed |
| 12 | FO4 | 450 | 0 | 0 | 0 | 24.1 s | 0 | none | DONE, all passed |
| 13 | FO5 | 475 | 0 | 0 | 0 | 45.2 s | 0 | none | DONE, all passed |
| 14 | FO6 | 866 | 0 | 0 | 0 | 58.4 s | 0 | none | DONE, all passed |
| 15 | FR1 | 372 | 0 | 48 | 0 | 13.6 s | 0 | none | DONE, none failed |
| 16 | FR2 | 422 | 31 | 67 | 0 | 15.0 s | 0 | none | DONE |

(The table's "row" column counts from 1; the rows of this session are rows 0 and 3-15 counted from 0.)

## Result

Run 2026-11-24 23:33 to 2026-11-25 00:50 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr, tryboot). 7,053 mustpass cases in the fourteen images, all 7,053 with a result: **6,204 pass, 403 fail, 446 NotSupported, 0 other, 0 never began**. All fourteen images ended `complete=1` with `0 without` a result: no image aborted. 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run. The longest run is FO6's 58.4 s of the 210 s record deadline.

Against `b1b5b845` (`docs/es3/pi4-gles3-fails.tsv`, the same fourteen images, 7,053 mustpass cases there too): 155 cases went from not-Pass to Pass, 0 from Pass to not-Pass, 0 changed from one non-Pass status to another:

| image | b1b5b845 Pass/Fail/NotSupported | 87881c78 Pass/Fail/NotSupported | not-Pass to Pass | Pass to not-Pass |
|---|---|---|---|---|
| A1 | 461 / 11 / 0 | 470 / 2 / 0 | 9 | 0 |
| D1 | 367 / 0 / 76 | 367 / 0 / 76 | 0 | 0 |
| FB1 | 559 / 14 / 7 | 573 / 0 / 7 | 14 | 0 |
| FB2 | 445 / 19 / 146 | 461 / 3 / 146 | 16 | 0 |
| FB3 | 157 / 136 / 36 | 243 / 50 / 36 | 86 | 0 |
| FB4 | 108 / 335 / 66 | 126 / 317 / 66 | 18 | 0 |
| FO1-FO6 | all Pass | all Pass | 0 | 0 |
| FR1 | 360 / 12 / 48 | 372 / 0 / 48 | 12 | 0 |
| FR2 | 422 / 31 / 67 | 422 / 31 / 67 | 0 | 0 |

No image aborted at `b1b5b845` either; this session removes no abort. `table.txt` names every group; the remaining Fail cases are in `fbo` (370) and `fragment_out` (31) and `implementation_limits` (2, A1: `max_color_attachments`, `max_draw_buffers`, "Requirement not satisfied").

"begun" (the image's own counter) above the mustpass total of a table row means the filter also matches cases outside `gles3-mustpass.txt` (A1 497 / 472, D1 445 / 443, FB1 598 / 580, FB2 641 / 610, FO6 872 / 866); the Pass / Fail / NotSupported columns above are the mustpass view.

## Method

A clone of the repository checked out at `87881c78` (`git describe` there: `87881c78`), outside the session's checkout. VK-GL-CTS 93bca01 with its externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores), then the fourteen images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> A1 D1 FB1 ... FR2` (section 3): 14 `rc=0 build=87881c78` lines.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console (an MCP server that connected forwards) held that slot. As in the earlier hand-run sessions, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time (execution order A1 D1 FB1 ... FR2; one GL client on the board at a time): the image copied to `/boot/firmware/yakogl-<LABEL>.bin` (A1 alone first; the other thirteen all at once, each sha256 read back) and `tryboot.txt` written with `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`) with the image's sha256 and the text read back before each reboot; `sudo reboot "0 tryboot"`; the console read from the leased serial console (`serial_read`, the tool's saved result converted by a script, never copied by hand, cut from the firmware's first line to the image's end line); Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back to its previous content, read back. After the last image no `yakogl-*.bin` was left and `tryboot.txt` read as before the session. `config.txt` was not touched. The images' local window check (`board.sh pi4 run --dry-run`) was not run by hand.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build 87881c78 ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the fourteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
