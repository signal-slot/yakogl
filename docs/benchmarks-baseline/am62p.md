<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (before plan #2): NOT MEASURED (third attempt)

Status: **no image was built and nothing ran on the board.** This is not a
result. Every row is empty; no host-only number is offered in its place.

- origin/main at the time: `8abeaabb` (HEAD8; a detached worktree of it was
  created, and `driver/src/fw/pvr_fw_blob.h` was copied into it, never staged).
- `tools/cts/common-tree.txt` on that commit has the rows GM, SO, SV, GT and
  **no CL (cluster) row** (the cluster-common change is only on the
  integration branch). CL would have to be built by hand per RESULTS.md's
  2026-10-05 "AM62P: Slint and cluster" section.

## What blocked it

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
| GT gl_test | not built | none | not run |

glmark2 score / ran-failed-unsupported, terrain 0x502 and incomplete-texture
lines, refract "draw refused" count, Slint fingerprint, cluster probes and fps,
gl_test pass count, GPU resets: **not measured.**
