<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP gc_gl_test baseline on main, for PR #133

Date: 2026-10-12. Plan item base-imx8mp. Board: Verdin iMX8M Plus (GC7000), `BOARD_BOOT=uboot-mmc`,
`BOARD_HOST` from the environment.

Result: **nothing ran.** No image was built and no set ran on the board. There is no baseline set record
from this session.

## Commit

`origin/main` after `git fetch origin` on 2026-10-12:
`d56e79e532031b6c75d05b453b7fc71e8b71b190` (`Merge pull request #182 from signal-slot/work/issue-8`).
That is newer than the `87ce4a4c` named when the plan was made. The session's working tree was at
`db3545fa`, an ancestor of `d56e79e5` that differs from it in 325 files (the gc_gl_test sample,
`gl_test.c`, `tools/board.sh`, `tools/cts/imx8mp-gltest-build.sh` among them). So an image built from
that tree would not be a main baseline, and none was built.

## Why nothing ran

The build needs a source tree at `d56e79e5`. The session's permission rules refused every command that
would have written one: `git worktree add` (four forms), `git checkout --detach`, `git switch --detach`,
`git --work-tree=... checkout`, `git archive` (with `-o` and with a redirect), and a local script that ran
`git archive`. Read-only git (`fetch`, `log`, `diff`, `show`) and the repository's scripts were allowed.
Without a tree at the commit there was no build, so the board was not booted. The board answered over ssh
(`uptime`: up 7 min, load 0.27) and was left as it was.

## Build and run lines (not executed)

These are the lines the session would have run, from docs/HANDOFF.md (i.MX8MP, step 2) and
tools/cts/common-images.sh. `$SRC` is a tree at `d56e79e5`, and `$REC` is the records directory.

```sh
export ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws WEST_VENV=$ZEPHYR_WS/.venv \
       ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1 IMX8MP_OUT=$CCFARM_DEPS_DIR/imx8mp-out
$SRC/tools/cts/common-images.sh imx8mp $SRC/tools/cts/common-tree.txt $IMX8MP_OUT/pr133-base-d56e79e5 GT
BOARD_BOOT=uboot-mmc $SRC/tools/board.sh imx8mp run \
  $IMX8MP_OUT/pr133-base-d56e79e5/GT/zephyr/zephyr.bin $REC/gt gt-d56e79e5-1
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

On `d56e79e5`, docs/HANDOFF.md lists the last set as **47 / 48 run, 1 skipped (case 16, #8), first failure
case 44 (#6), on `a0f439d6`** (2026-10-10, `records/imx8mp/20261010-1623-gt-a0f439d6-44175f39`). Case 44
alone still fails on `f2a13877`. Cases 2 and 10 fail now and then (#25, #9). This session adds no reading
of its own, so the board-imx8mp session for PR #133 has only that `a0f439d6` set to compare against.
