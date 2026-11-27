<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 texture images (T01 T02 T03 T04 F3C1-F3C4 F3F1-F3F5 F3S1 F3S2 T05a T05b T05c T05ca T05cb T05cc T05d) at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-tex1`: rows 60-81 of `tools/cts/pi4-gles3-tree.txt` (22 images: T01 T02 T03 T04 F3C1 F3C2 F3C3 F3C4 F3F1 F3F2 F3F3 F3F4 F3F5 F3S1 F3S2 T05a T05b T05c T05ca T05cb T05cc T05d), the texture images that the #1261 V3D changes since `b1b5b845` reach (texture refusals, ETC2 sampling, 4096 targets). Item `board-tex1` of plan #399 (issue #1261). The tree lines of these labels are the same at `87881c78` and at `03e3d385` (main when this record was committed); the re-splits of other labels since `87881c78` move them to rows 75-96 of the tree at `03e3d385`.
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 60 | T01 | 0 | 0 | 266 | 0 | 9.5 s | 0 | none | DONE, none failed |
| 61 | T02 | 117 | 0 | 12 | 0 | 10.1 s | 0 | none | DONE, none failed |
| 62 | T03 | 122 | 0 | 12 | 0 | 26.1 s | 0 | none | DONE, none failed |
| 63 | T04 | 112 | 0 | 12 | 0 | 64.6 s | 0 | none | DONE, none failed |
| 64 | F3C1 | 14 | 0 | 0 | 0 | 2.0 s | 0 | none | DONE, all passed |
| 65 | F3C2 | 14 | 0 | 0 | 0 | 2.0 s | 0 | none | DONE, all passed |
| 66 | F3C3 | 14 | 0 | 0 | 0 | 1.9 s | 0 | none | DONE, all passed |
| 67 | F3C4 | 14 | 0 | 0 | 0 | 1.9 s | 0 | none | DONE, all passed |
| 68 | F3F1 | 15 | 0 | 0 | 0 | 2.7 s | 0 | none | DONE, all passed |
| 69 | F3F2 | 15 | 0 | 0 | 0 | 3.7 s | 0 | none | DONE, all passed |
| 70 | F3F3 | 15 | 0 | 0 | 0 | 3.7 s | 0 | none | DONE, all passed |
| 71 | F3F4 | 14 | 0 | 1 | 0 | 2.5 s | 0 | none | DONE, none failed |
| 72 | F3F5 | 0 | 0 | 11 | 0 | 0.5 s | 0 | none | DONE, none failed |
| 73 | F3S1 | 13 | 0 | 0 | 0 | 1.5 s | 0 | none | DONE, all passed |
| 74 | F3S2 | 13 | 0 | 0 | 0 | 1.3 s | 0 | none | DONE, all passed |
| 75 | T05a | 190 | 2 | 4 | 0 | 11.1 s | 0 | none | DONE |
| 76 | T05b | 12 | 0 | 0 | 0 | 0.5 s | 0 | none | DONE, all passed |
| 77 | T05c | 1 | 0 | 0 | 0 | 0.2 s | 0 | none | DONE, all passed |
| 78 | T05ca | 1 | 0 | 0 | 0 | 0.2 s | 0 | none | DONE, all passed |
| 79 | T05cb | 1 | 0 | 0 | 0 | 0.2 s | 0 | none | DONE, all passed |
| 80 | T05cc | 1 | 0 | 0 | 0 | 0.2 s | 0 | none | DONE, all passed |
| 81 | T05d | 8 | 0 | 0 | 0 | 0.9 s | 0 | none | DONE, all passed |

(The "row" column counts table rows from 0, as `TREE_ROWS` does: these are rows 60-81 of the tree at `87881c78`, the commit built.)

## Result

Run 2026-11-26 15:16 to 17:22 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr, tryboot). 1,026 mustpass cases in the twenty-two images, all 1,026 with a result: **706 pass, 2 fail, 318 NotSupported, 0 other, 0 never began**. All twenty-two images ended `complete=1` with `0 without` a result: no image aborted, so no follow-on image was needed. No `draw refused` line, 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run. The longest run is T04's 64.6 s of the 210 s record deadline.

The 2 Fail cases are in T05a: `texture.format.sized.2d.rgba32f_npot` and `texture.format.sized.2d.rgba32f_pot`, both "Image comparison failed" (Fail at `b1b5b845` too). The 318 NotSupported are T01's 266 ASTC cases ("ASTC not supported"), 12 sRGB R8/RG8 cases in each of T02, T03 and T04 (`GL_EXT_texture_sRGB_R8` / `_RG8` not supported), F3F4's 1 and F3F5's 11 3D sRGB R8/RG8 filtering cases, and T05a's 4 `sized.2d.srgb_r8` / `srgb_rg8` cases: all NotSupported at `b1b5b845` with the same status.

Against `b1b5b845` (`docs/es3/pi4-gles3-fails.tsv`, the same twenty-two images, 1,026 mustpass cases there too; T05ca-T05cc from `2026-11-pi4-gles3-b1b5b845-aborts`): 335 cases went from not-Pass to Pass, 0 from Pass to not-Pass, 0 changed from one non-Pass status to another:

| image | b1b5b845 Pass/Fail/NotSupported/other | 87881c78 Pass/Fail/NotSupported/other | not-Pass to Pass | Pass to not-Pass |
|---|---|---|---|---|
| T01 | 0 / 0 / 266 / 0 | 0 / 0 / 266 / 0 | 0 | 0 |
| T02 | 117 / 0 / 12 / 0 | 117 / 0 / 12 / 0 | 0 | 0 |
| T03 | 0 / 122 / 12 / 0 | 122 / 0 / 12 / 0 | 122 | 0 |
| T04 | 112 / 0 / 12 / 0 | 112 / 0 / 12 / 0 | 0 | 0 |
| F3C1-F3C4 | 0 / 56 / 0 / 0 | 56 / 0 / 0 / 0 | 56 | 0 |
| F3F1-F3F3 | 0 / 45 / 0 / 0 | 45 / 0 / 0 / 0 | 45 | 0 |
| F3F4 | 0 / 14 / 1 / 0 | 14 / 0 / 1 / 0 | 14 | 0 |
| F3F5 | 0 / 0 / 11 / 0 | 0 / 0 / 11 / 0 | 0 | 0 |
| F3S1-F3S2 | 0 / 26 / 0 / 0 | 26 / 0 / 0 / 0 | 26 | 0 |
| T05a | 142 / 34 / 20 / 0 | 190 / 2 / 4 / 0 | 48 | 0 |
| T05b | 0 / 4 / 8 / 0 | 12 / 0 / 0 / 0 | 12 | 0 |
| T05c | 0 / 0 / 0 / 1 | 1 / 0 / 0 / 0 | 1 | 0 |
| T05ca | 0 / 0 / 0 / 1 | 1 / 0 / 0 / 0 | 1 | 0 |
| T05cb | 0 / 0 / 0 / 1 | 1 / 0 / 0 / 0 | 1 | 0 |
| T05cc | 0 / 0 / 0 / 1 | 1 / 0 / 0 / 0 | 1 | 0 |
| T05d | 0 / 0 / 8 / 0 | 8 / 0 / 0 / 0 | 8 | 0 |
| total | 371 / 301 / 350 / 4 | 706 / 2 / 318 / 0 | 335 | 0 |

The four `other` of `b1b5b845` are the ResourceError aborts of T05c, T05ca, T05cb and T05cc (the ETC2 cube draws refused, `etc2_eac_rgba8_cube_npot`, `etc2_eac_rgba8_cube_pot`, `etc2_rgb8_cube_npot`, `etc2_rgb8_cube_pot`): each now runs to the end and passes, so the T05c abort of `b1b5b845` is gone. Of the 335 cases that now pass, 299 were Fail (T03's 122 `filtering.2d_array`, the 141 `filtering.3d` cases of F3C*, F3F1-F3F4 and F3S*, T05a's 32 `format.unsized` 2D-array and 3D cases, T05b's 4 ETC2 2D cases), 32 were NotSupported (T05a's 16 `format.compressed.eac_*` 2D and cube cases, T05b's 8 ETC2 sRGB and punch-through 2D cases, T05d's 8 ETC2 cube cases of the other formats: these compressed formats are now advertised and sampled) and 4 were the ResourceError aborts. `table.txt` names every not-passed case.

"begun" (the image's own counter) above the mustpass total of a table row means the filter also matches cases outside `gles3-mustpass.txt` (T01 322 / 266, T02 234 / 129, T03 222 / 134, T04 230 / 124); the Pass / Fail / NotSupported columns above are the mustpass view.

## Method

A git worktree of `87881c78` (`git describe` there: `87881c78`), outside the session's checkout. VK-GL-CTS 93bca01 with its externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores, 340 s), then the twenty-two images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> <LABEL>...` (section 3; T01 alone first, then the other twenty-one): 22 `rc=0 build=87881c78` lines, each image's sha256 in `cycles.txt`.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console (an MCP server that connected forwards) held that slot. As in the earlier hand-run sessions, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time in table order (one GL client on the board at a time): ssh prechecks (no `yakogl-*.bin`, no `tryboot.txt` and no `config-linux.txt` left in `/boot/firmware`), the image installed as `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`), the image's sha256 and the text read back before each reboot; `sudo reboot "0 tryboot"`; the console read from the leased serial console (`serial_read`, the tool's saved results converted by a script, never copied by hand, cut from the firmware's first line of that boot to the image's end line); Linux back (a new boot id), then the staged image and `tryboot.txt` removed and read back (none left). The card had no `tryboot.txt` before the session, so none was put back; after the last image no `yakogl-*.bin` and no `tryboot.txt` was left. `config.txt` was not touched. The images' local window check (`board.sh pi4 run --dry-run`) was not run by hand. After the consoles were saved, `tools/cts/console-redact.py` rewrote the dEQP version string of each console (no other change), and the tallies and `table.txt` were made from the redacted consoles.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build 87881c78 ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the twenty-two consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
