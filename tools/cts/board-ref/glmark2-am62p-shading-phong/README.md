<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 `shading=phong` on the Verdin AM62P, 2026-10-19

This run read back the last frame (800x600) of glmark2's `shading` scene
with `shading=phong` (a cat, lit with per-fragment Phong), drawn by YakoGL
on the AM62P's PowerVR BXS-4-64 under Zephyr. The pixels are the GPU's own
frame, read back with `glReadPixels` and sent over the UART as text
(`CONFIG_GLMARK2_FRAME_DUMP`, `driver/docs/glmark2.md` "The frame dump"),
then decoded by `tools/cts/glmark2-fb.py`.

The decoded picture (`glmark2-phong.png`) is no longer in the tree: it is a
rendering of glmark2's `cat.3ds` model, and glmark2 with its data is
GPL-3.0 (`docs/release/license-audit.md`, item 14). The run's own picture
value below (probes, CRCs, mean) stays as the record of what the GPU drew.

- Image: the glmark2 sample (GM row of `tools/cts/common-tree.txt`) built by
  `tools/cts/common-images.sh am62p` from the frame dump with the watchdog
  fed per row, with `CONFIG_GLMARK2_FRAME_DUMP=y` and
  `GLMARK2_ARGS="-b shading:shading=phong"` set. The settings are not in the
  tree (`default y` and the scene were a scratch commit, not for main).
  The run used the tree of the fix `glmark2 frame dump: feed the board
  watchdog per row`; the same code is in this tree (only a comment there
  differs). Image sha256
  `bc0cc1d353b164fa1424475b2aae369b38702e6b96aa92e386a5343a24f7abc5`.
- Run: 2026-10-19, `uboot-mmc` one-shot boot, console on the serial bridge.
  The dump took about 175 s of UART (about 11.6 KB/s). The run's own result
  line: `glmark2: score 26: 1 scenes ran, 0 failed, 0 unsupported (1 list
  entries); window 800x600, shown yes, GPU resets 0`, then `glmark2: PASS`.
  No `tools/board.sh` verdict line: its console capture could not open the
  bridge (the lease's serial tool held it), so the image was staged and
  booted by hand with the same U-Boot env steps and the console was read
  from the serial tool's transcript. The board came back to Linux,
  `yako_once` empty, the staged file removed.
- Decoder: `[shading] shading=phong: 800x600, 600 of 600 rows good`; `frame
  crc32 bd8a83a5 agrees`.
- Picture value: the dump agrees with the same run's line `glmark2-picture:
  [shading] shading=phong: 800x600; probes c 0505a2ff, tl 000000ff, tr
  000000ff, bl 050586ff, br 0505c0ff; crc16 61d9, crc16q 0226; mean 1 1 28;
  black no` (probes, crc16, crc16q and mean all match).

This record is from that run (an earlier ccfarm task); the code it ran is
the code in this tree. A rebuilt image (GM, sha256
`7fc2e564a3b8b6f5b64e7bdac921e101c1a035fed9cb6bbe5e6f980b916f7d96`, tree
8c2c47fb) compiled, but a second board run was not possible in the task that
re-landed this change (`tools/board.sh` needs BOARD_HOST from the
environment, which only `tools/cts/board-stage.sh` derives, and it was
refused).

A first run, before the watchdog feed, was reset by the RTI watchdog 20 s
into the dump (`rti_arm: the sample stopped saying it is alive`, after row
107): the feed window (`CONFIG_YAKOGL_BOARD_ALIVE_S`, 20 s) lapsed.
