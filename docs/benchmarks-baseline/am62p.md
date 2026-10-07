<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (before plan #2): NOT MEASURED

Status: **blocked before the board.** No image was built and nothing was run on
the Verdin AM62P. Every row below is empty on purpose; no host-only number is
presented as a board result.

- origin/main at the time: `16a53f8d` (read from the workspace's read-only copy
  of `origin/main`).
- `tools/cts/common-tree.txt` on that commit has four rows: GM (glmark2),
  SO (slint), SV (slint-view), GT (gl_test). It has **no cluster (CL) row**;
  the cluster-common item is merged only into `ccfarm/integration`.

## Why nothing ran

`tools/cts/cts-common.sh am62p build` needs these node-local variables, and this
worker has none of them: `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`,
`GLMARK2_SRC`, `SLINT_NOSTD_DIR`, `NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`,
`RUSTUP_TOOLCHAIN` (nightly). `NOTO_SANS_DIR` was not checked separately.
Building the cluster sample by hand needs `AGL_SLINT_CLUSTER_DIR` as well.

Also missing: `driver/src/fw/pvr_fw_blob.h` is not in the checkout (the origin
checkout is not reachable from this worker). It must never be committed.

## Rows

| Row | Image | Record | `tools/board.sh` verdict line |
|-----|-------|--------|-------------------------------|
| GM  | glmark2 | none | not run (no image) |
| SO  | slint, view off | none | not run (no image) |
| SV  | slint-view | none | not run (no image; the AM62P ignores the view option, so SV is the copy path again) |
| CL  | cluster | none | not run (no CL row on origin/main; needs `AGL_SLINT_CLUSTER_DIR`) |
| GT  | gl_test | none | not run (no image) |

| Measurement | Value |
|-------------|-------|
| glmark2 score, ran/failed/unsupported | not measured |
| terrain: GL error 0x502 lines, incomplete-texture lines | not measured |
| refract: "draw refused" count | not measured |
| Slint SO/SV verdicts and fingerprint | not measured |
| cluster probes and fps | not measured |
| gl_test pass count | not measured |
| GPU resets | not measured |

## To redo

Set the variables above (`ccfarm env set yakogl <NAME> <value>`), put
`pvr_fw_blob.h` into the worktree, then in a detached worktree of `origin/main`
run `tools/cts/cts-common.sh am62p build`, then `run`, then `tally`, with
`BOARD_BOOT`, `BOARD_HOST` and `BOARD_UART` taken from the farm's resources.
Build the cluster row by hand as RESULTS.md's 2026-10-05 "AM62P: Slint and
cluster" section describes, and run it with
`tools/board.sh am62p run <zephyr.bin> <rec> <label>`.
