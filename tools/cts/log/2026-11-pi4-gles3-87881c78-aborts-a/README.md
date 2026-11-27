<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 at 87881c78: the eight images that aborted at b1b5b845 (S03a S07 T17a TF1a TF1b TF2a TF2b TF2c)

commit: 87881c78c2f3e5347dfa84aadf4380aead77ff50

Session `2026-11-pi4-gles3-87881c78-aborts-a`, item board-aborts-a of plan #399 (issue #1261). At `b1b5b845` these eight images aborted on a refused draw (`../2026-11-pi4-gles3-b1b5b845-aborts/`). `docs/notes/v3d-refusals.md` says `6d2e7d0d` removes the S03a and S07 aborts, `455c3bb5` the T17a one, and `6581f22b` plus `b7c1484e` those of TF1a to TF2c. This session checks that on the board. The images are built from `87881c78` with that commit's own `tools/cts/pi4-gles3-tree.txt`, with no re-split. The mustpass case counts there are S03a 1141, S07 1, T17a 71, TF1a 184, TF1b 189, TF2a 177, TF2b 189 and TF2c 60, so 2,012 cases in all. The integration tip's tree has since split S03a into S03a1 and S03a2a..S03a7b, and TF1a, TF1b, TF2a and TF2b into a label for the cases before the old aborting case and a one-case label for that case (item tree-resplit). Those labels are not in this session: item board-aborts runs them.

Per-image files, relative to this directory: `<LABEL>.console.txt` and `<LABEL>.tally.txt`. The counts below are the mustpass view of each tally's TOTAL line ("no result" means a case of the image's filter that never began). The run, Pass, Fail and NotSupported columns of the dEQP counters (`deqp: done`) count every case the filter ran, including the ones outside the mustpass.

| image | mustpass | Pass | Fail | NotSupported | other | no result | dEQP ran | end line | carve-out high-water (KiB, of 48636) |
|---|---|---|---|---|---|---|---|---|---|
| S03a | 1141 | 204 | 0 | 0 | 0 | 937 | 276 begun, 276 P, in 209.0 s | INCOMPLETE (the photo deadline came first) | 22918 |
| S07 | 1 | 1 | 0 | 0 | 0 | 0 | 4 P, complete=1 | DONE, all passed | 21653 |
| T17a | 71 | 71 | 0 | 0 | 0 | 0 | 71 P, complete=1 | DONE, all passed | 20190 |
| TF1a | 184 | 184 | 0 | 0 | 0 | 0 | 184 P, complete=1 | DONE, all passed | 20314 |
| TF1b | 189 | 189 | 0 | 0 | 0 | 0 | 189 P, complete=1 | DONE, all passed | 20314 |
| TF2a | 177 | 177 | 0 | 0 | 0 | 0 | 177 P, complete=1 | DONE, all passed | 20262 |
| TF2b | 189 | 189 | 0 | 0 | 0 | 0 | 189 P, complete=1 | DONE, all passed | 20262 |
| TF2c | 60 | 60 | 0 | 0 | 0 | 0 | 60 P, complete=1 | DONE, all passed | 20331 |
| total | 2012 | 1075 | 0 | 0 | 0 | 937 | | | |

## Result

Run on 2026-11-27 from 02:30 to 03:12 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr, tryboot), one cycle per image, with no retry.

**No image aborted.** Not one console has a refused draw (`yakogl/v3d: draw refused`), a `Test run was ABORTED!`, a `STOPPED EARLY` or a ResourceError. Every image reports `allocations refused for lack of GPU memory: 0`, 0 slow jobs, 0 MMU faults, 0 recoveries, no hung job and `FIRST FAILURE: none`. The board shows what the notes said: `6d2e7d0d` removes the S03a and S07 aborts, `455c3bb5` the T17a one, and `6581f22b` plus `b7c1484e` those of TF1a, TF1b, TF2a, TF2b and TF2c.

