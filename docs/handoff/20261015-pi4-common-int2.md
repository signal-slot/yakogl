<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-common-int2, 2026-10-15, integration 3cc4b60a (plan 18 item v3d-null-cpu)

## State of main

- Pi 4 common set from the integration tip `3cc4b60a`: **built, started, no
  result**. `tools/cts/cts-common.sh pi4 build` gave rc=0 for GM, SO, SV, CL
  and GT, at `3cc4b60a` and again at `1e0c7566` (docs only on top, the same
  sources). GM carries the 96 MiB carve-out and the 24 MiB tiled-copy budget
  and links (footprint 250,490,880 bytes). The run's first row ended
  `board.sh pi4 GM rc=5 verdict=not_back boot=tryboot row boot_verdict=not_back result=none resets=0 record=absent back_after=?s`.
  The board hung in the firmware before the kernel, and the chain stopped,
  so SO, SV, CL and GT did not run (`docs/board-sessions/pi4-common-int2.md`).
- No board figure changes. The cells stay as they are:
  - `README.md` Status table, glmark2 score row: keep
    `| Raspberry Pi 4 | 183 (2026-10-04) |` unchanged.
  - `tools/cts/RESULTS.md`: no Pi 4 cell or row changes. Add no section for
    this session.
  - `tools/cts/STATUS.md`: no Pi 4 line changes (it has no Pi 4 benchmark
    line today; add none).
- When the session runs, the cells to write (fill the `<...>` from the
  verdict lines and the tally):
  - `README.md`: `| Raspberry Pi 4 | <score> (`<HEAD8>`, <date>) |`
  - `tools/cts/RESULTS.md`, a new section
    `## Pi 4 common set from the integration tip <HEAD8> (<date>)` with the
    five verdict lines, the common-tally output and the per-scene glmark2
    table of `docs/board-sessions/pi4-common-int2.md`.
  - `tools/cts/STATUS.md`, under 「ひとことで」:
    `- **Pi 4: 共通セット（<HEAD8>、<date>、tryboot、records/pi4/common-<HEAD8>）。** glmark2 は <score>（33 シーン、失敗 <n>、carve-out 96 MiB、refract と terrain のメモリ不足の行 <n>）、SO / SV / CL のプローブは <合格|不合格>、GT は <p> / <n>。全行でリセット <n>。`

## Open work

- #4 and #5 (the Pi 4 glmark2 refract and terrain on the 96 MiB carve-out)
  **do not close from this item**: GM never reached Zephyr. The closest
  evidence is still `docs/board-sessions/pi4-common-int.md` (main snapshot
  `86be8aeb`, 2026-10-14). There, the carve-out boot line says 96 MiB, refract
  and terrain print no memory refusal, GL errors are 0 and the score is 183.
  Whoever folds this note decides whether that run closes #4 and #5, or
  whether a run from the integration tip is still needed.

## Known defects

- Environment, not driver: `system76`'s node-local `BOARD_HDMI_TIMINGS` for
  this project is `1920x720`. The lab's value is the 17 fields
  `1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1`
  (`tools/cts/README.md`; the farm's other nodes have it). board.sh copies
  the value into `tryboot.txt` as `hdmi_timings=`, and the GM tryboot stopped
  in the firmware right after reading that file (last line `brfs: File read:
  210 bytes`, no kernel load). That is the likely cause, not a proven one.
  Fix it with `ccfarm env set yakogl BOARD_HDMI_TIMINGS '<the 17 fields>'` on
  that node.
- board.sh does not check the shape of `BOARD_HDMI_TIMINGS`: a value that is
  not 17 numbers goes into `tryboot.txt` as it is, and a boot that hangs in
  the firmware has no watchdog to get out. A check that refuses before the
  board is changed (rc=3) would have stopped this run. That is a follow-up,
  not done here.

## Next board session

- Pi 4, first: **power-cycle the board** (it does not answer ssh since the
  06:49 tryboot of 2026-10-15). Tryboot is one-shot, so it comes back in
  Linux. Then remove the leftover `/boot/firmware/yakogl-GM.bin` and put back
  the empty `tryboot.txt`; `tools/board.sh pi4 status` shows both.
- Then run the common set from the integration tip, with
  `BOARD_HDMI_TIMINGS` set to the 17 fields, `BOARD_UART` (the debug probe's
  serial bridge, `host:port`) and `BOARD_HOST`, using "Commands to rerun" in
  `docs/board-sessions/pi4-common-int2.md`. The images at
  `build/common/pi4-1e0c7566` (or a rebuild at the tip) are ready. Check the
  GM boot line for `carve-out 96 MiB`, check refract and terrain for memory
  and GL-error lines, compare the SO/SV crc16 with `c592` (fingerprint
  `0xe7113010` reported by an unmerged run, unchecked), and compare CL with
  `7b93` at about 139.5 fps and GT with 12 of 12. If GT fails 2, 10 or 16,
  rerun GT once.
