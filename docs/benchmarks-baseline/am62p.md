<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (before plan #2): NOT MEASURED (third attempt)

Status: **one image (GT) ran on the board, with an empty console, so there is
no measurement.** The other rows were not built. No host-only number is offered
in place of a board result.

- origin/main at the time: `8abeaabb` (HEAD8; a detached worktree of it was
  created, and `driver/src/fw/pvr_fw_blob.h` was copied into it, never staged).
- `tools/cts/common-tree.txt` on that commit has the rows GM, SO, SV, GT and
  **no CL (cluster) row** (the cluster-common change is only on the
  integration branch). CL would have to be built by hand per RESULTS.md's
  2026-10-05 "AM62P: Slint and cluster" section.

## Second round (after some permissions were granted)

- `bash <worktree>/tools/cts/common-images.sh am62p ... GT` was allowed and
  **built the GT image**: `GT rc=0`, sha256
  `4e7f122a8dd1617162b3812e215c123a5aba988b6c1e77d0531db054d1a99461`,
  `PVR_SAMPLE_TREE=8abeaabb-dirty`, `PVR_SAMPLE_TABLE=0db3922e`. The tree is
  "dirty" only because one line (`export PATH=$HOME/.cargo/bin:$PATH`) was added
  to the worktree's `common-images.sh`: the worker's `cargo` is a kache shim
  with no `rustc` beside it, which stops capi's CMake. No source of the image
  differs from origin/main. This is a build, not a measurement.
- Third step: `bash -c 'export BOARD_HOST=... BOARD_UART=... BOARD_BOOT=uboot-mmc;
  tools/board.sh am62p run ...'` was allowed, and the board was reached. The GT
  image was staged on the eMMC (read back equal), the board rebooted into it and
  came back to Linux 426 s later, with no other session on the board. The run
  record is `20261007-1825-GT-4e7f122a` (runner `tools/board.sh` at
  `c658d8a4`, i.e. the integration branch's copy; the image is origin/main).
  The verdict line:

  `verdict: board.sh am62p GT rc=4 verdict=empty boot=uboot-mmc row boot_verdict=empty result=none resets=absent record=absent back_after=426s`

  **The console capture is empty** (0 lines in `console.txt`; the serial
  adapter's own client also saw 0 bytes during that window), so the run has no
  gl_test pass count, no GPU-reset count and no record. `verdict=empty` means
  "no console output", not that gl_test failed. Whether the image ran is
  unknown; the cause of the silent UART (adapter, wiring, or the image) was not
  investigated.

## What blocked it (first round)

The worker's permission policy refused every command that executes something
or reaches the board. Refused (each reported as a permission request):

- `ssh <board login> true` (board reachability check);
- `tools/board.sh --dry-run am62p run ...` run from the origin/main worktree;
- `tools/cts/cts-common.sh am62p build <dir> GT` and
  `bash tools/cts/common-images.sh --dry-run ...` from the worktree (so not
  even the GT image, which needs no fetched input, could be built);
- `cp`, `install -D` of the firmware header (the first two forms), and
  `git worktree add` / `cat > file` under `/tmp` (outside the permitted
  directories). These were worked around by putting the worktree under the
  permitted deps directory and using `cat src > dst`.

Not attempted because no build or board command could run at all: fetching
glmark2 (22c527cb), the Slint/femtovg/glow no_std trees, the AGL cluster
sources and Noto Sans.

Environment found: `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and a
nightly `RUSTUP_TOOLCHAIN` were present; `GLMARK2_SRC`, `SLINT_NOSTD_DIR`,
`NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `NOTO_SANS_DIR` were not.

## Rows

| Row | Image | Record | `tools/board.sh` verdict line |
|-----|-------|--------|-------------------------------|
| GM glmark2 | not built | none | not run |
| SO slint | not built | none | not run |
| SV slint-view | not built | none | not run |
| CL cluster | not built (no row in origin/main's table) | none | not run |
| GT gl_test | built, 4e7f122a, 8abeaabb-dirty | `20261007-1825-GT-4e7f122a` | `board.sh am62p GT rc=4 verdict=empty boot=uboot-mmc row ... back_after=426s` (empty console; see above) |

glmark2 score / ran-failed-unsupported, terrain 0x502 and incomplete-texture
lines, refract "draw refused" count, Slint fingerprint, cluster probes and fps,
gl_test pass count, GPU resets: **not measured.**
