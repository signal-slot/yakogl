<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-common-int2, 2026-10-15, integration 3cc4b60a (plan 18 item v3d-null-cpu)

## State of main

- Pi 4 common set from the integration tip `3cc4b60a`: **built, not run**.
  `tools/cts/cts-common.sh pi4 build` gave rc=0 for GM, SO, SV, CL and GT
  (GM `5e09d0ee...`, SO `b13cdc5a...`, SV `495826d8...`, CL `83abd1fa...`,
  GT `c0dfb81c...`). GM carries the 96 MiB carve-out and the 24 MiB tiled-copy
  budget and links (footprint 250,490,880 bytes). No row ran: the machine
  holding the Pi 4 lease had no `BOARD_UART`, and the Pi 4's results come
  back only on its console (`docs/board-sessions/pi4-common-int2.md`).
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
  **do not close from this item**: no run came from it. The closest evidence
  is `docs/board-sessions/pi4-common-int.md` (main snapshot `86be8aeb`,
  2026-10-14). There, the carve-out boot line says 96 MiB, refract and terrain
  print no memory refusal, GL errors are 0 and the score is 183. Whoever
  folds this note decides whether that run closes #4 and #5, or whether a
  run from the integration tip is still needed.

## Next board session

- Pi 4: the common set from the integration tip is still to run. The images
  build cleanly at `3cc4b60a`. Run it where `BOARD_UART` (the Pi 4's console
  device or serial-bridge `host:port`) and `BOARD_HDMI_TIMINGS` are set,
  using the commands in "Commands to rerun" of
  `docs/board-sessions/pi4-common-int2.md`. Check the GM boot line for
  `carve-out 96 MiB`, check refract and terrain for memory and GL-error
  lines, compare the SO/SV crc16 with `c592` (fingerprint `0xe7113010`
  reported by an unmerged run, unchecked), and compare CL with `7b93` at
  about 139.5 fps and GT with 12 of 12.