- S07, T17a, TF1a, TF1b, TF2a, TF2b and TF2c ended `DONE, all passed`, `complete=1`, with `N cases begun, N with a result, 0 without`. All 871 of their mustpass cases pass. That covers S07's `reflect.highp_fragment.vec3`, which never began at `b1b5b845` (S07's other three cases are outside the mustpass), and T17a's 71 `wrap.etc2_*` cases. It also covers the cases at which TF1/TF2 stopped at `b1b5b845` and which are now in these images: `array.interleaved.lines.highp_mat3` (TF1a), `array.separate.lines.highp_mat3` (TF1b), `basic_types.interleaved.lines.lowp_mat4` (TF2a) and `basic_types.separate.lines.highp_mat4` (TF2b).
- S03a no longer aborts: its first case `precision.abs.highp_fragment.vec2` passes, and so do the 275 after it. But it is too long for one image. The image's record came at its 210 s deadline (`276 cases in 209009 ms, record at 210 s after boot`) with case #276 `precision.cosh.lowp_vertex.vec3` begun and no result for it. Its end line is `v3d_deqp: build 87881c78 INCOMPLETE (the photo deadline came first): 276 cases, P 276 F 0 R 0 N 0 other 0`. That gives 204 of the 1141 mustpass cases a Pass and leaves 937 without a result (`table.txt` names them, `NoResult`). It is an out-of-time stop, not an abort: the image prints no `deqp: done` or `deqp: summary` line, so its tally says `MISSING: no summary line`, and `check-pi4-gles3-log.py` counts it as without `complete=1`. The tip's S03a1 and S03a2a..S03a7b split (item s03a-window-split) exists for this time limit. Item board-aborts runs those labels.

The highest carve-out high-water of the eight is 22918 KiB of 48636 (S03a), so no image comes close to the 48 MiB carve-out.

## Method

A git worktree of the repository at `87881c78` (`git describe` there: `87881c78`), outside the session's checkout. This machine's dependency cache had no dEQP stage 1. So `tools/cts/deps-env.sh --build --board rpi_4b DEQP_WORK` fetched VK-GL-CTS 93bca01 and built the gles2 stage 1. Then the gles3 stage 1 of `docs/es3/board-recipe-pi4.md` section 2 (`cts-build-zephyr.sh`, `MODULE=gles3`) built in 453 ninja steps and ended `cts-build-zephyr: libraries in <WORK>/build`. Last came the images, with `DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <dir> S03a S07 T17a TF1a TF1b TF2a TF2b TF2c` (section 3). That printed eight `rc=0 build=87881c78` lines, each image at `RAM: 166468-166488 KB` (63.50-63.51 %) of the 256 MB window. `cycles.txt` has each image's sha256.

The board runner could not capture the console: the Pi 4's UART bridge takes one client at a time, and the leased serial console (an MCP server that connected forwards) held that slot. As in the earlier hand-run sessions, `tools/board.sh pi4`'s steps were done by hand in its order, one image at a time:

1. Check that the board has no `/boot/firmware/yakogl-<LABEL>.bin`, no `tryboot.txt`, no `tryboot.txt.yk-orig` and no `config-linux.txt`. It had no `tryboot.txt` before the session.
2. Copy the image to `/boot/firmware/yakogl-<LABEL>.bin` and write `tryboot.txt` with `pi4_config`'s lines (`[all]`, `kernel=yakogl-<LABEL>.bin`, `arm_64bit=1`, `enable_uart=1`, `uart_2ndstage=1`; no `hdmi_timings`).
3. Read both back before the reboot: the image's sha256 is equal and the text is the same.
4. Reboot with `reboot "0 tryboot"`.
5. Wait 240 s, which is past the image's 210 s record and before the ~250 s PM watchdog reset.
6. Read the console with `serial_read` from the offset before the reboot. The tool's saved results were converted by a script, never copied by hand, and cut from the firmware's first line to the image's end line, with CRs and NULs dropped (the cut holds no other control byte).
7. Wait for Linux (a new boot id).
8. Remove the staged image and `tryboot.txt`, and check that neither is there.

`config.txt` was not touched. The board is left as found: no image and no `tryboot.txt`. Linux was back 301-312 s after each reboot.

Tallies: `PI4_TREE=<87881c78's pi4-gles3-tree.txt> tools/cts/pi4-session-tally.sh <this directory>`. The tree at `87881c78` is needed because the tip's tree no longer has the label S03a. The mustpass list (`tools/cts/gles3-mustpass.txt`) is the same at `87881c78` and at the tip. `table.txt` is `deqp-uart-tally.py --mustpass ../../gles3-mustpass.txt --table` over the eight consoles, run from inside this directory. Its counters column names the directory rather than the label, and S03a has no counters line because it printed no `deqp: summary`. The consoles were then passed through `tools/cts/console-redact.py`.

## Files

- `<LABEL>.console.txt`: that boot's console, from the firmware's first line to the image's end line, after a three-line header.
- `<LABEL>.tally.txt`: `deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt <LABEL>.console.txt`, through `pi4-session-tally.sh`.
- `table.txt`: `--table` over the eight consoles, with every mustpass case that is not a Pass named (S03a's 937 `NoResult`).
- `cycles.txt`: per image in run order: the sha256, the Linux boot id before and after, the reboot time, the seconds to Linux, the clean-up read-back, and the `cases in`, `slow jobs`, `carve-out` and end lines.
