<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 1-16 (A1 B1 C1 D1 FB1 FB2 FB3 FB4 FO1 FO2 FO3 FO4 FO5 FO6 FR1 FR2) at b1b5b845

commit: b1b5b8456ec69247c64d1bd07e42461266dc5136

Session `2026-11-pi4-gles3-b1b5b845-rows00-15`: rows 1-16 of `tools/cts/pi4-gles3-tree.txt` (16 images: A1 B1 C1 D1 FB1 FB2 FB3 FB4 FO1 FO2 FO3 FO4 FO5 FO6 FR1 FR2).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 1 | A1 | 461 | 11 | 0 | 0 | 28.8 s | 0 | none | DONE |
| 2 | B1 | 695 | 0 | 0 | 0 | 23.3 s | 0 | none | DONE, all passed |
| 3 | C1 | 602 | 0 | 0 | 0 | 25.0 s | 0 | none | DONE, all passed |
| 4 | D1 | 367 | 0 | 76 | 0 | 18.7 s | 0 | none | DONE, none failed |
| 5 | FB1 | 559 | 14 | 7 | 0 | 17.5 s | 0 | none | DONE |
| 6 | FB2 | 445 | 19 | 146 | 0 | 28.1 s | 0 | none | DONE |
| 7 | FB3 | 157 | 136 | 36 | 0 | 14.0 s | 0 | none | DONE |
| 8 | FB4 | 108 | 335 | 66 | 0 | 28.3 s | 0 | none | DONE |
| 9 | FO1 | 677 | 0 | 0 | 0 | 30.7 s | 0 | none | DONE, all passed |
| 10 | FO2 | 475 | 0 | 0 | 0 | 22.0 s | 0 | none | DONE, all passed |
| 11 | FO3 | 227 | 0 | 0 | 0 | 12.5 s | 0 | none | DONE, all passed |
| 12 | FO4 | 450 | 0 | 0 | 0 | 24.1 s | 0 | none | DONE, all passed |
| 13 | FO5 | 475 | 0 | 0 | 0 | 45.2 s | 0 | none | DONE, all passed |
| 14 | FO6 | 866 | 0 | 0 | 0 | 58.2 s | 0 | none | DONE, all passed |
| 15 | FR1 | 360 | 12 | 48 | 0 | 13.5 s | 0 | none | DONE |
| 16 | FR2 | 422 | 31 | 67 | 0 | 15.0 s | 0 | none | DONE |

(The table's "row" column counts from 1; the rows of this session are rows
0-15 counted from 0, the `TREE_ROWS=0` (A1 alone) and `TREE_ROWS=1-15` of the
recipe.)

## Result

Run 2026-11-11 16:21-17:51 UTC on the Raspberry Pi 4 (V3D 4.2, Zephyr
af17c0c8, tryboot), item pi4-s1 of plan #320 (issue #1261), session 1 of the
one-commit Pi 4 dEQP-GLES3 record: the first sixteen labels A1 ... FR2 of
`tools/cts/pi4-gles3-tree.txt`, A1 alone first, then rows 1-15 in one
`TREE_ROWS=1-15` chain. 8,350 mustpass cases in the sixteen images, all 8,350
with a result: **7,346 pass, 558 fail, 446 NotSupported, 0 never began**. All
sixteen images ended `complete=1` with `0 without` a result: no image aborted
(`check-pi4-gles3-log.py` has nothing to report as aborted). 0 GPU recoveries,
0 job timeouts, 0 hung jobs, 0 MMU faults, 0 resets before the end of any run;
`pi4-console-scan.py` finds one slow job (D1: job 1288 render, 3,153 ms, past
the 1,000 ms mark; the image still finished). No run came near its window:
the longest is FO6's 58.2 s of the 210 s record deadline. Every `board.sh`
verdict line is `rc=0 verdict=done boot=tryboot row boot_verdict=complete
result=legacy:v3d_deqp_0 resets=0 record=absent`, Linux back 296-327 s after
the reboot (`cycles.txt`).

FO6 (`fragment_ops.scissor.clear_int_buffer`) aborted at `0ff58329` on a
scissored integer clear; at `b1b5b845` it runs to the end: 872 cases, all pass.
M1 (`pbo`, row 17, aborted at `0ff58329` too) is in session 2's range.

