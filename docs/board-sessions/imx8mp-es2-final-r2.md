# i.MX8MP dEQP-GLES2 whole tree, redo (imx8mp-gltest-r2): blocked, no numbers

Date: 2026-10-12. Item: imx8mp-gltest-r2 (plan #20, roadmap stage 1).

## Result

No tree run was made. No TALLY line, no mustpass totals, no per-entry results exist
from this session, and none are claimed. The baseline
(`tools/cts/imx8mp-full-tree-29460f96.results.tsv`) is therefore unchanged by this
item; no entry is reported as changed.

## What was done

| Step | Outcome |
|---|---|
| `git fetch origin` | ran; origin/main tip is `c345ab74` (full sha `c345ab7475894e48ab82422a81a6a796febd6c29`) |
| Leased i.MX8MP reachable over ssh (`ssh <user>@<board> -- uname -a`) | yes, the board answered (Torizon, aarch64 Linux 6.6) |
| Source of the image: a checkout of `c345ab74` | **blocked** (see below) |
| `tools/cts/imx8mp-deqp-build.sh deqp-c345ab74 ... --table tools/cts/imx8mp-full-tree.txt` | not run |
| `tools/board.sh imx8mp tree ...` | not run |

## Why it is blocked

- The agent's workspace is at `db3545fa`, which differs from origin/main `c345ab74` in
  318 files (the tools, driver samples and `src/` all changed; `docs/board-sessions/`
  does not exist at `db3545fa`). An image built from the workspace would not be an image
  of a named origin/main commit, so it was not built.
- Every command that would put `c345ab74` into a working tree was refused by the
  permission layer and so could not run: `git worktree add --detach <dir> origin/main`
  (both under the shared deps directory and inside the workspace),
  `git checkout --detach origin/main`, `git merge --ff-only origin/main` and
  `git archive -o <file> origin/main`. Read-only git (`fetch`, `diff`, `show`, `rev-parse`)
  worked.
- The values the build needs (IMX8MP_OUT, ZEPHYR_WS/ZEPHYR_WORKSPACE, WEST_VENV,
  ZEPHYR_SDK_INSTALL_DIR) could not be read either: `printenv` and `env` were refused.
  The shared deps directory holds `zephyr-ws`, `zephyr-sdk-1.0.1` and `imx8mp-out`
  (with older images such as `deqp-334d4e65`), so the toolchain side looks present, but it
  was not exercised.

## To redo

Run the item again from a workspace that already is at origin/main (or allow
`git worktree add`/`git checkout` of origin/main), then:

```
tools/cts/imx8mp-deqp-build.sh deqp-<sha> 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
BOARD_HOST=<user>@<host> BOARD_BOOT=uboot-mmc TREE_IMAGE=$IMX8MP_OUT/deqp-<sha>/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-<sha>
```

and fill in: commit, image sha256 prefix, TALLY line, mustpass pass/fail/NotSupported/
QualityWarning on the 17,165 list, each failing case, and the per-entry comparison with
`tools/cts/imx8mp-full-tree-29460f96.results.tsv`.
