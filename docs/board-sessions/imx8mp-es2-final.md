<!-- SPDX-License-Identifier: Apache-2.0 -->
# i.MX8MP dEQP-GLES2 whole tree at the origin/main tip (imx8mp-es2-final)

Status: **BLOCKED after the build. The tree was not run on the board.
There are no board results in this record.**

## Commit

- origin/main after `git fetch origin`: `2a3a36fca96fdd67e771b33b2537cdf2b7e074c9`
  ("Merge pull request #138 from signal-slot/work/int-pr92").
- `5ad2fa0e` is an ancestor of it, so this is the "5ad2fa0e or newer" tip.

## Done

- Built in a worktree of that commit:
  `tools/cts/imx8mp-deqp-build.sh deqp-2a3a36fc 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt`
  printed `29 table rows`, `rc=0`, `CONFIG_DEQP_REPORT_ES2=y` and
  `check-deqp-images: the dEQP image recipes build what conformance needs`.
  Image: `$IMX8MP_OUT/deqp-2a3a36fc/zephyr/zephyr.bin`.
- The board answers over ssh (Torizon, aarch64).

## Blocker

The sandbox refused to start `tools/board.sh imx8mp tree ...`
("This command requires approval"), both as a plain command with the
`BOARD_HOST=... BOARD_BOOT=uboot-mmc TREE_IMAGE=...` prefix and through `env`.
Nobody can approve it in this unattended run, so the tree was never started.
After `Bash(tools/board.sh:*)` was allowed, the plain `tools/board.sh imx8mp tally <dir>` ran,
but the `tree` command still needs the `BOARD_HOST=... BOARD_BOOT=... TREE_IMAGE=...`
environment prefix. The permission matcher treats that prefix as a separate
operation and asks for approval again. board.sh reads these only from the environment.
`tools/board.sh imx8mp tally` was not run either.

(Earlier in the session `git worktree add` under /tmp was also refused. A worktree
inside the allowed deps directory worked.)

## Results

Nothing to compare with `tools/cts/imx8mp-full-tree-29460f96.results.tsv`.
Per-selector counts, the TOTAL, and the tree/tally verdict lines: none.
The baseline on 32778b57 (0 fail, 17,318 pass, 34 NotSupported,
19 QualityWarning) is **not** re-confirmed at 2a3a36fc.

## To finish

Run, from a worktree of origin/main with the image above, with approval for
`tools/board.sh` (it flashes and boots the board):

```
BOARD_HOST=<leased ssh login> BOARD_BOOT=uboot-mmc \
TREE_IMAGE=$IMX8MP_OUT/deqp-2a3a36fc/zephyr/zephyr.bin \
tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-2a3a36fc
tools/board.sh imx8mp tally records/imx8mp/deqp-2a3a36fc
```
