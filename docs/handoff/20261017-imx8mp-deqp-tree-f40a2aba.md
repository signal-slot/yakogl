<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES2 tree at f40a2aba, 2026-10-17, f40a2aba

## State of main
i.MX8MP dEQP-GLES2 full tree re-run on main `f40a2aba` (`tools/cts/board-stage.sh
imx8mp deqp`, 29 selector rows, `records/imx8mp/deqp-f40a2aba/`): 17,371 cases
begun, 17,318 pass, 0 fail, 34 NotSupported (the accepted set), 19 other
(e14 14, e27 5), no row without a result. Same totals as the `e9901869a` run;
stage 1 stays met on all three boards. `tools/cts/RESULTS.md` and `STATUS.md`
name the new commit.

## Next board session
Row e27 stopped once on a full local disk quota (board.sh could not write its
`once` token) and was rerun from the same records directory; free space in the
deps cache (`imx8mp-out`) before a tree run.
