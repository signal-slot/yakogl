<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 texture images T06-T17b (T06 T07 T08 T09 T10 T11 T12 T13 T14b T15 T16 T17 T17b) at 87881c78

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-tex2`: rows 82-89, 92-95 and 97 of `tools/cts/pi4-gles3-tree.txt` at `87881c78` (counted from 0; 13 images: T06 T07 T08 T09 T10 T11 T12 T13 T14b T15 T16 T17 T17b). T14, T14a1, T14aa, T14aa1, T17a1, T17aa, T17aaa and T17aaaa are not in this session: item `board-aborts` runs them with `texture-2048.conf`. Item `board-tex2` of plan #399 (issue #1261). The tree lines of these thirteen labels are the same at `87881c78` and at the integration tip (`ddf0a5f1`), except that the tip's T09 comment carries `(with texture-2048.conf)` (4d23bc3d); this session builds T09 the way `87881c78`'s tree says, with `rpi_4b.conf` (see T09 below).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 83 | T06 | 308 | 6 | 12 | 0 | 38.4 s | 0 | none | DONE |
| 84 | T07 | 196 | 28 | 0 | 0 | 37.8 s | 0 | none | DONE |
| 85 | T08 | 0 | 288 | 0 | 0 | 40.4 s | 0 | none | DONE |
| 86 | T09 | 72 | 150 | 0 | 0 | 97.6 s | 0 | none | DONE |
| 87 | T10 | 327 | 7 | 0 | 0 | 84.6 s | 0 | none | DONE |
| 88 | T11 | 319 | 1 | 0 | 0 | 34.4 s | 0 | none | DONE |
| 89 | T12 | 194 | 3 | 0 | 0 | 64.0 s | 0 | none | DONE |
| 90 | T13 | 306 | 6 | 0 | 0 | 32.8 s | 0 | none | DONE |
| 93 | T14b | 174 | 0 | 0 | 0 | 37.9 s | 0 | none | DONE, all passed |
| 94 | T15 | 0 | 0 | 432 | 0 | 14.0 s | 0 | none | DONE, none failed |
| 95 | T16 | 0 | 0 | 504 | 0 | 16.1 s | 0 | none | DONE, none failed |
| 96 | T17 | 181 | 0 | 72 | 0 | 17.4 s | 0 | none | DONE, none failed |
| 98 | T17b | 180 | 0 | 0 | 0 | 16.1 s | 0 | none | DONE, all passed |

(The table's "row" column counts from 1; the rows of this session are 82-89, 92-95 and 97 counted from 0, the `TREE_ROWS` form.)

## Result

Run 2026-11-26 11:54 to 13:12 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr, tryboot). 3,766 mustpass cases in the thirteen images, all 3,766 with a result: **2,257 pass, 489 fail, 1,020 NotSupported, 0 other, 0 never began**. All thirteen images ended `complete=1` with `0 without` a result: no image aborted, so no follow-on image was needed. 0 GPU recoveries, 0 slow jobs, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run. The longest run is T09's 97.6 s of the 210 s record deadline.

T17, which stopped at `b1b5b845` on its last case `wrap.etc2_eac_rgba8.clamp_clamp_linear_npot` (ResourceError, then dEQP stopped), now runs to its end line: that case passes.

T09: its six `texture.size.2d.2048x2048_*` cases (`rgb888`, `rgba4444`, `rgba8888`, each plain and `_mipmap`; cases #154-#159) are refused draws (`R`, a Fail in the mustpass view), cause `a texture larger than the tiled-copy budget (CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES) and the texture arena (8 MiB tiled)`. That is the build's `rpi_4b.conf` budget, as at `b1b5b845` (the same six cases failed there); the tip's tree builds T09 with `texture-2048.conf` for exactly these cases, which this session did not run.

Against `b1b5b845` (`docs/es3/pi4-gles3-fails.tsv`, the same thirteen images, 3,766 mustpass cases there too): 1,094 cases went from not-Pass to Pass, 0 from Pass to not-Pass, 0 changed from one non-Pass status to another:

| image | b1b5b845 Pass/Fail/NotSupported/other | 87881c78 Pass/Fail/NotSupported/other | not-Pass to Pass | Pass to not-Pass |
|---|---|---|---|---|
| T06 | 106 / 208 / 12 / 0 | 308 / 6 / 12 / 0 | 202 | 0 |
| T07 | 140 / 84 / 0 / 0 | 196 / 28 / 0 / 0 | 56 | 0 |
| T08 | 0 / 288 / 0 / 0 | 0 / 288 / 0 / 0 | 0 | 0 |
| T09 | 72 / 150 / 0 / 0 | 72 / 150 / 0 / 0 | 0 | 0 |
| T10 | 212 / 122 / 0 / 0 | 327 / 7 / 0 / 0 | 115 | 0 |
| T11 | 184 / 136 / 0 / 0 | 319 / 1 / 0 / 0 | 135 | 0 |
| T12 | 119 / 78 / 0 / 0 | 194 / 3 / 0 / 0 | 75 | 0 |
| T13 | 180 / 132 / 0 / 0 | 306 / 6 / 0 / 0 | 126 | 0 |
| T14b | 78 / 96 / 0 / 0 | 174 / 0 / 0 / 0 | 96 | 0 |
| T15 | 0 / 0 / 432 / 0 | 0 / 0 / 432 / 0 | 0 | 0 |
| T16 | 0 / 0 / 504 / 0 | 0 / 0 / 504 / 0 | 0 | 0 |
| T17 | 36 / 0 / 216 / 1 | 181 / 0 / 72 / 0 | 145 | 0 |
| T17b | 36 / 0 / 144 / 0 | 180 / 0 / 0 / 0 | 144 | 0 |
| total | 1,163 / 1,294 / 1,308 / 1 | 2,257 / 489 / 1,020 / 0 | 1,094 | 0 |

What the remaining non-Pass results are (`table.txt` lists every case):

- T06, T10, T11, T12, T13: 23 Fail, every one an `rgba32f` case (`format.sized.{2d_array,3d,cube}.rgba32f_{pot,npot}`, `specification.basic_teximage2d/3d`, `basic_texsubimage2d/3d`, `teximage2d_pbo`, `texstorage2d/3d.format`, `texsubimage2d/3d_pbo`, `texsubimage2d/3d_unpack_params` of `rgba32f`), "Image comparison failed". T06's 12 NotSupported are as at `b1b5b845`.
- T07: 28 Fail in `mipmap.{2d,3d,cube}` `base_level`, `max_lod` and `min_lod` (4 each of 7 groups).
- T08: all 288 `shadow.2d` and `shadow.2d_array` cases Fail, as at `b1b5b845`; T09: all 144 `shadow.cube` cases Fail, as at `b1b5b845`, plus the six refused 2048x2048 cases above.
- T15, T16: every case NotSupported (`texture.wrap.astc_*`: ASTC is not advertised), as at `b1b5b845`. T17: 72 NotSupported, `wrap.astc_8x8` and `wrap.astc_8x8_srgb` (36 each), as at `b1b5b845`; its 144 EAC cases (`eac_r11`, `eac_rg11`, `eac_signed_r11`, `eac_signed_rg11`, NotSupported there) and `etc2_eac_rgba8.clamp_clamp_linear_npot` (ResourceError there) now pass, with the 36 `etc1` cases that passed then too (181). T17b's 144 ETC2 cases, NotSupported at `b1b5b845`, all pass.

## Method

A git worktree of the repository at `87881c78` (`git describe` there: `87881c78`), outside the session's checkout. VK-GL-CTS 93bca01 with its externals, the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores, 229 s), then the images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> <labels>` (section 3; T06 alone first, then the other twelve into a second directory): 13 `rc=0 build=87881c78` lines, each image `RAM: 166468 KB` (63.50 %; T10 166472 KB) of the 256 MB window.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console (an MCP server that connected forwards) held that slot. As in the earlier hand-run sessions, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time in table order: the image copied to `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines (`[all]`, `kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`), the image's sha256 and the text read back before each reboot; `sudo reboot "0 tryboot"`; Linux back (a new boot_id), the staged image and `tryboot.txt` removed and read back (no `tryboot.txt` was there before the session, so none is left). The console was read from the leased serial console (`serial_read` from the offset before the reboot, the tool's saved results converted by a script, never copied by hand, cut from the firmware's first line to the image's end line). After the last image no `yakogl-*.bin` and no `tryboot.txt` was left. `config.txt` was not touched. The images' local window check (`board.sh pi4 run --dry-run`) was not run by hand.

T10 ran twice: in the first cycle the serial bridge stopped delivering output in the middle of a line after case #160 and resumed only after Linux was back (Linux's own boot lines were lost too, and a line written to the UART from Linux afterwards arrived), so that console has no end line and is not kept; the second cycle of the same image (same sha256) is `T10.console.txt`. `cycles.txt` lists both.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build 87881c78 ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the thirteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
