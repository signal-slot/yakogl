<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gles3-baseline, 2026-10-14, cd738b1a

Blocked session: no Pi 4 result. Details in
`docs/board-sessions/pi4-gles3-baseline.md`.

## State of main

No change. The Pi 4 has no ES 3.0 gl_test result and no dEQP-GLES3 baseline
yet. On `cd738b1a`: the gles3 stage 1 for `rpi_4b` is in the deps cache
(`deqp-stage1-rpi_4b-gles3`), and the default-list `v3d_gl_test` image builds
(sha256 `70f6e1ce...`).

## Open work

- Not done: Pi 4 gl_test 196, 202, 203, 204, 205 and the first
  `tools/cts/pi4-gles3-tree.txt` image(s). Unchanged.
- Record for the recipe: on the Pi 4 the case symbols are
  `CONFIG_V3D_GL_TEST_CASE` (one case alone) and `CONFIG_V3D_GL_TEST_CASES`
  (a list); `CONFIG_GL_TEST_CASES` is the AM62P sample's. `common-images.sh`
  has no way to set them (only the sample default); a case option would
  save editing the Kconfig for each build.

## Known defects

- A real `tools/board.sh pi4 run|tree` needs `BOARD_HOST` (and `BOARD_UART`)
  in the environment; `--dry-run` does not, so a dry run passes in a session
  that cannot run the board. A farm Pi 4 session must export both before it
  starts, or its permission layer must allow an environment prefix on the
  repository scripts.

## Next board session

Start the Pi 4 session in a shell that already has `BOARD_HOST`,
`BOARD_UART`, `BOARD_HDMI_TIMINGS`, `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR`, `RUSTUP_TOOLCHAIN=nightly` and `DEQP_WORK` (the
gles3 stage 1). Then the commands in the "Commands for the re-run" section of
the board-session record, one image per gl_test case.
