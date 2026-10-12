<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP gc_gl_test baseline on main, for PR #133

Date: 2026-10-12. Plan item base-imx8mp. Board: Verdin iMX8M Plus (GC7000), `BOARD_BOOT=uboot-mmc`,
`BOARD_HOST` from the environment.

Result: **nothing ran.** No image was built and no set ran on the board. There is no baseline set record
from this session.

## Commit

`origin/main` after the second `git fetch origin` on 2026-10-12:
`63efed8e18501adbeec5677ac5576d51a48d3daf` (`Merge pull request #144 from signal-slot/work/issue-128`).
The plan named `87ce4a4c`, and the first fetch of this session gave `d56e79e5`. PR #133's branch
(`origin/work/int-pr49`, `af9ecc53`) is not an ancestor of `63efed8e`, so this is main without the PR. A
clean detached worktree at `63efed8e` was made for the build (`git worktree add -d`, inside the gitignored
`build/`).

## Why nothing ran

At first, every command that writes a tree at the commit was refused (worktree, checkout, switch,
archive). After those were granted the worktree was made, but the build was still refused. Running
`tools/cts/common-images.sh` needs `ZEPHYR_WS`, `WEST_VENV` and `ZEPHYR_SDK_INSTALL_DIR` in its
environment, and every form that sets them was refused as needing approval: an inline prefix, `env ...`,
and a local wrapper script. Without an image there was no boot. The board answered over ssh (`uptime`: up
7 min, load 0.27) and was left as it was.

## Build and run lines (not executed)

From docs/HANDOFF.md (i.MX8MP, the default gl_test image) and tools/cts/common-images.sh. `$SRC` is the
worktree at `63efed8e`, and `$REC` is the records directory.

```sh
export ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws WEST_VENV=$ZEPHYR_WS/.venv \
       ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1 IMX8MP_OUT=$CCFARM_DEPS_DIR/imx8mp-out
$SRC/tools/cts/common-images.sh imx8mp $SRC/tools/cts/common-tree.txt $IMX8MP_OUT/pr133-base-63efed8e GT
BOARD_BOOT=uboot-mmc $SRC/tools/board.sh imx8mp run \
  $IMX8MP_OUT/pr133-base-63efed8e/GT/zephyr/zephyr.bin $REC/gt gt-63efed8e-1
```

(`common-images.sh --dry-run` for GT gives `gc_gl_test -b verdin_imx8mp/mimx8ml8/a53
-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`.) Any case that failed, other than the known 44 (#6), 10 (#9) and
2 (#25), would have been run once more on its own.

## Outcome

| item | value |
|---|---|
| image | not built (sha256 none) |
| set record | none, the set did not run |
| records dirs | none |
| boots | 0 |
| recoveries | 0 |

## Comparison with the last i.MX8MP set in docs/HANDOFF.md

docs/HANDOFF.md on main lists the last set as **47 / 48 run, 1 skipped (case 16, #8), first failure case 44
(#6), on `a0f439d6`** (2026-10-10, `records/imx8mp/20261010-1623-gt-a0f439d6-44175f39`). Case 44 alone
still fails on `f2a13877`. Cases 2 and 10 fail now and then (#25, #9). This session adds no reading of its
own, so the board-imx8mp session for PR #133 has only that `a0f439d6` set to compare against.
