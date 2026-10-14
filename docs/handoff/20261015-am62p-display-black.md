<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-display-black, 2026-10-15, the pull request of branch ccfarm/01a13b7b-5565-76f2-ae9a-41c31fa52c3b

## Open work

Add, after the AM62P items of the list:

- AM62P: the panel stays black (backlight on) under a kexec'd yakogl
  (owner's report, 2026-10-14); the U-Boot splash shows. Not settled: the
  red-fill image ran on the board twice from hx90 (2026-10-15,
  `tools/cts/log/board-sessions/am62p-display-black-2026-10-15.md` and
  its `-console.txt` record) and every register of the path on the board
  equals the Linux dump except the allow-listed deviations and the
  application plane being on; the DSI streams (VPOS advancing, no live
  error), the bridge reads IRQ_STAT 0x00 with video flowing (the first such
  reading), VP2 at 58.8 Hz with 0 underflows, and the image resets back
  into Linux. The picture itself was not seen: the only camera on this
  panel is on Lepton. Ruled out before that by the tree: the bridge's
  CSRs equal Linux's i2cdump (again on the board), `tisci_power_on` is
  enabled in every AM62P sample overlay and the bridge answered over I2C1,
  the safety layer is zeroed/transparent at init. Candidates left, all
  below the registers: the C-vs-Rust order and timing differences of the
  note's table ((a) bridge EN after the DSI attach, (b) VID_VSIZE1
  10/23/11, (c) WAIT_BURST_TIME 4, (d) lanes after PLL_LCK, ...), or the
  owner's run itself (a `HOLD_SECONDS=0` image resets at once). The
  instruments are in the tree: `CONFIG_PVR_SAMPLE_DISPLAY_FILL`
  (known-colour fill of the safety layer) and
  `CONFIG_PVR_SAMPLE_DISPLAY_DUMP` (the Linux dump's read list with the
  counters and IRQ_STAT, before and after the hold), in both sample
  routes.

## Next board session

Add to the AM62P section, before the dEQP steps (the panel must be seen
before any further AM62P display result is trusted):

1. The black-panel diagnosis, from Lepton (the document camera,
   `/dev/video2`): build `gl_triangle` with
   `CONFIG_PVR_SAMPLE_DISPLAY_FILL=0xffff0000`,
   `CONFIG_PVR_SAMPLE_DISPLAY_DUMP=y` and `CONFIG_PVR_SAMPLE_HOLD_SECONDS=40`
   (the `west build` line in
   `tools/cts/log/board-sessions/am62p-display-black-2026-10-15.md`,
   "Board procedure"), kexec it and photograph the panel 15-20 s after the
   jump; the console side was already read on 2026-10-15 (the note's "On
   the board": registers equal to Linux's, DSI streaming, bridge clean) and
   only needs its `sn65dsi8x IRQ_STAT` and `dss stats` lines re-checked.
   Red panel: the path is fine; the owner's black panel is the application
   plane or the image/hold they ran, so repeat with the triangle alone
   (`DISPLAY_FILL=0`) and photograph it. Black panel: the fault is below the
   registers; bisect the note's table one line at a time ((a), (b) first),
   each change with its host tests, until the fill shows. Record the
   photos in the note.
