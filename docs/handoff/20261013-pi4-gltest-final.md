<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gltest-final, 2026-10-13, a63fcc5b

## Open work
- Pi 4 `gl_test` default list from `origin/main` is still not run. The
  `v3d_gl_test` image builds at `a63fcc5b` (`rc=0`, sha256
  `25548c99d50e4c2aacd3f77e01c987be3ea489affc6ee929ab176a3cad5f3d1c`), but the
  board session stopped: `BOARD_UART` (the serial bridge) was not in the lease
  and `ask_farm` was refused. See `docs/board-sessions/pi4-gltest-final.md`.

## Next board session
- Pi 4: give the session `BOARD_UART` (bridge `host:port`, see
  `docs/board-sessions/pi4-console-probe.md`) and run row GT of
  `tools/cts/common-tree.txt` with `tools/board.sh pi4`; then replace the
  record with the verdict lines and the per-case results.
