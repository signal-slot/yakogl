<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 47-64 (S26 S27 S5f S5v S30 S31 S32a S32b S32c S32d T01 T02 T03 T04 F3C1 F3C2 F3C3 F3C4) at b1b5b845

commit b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-rows46-60`: rows 47-64 of `tools/cts/pi4-gles3-tree.txt` (18 images: S26 S27 S5f S5v S30 S31 S32a S32b S32c S32d T01 T02 T03 T04 F3C1 F3C2 F3C3 F3C4).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 47 | S26 | 700 | 0 | 0 | 0 | 25.4 s | 0 | none | DONE, all passed |
| 48 | S27 | 987 | 0 | 0 | 0 | 35.6 s | 0 | none | DONE, all passed |
| 49 | S5f | 66 | 0 | 0 | 0 | 6.6 s | 0 | none | DONE, all passed |
| 50 | S5v | 70 | 0 | 0 | 0 | 4.3 s | 0 | none | DONE, all passed |
| 51 | S30 | 915 | 0 | 0 | 0 | 38.6 s | 0 | none | DONE, all passed |
| 52 | S31 | 648 | 0 | 0 | 0 | 22.6 s | 0 | none | DONE, all passed |
| 53 | S32a | 262 | 260 | 0 | 0 | 33.9 s | 0 | none | DONE |
| 54 | S32b | 0 | 34 | 0 | 0 | 2.5 s | 0 | none | DONE |
| 55 | S32c | 0 | 0 | 0 | 1 | 0.2 s | 0 | none | incomplete (no complete deqp summary) |
| 56 | S32d | 116 | 2 | 0 | 0 | 5.1 s | 0 | none | DONE |
| 57 | T01 | 0 | 0 | 266 | 0 | 9.6 s | 0 | none | DONE, none failed |
| 58 | T02 | 117 | 0 | 12 | 0 | 10.1 s | 0 | none | DONE, none failed |
| 59 | T03 | 0 | 122 | 12 | 0 | 16.9 s | 0 | none | DONE |
| 60 | T04 | 112 | 0 | 12 | 0 | 64.6 s | 0 | none | DONE, none failed |
| 61 | F3C1 | 0 | 14 | 0 | 0 | 1.3 s | 0 | none | DONE |
| 62 | F3C2 | 0 | 14 | 0 | 0 | 1.3 s | 0 | none | DONE |
| 63 | F3C3 | 0 | 14 | 0 | 0 | 1.3 s | 0 | none | DONE |
| 64 | F3C4 | 0 | 14 | 0 | 0 | 1.3 s | 0 | none | DONE |

(The table's "row" column counts from 1 and follows the table as re-split by this item (94 images); the session's `TREE_ROWS` were 46-60 of the 91-image table, S32 being row 52 there. In the 94-image table the same images are rows 46-63 counted from 0.)

## Result

Run 2026-11-15 on the Raspberry Pi 4 (V3D 4.2, Zephyr af17c0c8, tryboot), item pi4-s4 of plan #320 (issue #1261), session 4 of the one-commit Pi 4 dEQP-GLES3 record: rows 46-60 of `tools/cts/pi4-gles3-tree.txt` (S26 S27 S5f S5v S30 S31 S32 T01 T02 T03 T04 F3C1 F3C2 F3C3 F3C4), S32 re-split into S32a-d after it aborted again. 4,773 mustpass cases in the eighteen images, 4,770 with a result: **3,993 pass, 474 fail, 302 NotSupported, 1 ResourceError, 3 never began**. Seventeen images ended `complete=1` with `0 without` a result; S32c aborted on its first case (below). 0 GPU recoveries, 0 job timeouts, 0 hung jobs, 0 MMU faults, 0 slow jobs, 0 resets before the end of any run (`pi4-console-scan.py`). The longest run is T04's 64.6 s of the 210 s record deadline, so no image overran and nothing was re-split for time. Every console ends with the image's own end line (`v3d_deqp: build b1b5b845 ...`), Linux was back 298-309 s after each reboot (`cycles.txt`) and every cycle read back as before.

**S26 S27 S5f S5v S30 S31** (`shaders.random`, `return`, `scoping`, `struct`, `switch`, `swizzle_math_operations`, `swizzles`; 3,386 mustpass cases) pass in full, as S26 did at 009f62a5.

**S32 aborted again, at the case it aborted on at 009f62a5.** The 678-case image (`shaders.texture_functions` + `shaders.uniform_block`) stopped at #589, `texture_functions.texturesize.sampler2dshadow_fragment`: `yakogl/v3d: draw refused: a texture of 32-bit channels (R32F, RGBA32F, RGBA32UI, RGBA32I) read by a look-up compiled for the 16-bit TMU output`, `GL error 0x505 in glDrawArrays`, a ResourceError, `Test run was ABORTED!`, `complete=0`, end line `STOPPED EARLY by dEQP (complete=0; later cases never began): 590 cases, P 276 F 313 R 1`. Recipe section 6 (*at the refusing case*): S32 is re-split in `tools/cts/pi4-gles3-tree.txt` (91 images to 94, `check-gles3-tree.py` 42810 matched once) into S32a (the 15 `texture_functions` groups but `texturesize`, 522 mustpass), S32b (`texturesize` without the four shadow look-ups, 34), S32c (`texturesize.sampler2dshadow_*` and `samplercubeshadow_*`, 4) and S32d (`uniform_block`, 118). The aborted original run's console is not kept in this directory (its label is no longer in the table); its figures are in this paragraph.

- S32a: 571 cases begun, `DONE`, 262 pass / 260 fail in the mustpass view: every `texelfetch*` 2D-array / 3D case and every look-up in `texture`, `texturegrad*`, `texturelod*`, `textureoffset`, `textureproj*` other than 2D or cube, with gradients, or a shadow look-up fails at shader build (`Program(NotYet(...))`, the online compiler refusing the program).
- S32b: 34 cases begun, `DONE`, 34 Fail: 18 `NotYet("textureSize of a 3D or array sampler")` and 16 `Got unexpected texture size`. The run is past the point where S32 aborted, so the abort is the shadow look-up only.
- **S32c aborted on its first case**, `texturesize.sampler2dshadow_fragment`, the same refusal; the other three cases (`sampler2dshadow_vertex`, `samplercubeshadow_fragment`, `samplercubeshadow_vertex`) never began (`none` 3 of `table.txt`). Whether they refuse too is not known from this record. `check-pi4-gles3-log.py` reports `aborted, no complete deqp summary (1): S32c`. The cause is one refusal (RC2), not the table: fixing it is a V3D back-end item for plan #184.
- S32d: 138 cases begun, `DONE`, 116 pass / 2 fail (`uniform_block.valid.long_instance_name_fragment` and `_vertex`: the reported instance name is truncated); at 009f62a5 these 118 cases never ran.

**The texture images.** T01 (`texture.compressed`, 322 begun, 266 mustpass): all NotSupported (the console says `ASTC not supported`). T02 (`filtering.2d`): 117 pass, 12 NotSupported (`formats.srgb_r8_*`, `srgb_rg8_*`). T03 (`filtering.2d_array`): 0 pass, 122 Fail in the mustpass view (210 F and 12 N of 222 begun in the image's own counters; every reason `NotYet("a texture look-up other than 2D or cube")`) and the same 12 NotSupported. T04 (`filtering.cube`): 112 pass, 12 NotSupported. F3C1-F3C4 (`filtering.3d.combinations`, 14 names each): 56 of 56 Fail, `NotYet("a texture look-up other than 2D or cube")`, 1.3 s each. The causes are not read here; the failure groups go to the triage item.

**T04 was run twice.** The first capture of T04 lost its first lines (the leased console's 1 MiB buffer wrapped: T04's output is 694 KB); the image was staged and run again, same sha256, same end line (`DONE, none failed: 230 cases, P 218 F 0 R 0 N 12`), and the second console is the one kept. `cycles.txt` has the second run's reboot time. T04 is the only image whose run needed a second look at the board.

## Method

A clone of the repository at `b1b5b845` outside the checkout (`git describe` there: `b1b5b845`). VK-GL-CTS 93bca01 with its externals (a shallow fetch of that commit with no tags, so the consoles' `dEQP Core` line carries the bare commit hash where the earlier sessions' carry the tag-derived version string), the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (Zephyr SDK 1.0.1, 4 cores), then the images with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh` (section 3): 15 `rc=0 build=b1b5b845` lines for S26-F3C4, then S32a-d (4 more) after the re-split, built from the re-split table with the `b1b5b845` sources; each `.config` passed by `check-deqp-images.py --config --gles3`.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time and the leased serial console (an MCP server that connected forwards) held that slot. As in `../2026-11-pi4-gles3-b1b5b845-rows16-30/README.md`, the steps of `tools/board.sh pi4` were done by hand, in its order, one image at a time (one GL client on the board at a time): free-space and leftover prechecks over ssh, the image installed to `/boot/firmware/yakogl-<LABEL>.bin` and `tryboot.txt` written with `pi4_config`'s lines (`kernel=`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`, the panel's value was not in the environment), both read back (sha256, text) before the reboot; `sudo reboot "0 tryboot"`; the console through the leased serial console (`serial_expect` on the end line, then `serial_read` of the whole run into a file, converted by a script, never copied by hand); Linux back (a new boot_id), the staged image removed and `tryboot.txt` put back to its previous one-newline content, read back. `config.txt` was not touched. The images' local window check (`board.sh pi4 run --dry-run`) was not run by hand; the images link like the 54 that passed it in the earlier records.

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of its boot to the `v3d_deqp: build b1b5b845 ...` end line, CRs and NULs dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the eighteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time (unix seconds), the seconds until Linux was back, the clean-up read-back, the `cases in` line, the jobs line and the end line.
