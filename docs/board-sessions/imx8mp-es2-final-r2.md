# i.MX8MP dEQP-GLES2 whole tree, redo (imx8mp-gltest-r2): image built, board tree run not started

Date: 2026-10-12. Item: imx8mp-gltest-r2 (plan #20, roadmap stage 1).
Commit: origin/main `c345ab74` (`c345ab7475894e48ab82422a81a6a796febd6c29`).

## Result

The image was built from `c345ab74`; the board tree run was **not** made. No TALLY line,
no mustpass totals and no per-entry results exist from this session, and none are
claimed. No entry of `tools/cts/imx8mp-full-tree-29460f96.results.tsv` is reported as
changed.

| Step | Outcome |
|---|---|
| `git fetch origin`, tip of origin/main | `c345ab74` |
| Detached worktree of `c345ab74` (outside the item's branch) | done |
| `tools/cts/imx8mp-deqp-build.sh deqp-c345ab74 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt` | rc=0, 29 table rows, `CONFIG_DEQP_REPORT_ES2=y`, conformance-image check ran |
| Image `deqp-c345ab74/zephyr/zephyr.bin` sha256 | prefix `c4d409ab8515` |
| Leased i.MX8MP reachable over ssh | yes (Torizon, aarch64) |
| `tools/board.sh imx8mp tree ...` | not run: the command was refused by the permission layer (approval pending) |

## Notes on the build

- `DEQP_STAGE1` in the farm environment points at `<IMX8MP_OUT>/deqp-stage1-gles3`
  (ES3 stage 1, no `libdeqp-gles2-package.a`); the first build failed there
  (`ninja: ... libdeqp-gles2-package.a ... missing`). The image was built with
  `DEQP_STAGE1=<deps>/deqp-stage1-imx8mp`, the GLES2 stage-1 libraries.
- The conformance recipe is as the script leaves it: compiler, REPORT_ES2, no
  precompiled table.

## To finish

```
BOARD_HOST=<user>@<host> BOARD_BOOT=uboot-mmc TREE_IMAGE=$IMX8MP_OUT/deqp-c345ab74/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt records/imx8mp/deqp-c345ab74
```

then fill in the TALLY line, mustpass pass/fail/NotSupported/QualityWarning on the
17,165 list, each failing or NotSupported case, and the per-entry comparison. Baseline
non-pass rows: 14 (14 texture.mipmap.cube.generate cases), 22 (9 NotSupported),
27 (5 rasterization.interpolation line cases), 28 (25 NotSupported).
