<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-gltest-final, 2026-10-14, cd738b1a

## State of main

AM62P gl_test at `cd738b1a` (uboot-mmc, console on main_uart0): the default
list **50 of 50 pass**, and cases 198 (a2cconst), 197 (vtxgrid) and 171
(viewwin) **3 of 3 pass**; 0 failed, 0 resets. Record:
`docs/board-sessions/am62p-gltest-final.md`.

## Open work

AM62P gl_test from `origin/main` (plan #143 item am62p-gltest-final): done.
Together with `am62p-es2-final-r2.md` (dEQP-GLES2 whole tree, 0 failures),
the AM62P part of the stage-1 gl_test gap is closed.

## Next board session

`board.sh am62p tree` refuses when a dead run left a one-shot image in the
eMMC staging directory; `board.sh am62p disarm` clears it. A bridge capture
timeout (`verdict=scan_failed`, rc 4) is a transport problem, not a case
failure: rerun the row.
