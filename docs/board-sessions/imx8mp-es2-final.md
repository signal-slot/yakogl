<!-- SPDX-License-Identifier: Apache-2.0 -->
# i.MX8MP dEQP-GLES2 whole tree at the origin/main tip (imx8mp-es2-final)

Status: **BLOCKED. No tree was run. There are no board results in this record.**

## Commit

- origin/main after `git fetch origin`: `2a3a36fca96fdd67e771b33b2537cdf2b7e074c9`
  ("Merge pull request #138 from signal-slot/work/int-pr92").
- `5ad2fa0e` is an ancestor of it, so this is the "5ad2fa0e or newer" tip.

## What worked

- The board answers over ssh (Torizon, aarch64). Reachability is fine.

## Blocker

The build needs a worktree of origin/main outside the workspace.
The sandbox refused every way of making one:

- `git worktree add --detach <dir> origin/main` was refused for a path
  under /tmp and for a path under `.git/ccfarm/`. Each time it asked for approval.
- Variable expansion such as `$TMPDIR` and `$CCFARM_DEPS_DIR` in a command
  was also refused, so the environment values (IMX8MP_OUT, ZEPHYR_WS,
  HAL_NXP_DIR, DEQP_WORK, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR) could not be read.

Because of this, `tools/cts/imx8mp-deqp-build.sh`, `tools/board.sh imx8mp tree`
and `tools/board.sh imx8mp tally` were not run.

## Results

Nothing to compare with `tools/cts/imx8mp-full-tree-29460f96.results.tsv`.
Per-selector counts, the TOTAL, and the tree/tally verdict lines: none.
The baseline on 32778b57 (0 fail, 17,318 pass, 34 NotSupported,
19 QualityWarning) is **not** re-confirmed at 2a3a36fc.

## To finish

Re-run this item with permission for `git worktree add` under a scratch
directory and for `$TMPDIR` / `$CCFARM_DEPS_DIR` expansion, then follow the
procedure in the item text.
