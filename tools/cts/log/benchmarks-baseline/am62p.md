<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P benchmark baseline (before plan #2): NOT MEASURED

Status: **all five common-set images built; one (GT) ran on the board and its
console capture was empty, so there is no benchmark number.** No host-only
number is offered in place of a board result.

- origin/main: `8abeaabb` (HEAD8).
- The images are **not unmodified origin/main builds**. Every image reads
  `PVR_SAMPLE_TREE=8abeaabb-dirty` because the detached worktree's
  `tools/cts/common-images.sh` was edited (not committed anywhere):
  1. `export PATH=$HOME/.cargo/bin:$PATH` (the worker's `cargo` is a kache shim
     with no `rustc` beside it, which stops capi's CMake);
  2. a `cluster)` case for the CL row (see below).
  The firmware header was copied into the worktree (gitignored). No source file
  of the image differs from origin/main.
- origin/main's `tools/cts/common-tree.txt` has **no CL row**
  (GM, SO, SV, GT only). CL was built with the same script and a one-row table
  `cluster`, on `driver/samples/zephyr/cluster` (present on origin/main), with
  `AGL_SLINT_CLUSTER_DIR` (agl-slint-cluster clone), the Slint no_std tree and
  Noto Sans Regular/Light/ExtraLight (googlefonts/noto-fonts) as the sample's
  CMakeLists describes.
- Inputs were fetched into the project's deps directory, outside the repo:
  glmark2 at `22c527cb`, slint `1007c032` + `slint-nostd.patch`, femtovg 0.26.0
  and glow 0.18.0 from crates.io + their patches.

## Builds (`common-images.sh am62p`, `rc=0` each)

| Row | sha256 of zephyr.bin |
|-----|----------------------|
| GM glmark2 | `141d9eff487de9ba21beff017502f9acd7c7ef2303b4065de6055f5d8eae91b4` |
| SO slint | `4a4a507f3bfba1f05fed7f7f2d30170cb4d1bfecca61dfe62de7ef3c2befa463` |
| SV slint-view | same sha256 as SO (the AM62P board layer has no scan-out view) |
| CL cluster | `ff3f7ac2efeda8c0587135ad81d87ccf5a3426ea0df0cbb80b39e6752f2b7950` |
| GT gl_test | `4e7f122a8dd1617162b3812e215c123a5aba988b6c1e77d0531db054d1a99461` |

## Board runs

Run as `bash -c 'export BOARD_HOST=... BOARD_UART=... BOARD_BOOT=uboot-mmc;
tools/board.sh am62p run <zephyr.bin> <rec> <label>'` (the integration branch's
`tools/board.sh`, `c658d8a4`; the images are origin/main). The board was idle
(`who_sessions=0`), the image was staged on the eMMC and read back equal.

| Row | Record | `tools/board.sh` verdict line |
|-----|--------|-------------------------------|
| GT | `20261007-1825-GT-4e7f122a` | `verdict: board.sh am62p GT rc=4 verdict=empty boot=uboot-mmc row boot_verdict=empty result=none resets=absent record=absent back_after=426s` |
| GM | `20261007-1914-GM-141d9eff` | none: run interrupted (daemon restart) before the verdict; its console capture had only `busy: another client is connected` |
| SO | `20261007-1929-SO-4a4a507f` | none: same |
| SV | not run | interrupted before it started |
| CL | not run | interrupted before it started |

## Why there is no measurement

- GT's console capture was empty (0 lines); the board returned to Linux after
  426 s. `verdict=empty` means "no console output", not that gl_test failed.
- The UART path is a single-client serial bridge. In the GM and SO runs
  board.sh's capture was answered `busy: another client is connected`: the
  task's own serial console lease (the MCP client) holds the connection, and
  board.sh cannot capture while it does. In the GT run the capture was not
  refused but got no bytes, and the lease's client saw none either; writing to
  the console through it, and writing a line to Linux's console tty from the
  board, produced nothing on it. So the console is silent in the session as
  leased (adapter/wiring or the lease holding the bridge); this could not be
  fixed from the task.
- Therefore the glmark2 score and ran/failed/unsupported counts, terrain 0x502
  and incomplete-texture lines, refract "draw refused" count, Slint SO/SV
  verdict and fingerprint, cluster probes and fps, the gl_test pass count and
  GPU resets are **not measured**. The next attempt should run with the serial
  lease not connected (or board.sh pointed at a bridge no one else holds), and
  needs only the five built images above (rebuild from a clean origin/main
  worktree to get a non-dirty tree).
