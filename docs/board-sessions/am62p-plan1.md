<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P board session: plan #1 on the integration image

**Integration image, not a landing proof.** Source: the ccfarm integration
snapshot, HEAD `198e8a5de6da1b4452f8cc7c9c502b6cd757f1bf` (`198e8a5d`).
`4e9397c` and `c6aa6ba` are both ancestors of HEAD (checked with
`git merge-base --is-ancestor`). A later origin/main session repeats this
before landing.

<<<<<<< HEAD
## Result: nothing ran

No image was built and no board record exists. This session's shell was
restricted to the workspace: every command outside it was refused for
approval, and the approval was not available in an unattended run. In detail:

- `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and `DEQP_WORK` (the
  dEQP stage 1) have no value on this machine, and the directories that could
  hold them (the home directory, the ccfarm dependency cache) could not be
  listed. `.inputs/` from baseline2-am62p is not in this workspace.
- `printenv`/`env`, `ssh torizon@verdin-am62p` (idle check), `cp` of
  `pvr_fw_blob.h` from the origin checkout and `mcp__ccfarm__ask_farm` were
  all refused.
- Retry after the commands were allowed: `ask_farm` named the node's Zephyr
  workspace, venv and SDK directories (not `DEQP_WORK`), but they are outside
  this session's allowed directories and cannot be listed or used. The
  idle-check `ssh` to the board answered `Permission denied
  (publickey,password)`: no working login from this worker. The serial bridge
  is connected (no bytes read).
- Hence `tools/cts/am62p-deqp-images.sh` could not be started, and no UART or
  board command was issued. The board and the serial console were not touched.
=======
## Result: images 1 to 7 ran on the AM62P board; 10,092 pass, 9 fail

All seven shaders images (table rows 1 to 7, `TREE_ROWS=0-6`) built at
`b83216d4` and ran to `verdict=done` through `tools/board.sh am62p tree`
(boot `uboot-mmc`, serial bridge `$BOARD_UART`). Every case in every
image got a result. Six images match the `d60b9d76` baseline exactly. Image 1
has **9 new Fail cases** (`shaders.invariance.*.common_subexpression_1/2/3`
in all three precisions) and one expected flip: `fragdata.invalid_assign_to_1`
moved from NotSupported to **Pass**, confirming the prediction in
`docs/es2-cts-failures.md` ("expected pass, awaiting board"). The 9 failures
are a regression against `d60b9d76` (they passed there) and are not in
`docs/es2-cts-failures.md`.

## How it was run

Inputs on this node (Lepton), all under the ccfarm dependency cache
`$CCFARM_DEPS_DIR`. Board address, UART bridge and workspace paths are
environment only (`.ccfarm/CYCLE.md`, "Secrets and machine facts"); they are
named here by their variables, never by their values:

- `ZEPHYR_WS=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b/zephyr-ws`,
  `WEST_VENV=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b/venv`,
  `ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b/sdk/zephyr-sdk-1.0.1`,
  `CTS=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b/cts-src` (shared with the Pi 4 stage 1).
- Stage 1 for this board: `WORK=$CCFARM_DEPS_DIR/deqp-stage1-am62p
  BOARD=verdin_am62p/am62p54/a53 tools/cts/cts-build-zephyr.sh` (rc=0).
  It needs a real `cargo`/`rustc` first on PATH (`$CARGO_HOME/bin`);
  the kache cargo shim has no rustc next to it (same finding as baseline2).
- `driver/src/fw/pvr_fw_blob.h` regenerated from the board's
  `/lib/firmware/rgx.fw.36.53.104.796` (fetched with
  `ssh $BOARD_HOST cat ... > .inputs/rgx.fw`, then the
  `driver/tools/mkfwblob.sh` format). Never committed.
- Images: `TABLE=$CCFARM_DEPS_DIR/plan1-am62p-images/am62p-shaders7.txt
  tools/cts/am62p-deqp-images.sh <that table> b83216d4` — the table is rows
  1 to 7 of `tools/cts/am62p-full-tree.txt`. All seven `img<N> rc=0`
  (`build.out`). The script still exits 1 after the last unlabelled row (a
  `set -e` quirk in its final labelled-table test); every image built.
- Board: `BOARD_HOST` and `BOARD_UART` from the environment (values never
  written here), `BOARD_BOOT=uboot-mmc
  TREE_IMAGE_DIR=<the images dir above> TREE_TAG=b83216d4
  TREE_ROWS=0-6 tools/board.sh am62p tree tools/cts/am62p-full-tree.txt
  records/plan1-am62p/full`, then `tools/board.sh am62p tally
  records/plan1-am62p/full`. The tree is resumable: rows with an `INDEX.txt`
  are skipped, so an interrupted row re-runs alone.
>>>>>>> 2316e085a2ac6ff5bc2961d33ee0fb82de507ae8

## Records and tallies

| image | record | pass | fail | NotSupported |
|---|---|---|---|---|
| 1 to 7 (`TREE_ROWS=0-6`) | none | not run | not run | not run |
| 8 to 17, ms4 | none | not run | not run | not run |

## Comparison with d60b9d76 (17,141 pass / 0 fail / 19 NotSupported / 5 warnings)

Not possible: no run. `fragdata.invalid_assign_to_1`: not measured (expected
Pass, 17,142 / 18 NotSupported overall, per HANDOFF). No case was observed to
move from Pass.

## To run it

<<<<<<< HEAD
With the inputs in place, follow `docs/HANDOFF.md`, "Next board session", then
"AM62P", from a detached worktree of `198e8a5d` (no fetch):
`am62p-deqp-images.sh` on `am62p-full-tree.txt`, then `TREE_ROWS=0-6
board.sh am62p tree`, `tally`, and `deqp-uart-tally.py --mustpass
tools/cts/gles2-mustpass.txt`. Needs `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR`, `DEQP_WORK`, `BOARD_HOST`, `BOARD_UART`, and
`driver/src/fw/pvr_fw_blob.h` copied in (never committed).
=======
```
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_1   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_2   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_3   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_1    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_2    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_3    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_1 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_2 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_3 Fail (Detected variance between two invariant values)
```

`common_subexpression_0` passes in all three precisions; the failure message
carries no numeric detail. These 9 names are not in `docs/es2-cts-failures.md`
(no `invariance` entry anywhere in it): a new AM62P failure. The test computes
one common subexpression twice and requires bit-identical results, so a
compiler that fuses or reassociates one occurrence and not the other produces
exactly this. The prime suspects are the AM62P fusion/predication commits
listed in `docs/es2-cts-failures.md` section D with risk H — `6f96104` +
`18972dc` (fmad fusion), `39bd698` (varying product fused with its sum),
`81c53c6`, `afa12e9`, `ecd341d`, `f4c41d0`, `97ba03d`/`5f4d367` — plus
`c6aa6ba` (vertex attribute registers). A bisect of image 1 is the next step;
the host `compiler-regression-host` corpus did not catch this.

## Not run

Rows 8 to 17 (fbo, texture, vertex attribute, …) and the `ms4` multisample
image were not built or run: this item covers images 1 to 7 and the fragdata
row only. The whole-tree comparison against 17,141 / 0 / 19 / 5 needs those
rows too.

## Handoff:

- The 9 `invariance.common_subexpression_1/2/3` failures (all precisions) are
  the item's main finding: a board-proven regression of the pvr-online
  compiler since `d60b9d76`, first error line "Detected variance between two
  invariant values", records under
  `records/plan1-am62p/full/20261008-0634-img1-b83216d4-f4c169af/`. Bisect
  image 1 across the section-D H-risk commits; a dEQP-shaped host repro of
  "same subexpression, two occurrences, one fused" would be cheaper than
  board bisection.
- `fragdata.invalid_assign_to_1` Pass is board-confirmed; `es2-cts-failures.md`
  (owned by bench-recipes) can move that row from "awaiting board" to done.
- Stage 1, the seven images, and the records are on this node:
  `DEQP_WORK=$CCFARM_DEPS_DIR/deqp-stage1-am62p`,
  images in `$CCFARM_DEPS_DIR/plan1-am62p-images/img1..7-b83216d4` (labels
  `img<N>-b83216d4`, `build.out` present). A re-run of rows 8 to 17 needs no
  rebuild of rows 1 to 7: the tree skips rows with `INDEX.txt`.
- `am62p-deqp-images.sh` exits 1 after the last unlabelled row even when all
  images built (`set -e` on its final `[ "$labelled" = 1 ]` test) — worth a
  one-line fix by the tooling owner.
>>>>>>> 2316e085a2ac6ff5bc2961d33ee0fb82de507ae8