"begun" (the image's own counter) above the mustpass total of a table row
means the filter also matches cases outside `gles3-mustpass.txt` (begun /
mustpass: A1 497 / 472, D1 445 / 443, FB1 598 / 580, FB2 641 / 610, FO6 872 /
866; the other eleven images begin exactly their mustpass cases); the
Pass / Fail / NotSupported columns above are the mustpass view.

No image overran or hung, so nothing was re-split (recipe section 6):
`tools/cts/pi4-gles3-tree.txt` is unchanged and `check-gles3-tree.py` still
ends `91 images, 42810 matched once, 0 not matched, 0 matched more than once,
0 empty images`.

### The non-Pass cases, by group

`table.txt` names every one. By group (mustpass view): `fbo` 504 Fail / 255
NotSupported, `fragment_out` 43 / 115, `implementation_limits` 11 / 0,
`draw_buffers_indexed` 0 / 76; every other group passes in full:
`attribute_location` 274, `buffer` 695, `clipping` 602, `color_clear` 19,
`default_vertex_array_object` 1, `default_vertex_attrib` 80,
`depth_stencil_clear` 10, `dither` 20, `draw` 322, `fence_sync` 10,
`fragment_ops` 3,170 (all of it, FO1-FO6), `info` 6, `instanced` 45,
`prerequisite` 3.

- `fbo` (FB1-FB4): FB4 335 Fail (`fbo.render.*` recreate / resize /
  shared_colorbuffer / shared_depth_stencil / stencil_clear 275, `fbo.msaa.{2,4}_samples` 34,
  `fbo.invalidate.*` 26, ...), FB3 136 Fail (`fbo.color.tex2d` / `tex2darray` /
  `tex3d` / `texcube`, `repeated_clear` 33, `depth_test_clamp` /
  `depth_write_clamp` 10), FB2 19 (`blit.depth_stencil` 16,
  `blit.default_framebuffer` 3) and 146 NotSupported (`blit.conversion` 112,
  ...), FB1 14 (`completeness.renderable`). `fbo.msaa.8_samples` is
  NotSupported (22).
- `fragment_out` (FR1, FR2): `array.float` 12 Fail and 48 NotSupported,
  `basic.float` 48 NotSupported, 31 `random.*` Fail and 19 NotSupported.
- `implementation_limits` (A1): 11 Fail (`max_color_attachments`,
  `max_draw_buffers`, the `max_*_uniform_*` / `max_varying_*` /
  `max_vertex_output_components` / `max_fragment_input_components` queries,
  `compressed_texture_formats`, `num_compressed_texture_formats`).
- `draw_buffers_indexed` (D1): 76 NotSupported (`overwrite_common` 18,
  `overwrite_indexed` 18, `random.max_*_draw_buffers` 40).

The causes are not read here: the failure groups go to the triage item
(`gles3-triage.py --gpu v3d`).

## Method

Everything was built from a detached worktree of `b1b5b845` outside the
repository (`git describe` there: `b1b5b845`): the gles2 stage 1 through
`tools/cts/deps-env.sh --build --board rpi_4b DEQP_WORK`, the gles3 stage 1 of
`docs/es3/board-recipe-pi4.md` section 2 (VK-GL-CTS 93bca01, Zephyr SDK 1.0.1;
453 ninja steps), then the sixteen images with `DEQP_MODULE=gles3
tools/cts/pi4-deqp-images.sh` (section 3): 16 `rc=0 build=b1b5b845` lines
(A1 `sha256=b8b88d18...`, each `.config` passed by `check-deqp-images.py
--config --gles3`). The runs used the board runner itself, `tools/board.sh pi4
tree` with tryboot, `TREE_ROWS=0` first (A1 alone), then `TREE_ROWS=1-15`; the
serial bridge took the runner's own capture (unlike the earlier by-hand
records), `BOARD_UART` and `BOARD_HOST` from the environment.

Two notes on the first attempt: `board.sh`'s precheck refused the first A1
start (`rc=3`: it saw an ssh session from an address that is not one of the
runner host's own; it was this task's own forwarded connection, the only
remote session on the board, so the retry ran with the runner told that
address); nothing was staged or booted by that attempt, and its scratch record
was deleted. One GL client at a time: each image ran alone, the next staged
only after Linux was back and the previous one's files were gone (`cleanup=`
in `cycles.txt`: the staged image removed, `tryboot.txt` put back, read back).

## Files

Every `.txt` starts with three `#` header lines (SPDX, then a title); the
tally reads past them.

- `<LABEL>.console.txt`: the image's console from the firmware's first line of
  its boot to the `v3d_deqp: build b1b5b845 ...` end line, CRs and NULs
  dropped, no time stamps.
- `<LABEL>.tally.txt`: `tools/cts/pi4-session-tally.sh` (deqp-uart-tally.py
  `--label <LABEL> <LABEL>.console.txt` from inside this directory).
- `table.txt`: `--table` over the sixteen consoles, in the table's order.
- `cycles.txt`: per image the sha256 of `zephyr.bin`, the reboot time, the
  seconds until Linux was back, the clean-up read-back, the `cases in` line,
  the jobs line and the end line (`board.sh` does not record the boot_id
  before the reboot; it checks that it changed, `back=1`).
