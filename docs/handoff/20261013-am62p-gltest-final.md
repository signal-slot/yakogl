<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-gltest-final, 2026-10-14, cd738b1a

## Open work

The AM62P gl_test default list and cases 198, 197, 171 from `origin/main`
(`cd738b1a`) are **not yet run**. `docs/board-sessions/am62p-gltest-final.md`
records the blocker: no `BOARD_HOST` / `BOARD_UART` (port 33) in the session,
`ask_farm` and environment reads refused, and the 198,197,171 image build
refused. The GT image built (`rc=0`). The item stays open.

## Next board session

Give the session the board's ssh login and the port-33 bridge address, allow
the `west build` of `gl_test` with `-DCONFIG_GL_TEST_CASES="198,197,171"`,
and run `tools/board.sh am62p tree` with `BOARD_BOOT=uboot-mmc` for both
images (steps in `docs/board-sessions/am62p-es2-r1c.md`).
