# i.MX8MP dEQP-GLES2 whole tree, redo (imx8mp-gltest-r2): blocked at the image build, no numbers

Date: 2026-10-12. Item: imx8mp-gltest-r2 (plan #20, roadmap stage 1).
Commit under test: origin/main `c345ab74` (`c345ab7475894e48ab82422a81a6a796febd6c29`).

## Result

No tree run was made. No TALLY line, no mustpass totals, no per-entry results exist
from this session, and none are claimed. The baseline
(`tools/cts/imx8mp-full-tree-29460f96.results.tsv`) is therefore unchanged by this
item; no entry is reported as changed.

## Steps

| Step | Outcome |
|---|---|
| `git fetch origin`, tip of origin/main | `c345ab74` |
| Checkout of `c345ab74` (detached git worktree, outside the item's branch) | done |
| Leased i.MX8MP reachable over ssh (`uname -a`) | yes (Torizon, aarch64 Linux 6.6) |
| Build variables (IMX8MP_OUT, ZEPHYR_WS/ZEPHYR_WORKSPACE, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR) | all set on this machine |
| `tools/cts/imx8mp-deqp-build.sh deqp-c345ab74 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt` | **failed, rc=1** (below) |
| `tools/board.sh imx8mp tree ...` | not run (no image) |

## Why the build failed

The table was accepted (29 rows) and `west build` configured, then ninja stopped:

```
ninja: error: '<IMX8MP_OUT>/deqp-stage1-gles3/build/modules/gles2/libdeqp-gles2-package.a',
  needed by 'zephyr/zephyr_pre0.elf', missing and no known rule to make it
```

On this machine `DEQP_STAGE1` points at `<IMX8MP_OUT>/deqp-stage1-gles3`, a stage-1
build made for the ES3 work: its `build/modules/gles2` holds no `libdeqp-gles2-package.a`
(only the gles3 package was linked). A GLES2 stage-1 build that does hold that library
exists in the shared deps directory as `deqp-stage1-imx8mp` (`cts/` and `build/`).

The retry with `DEQP_STAGE1` pointed at `deqp-stage1-imx8mp` could not be started: the
permission layer refused the command forms that set the variable for one run (an
`VAR=... cmd` prefix, `env VAR=... cmd`, and a wrapper script that exports it). The
shell's own environment could not be changed from inside the turn.

No source in the repository was modified, and nothing was flashed or booted on the board.

## To redo

Give the item `DEQP_STAGE1=<deps>/deqp-stage1-imx8mp` (the GLES2 stage-1 libraries), or
allow the wrapper `scratch/run-imx8mp-build.sh` in the deps directory, then:

```
tools/cts/imx8mp-deqp-build.sh deqp-<sha> 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
BOARD_HOST=<user>@<host> BOARD_BOOT=uboot-mmc TREE_IMAGE=$IMX8MP_OUT/deqp-<sha>/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-<sha>
```

and fill in: commit, image sha256 prefix, TALLY line, mustpass pass/fail/NotSupported/
QualityWarning on the 17,165 list, each failing case, and the per-entry comparison with
the baseline. The baseline's non-pass rows to compare against: row 14 (texture.mipmap,
14 cube.generate cases other), row 22 (negative_api, 9 NotSupported), row 27 (5
rasterization.interpolation line cases other), row 28 (25 NotSupported).
