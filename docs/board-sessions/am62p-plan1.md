<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P board session: plan #1 on the integration image

**Integration image, not a landing proof.** Source: the ccfarm integration
snapshot, HEAD `198e8a5de6da1b4452f8cc7c9c502b6cd757f1bf` (`198e8a5d`).
`4e9397c` and `c6aa6ba` are both ancestors of HEAD (checked with
`git merge-base --is-ancestor`). A later origin/main session repeats this
before landing.

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
- Hence `tools/cts/am62p-deqp-images.sh` could not be started, and no UART or
  board command was issued. The board and the serial console were not touched.

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

With the inputs in place, follow `docs/HANDOFF.md`, "Next board session", then
"AM62P", from a detached worktree of `198e8a5d` (no fetch):
`am62p-deqp-images.sh` on `am62p-full-tree.txt`, then `TREE_ROWS=0-6
board.sh am62p tree`, `tally`, and `deqp-uart-tally.py --mustpass
tools/cts/gles2-mustpass.txt`. Needs `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR`, `DEQP_WORK`, `BOARD_HOST`, `BOARD_UART`, and
`driver/src/fw/pvr_fw_blob.h` copied in (never committed).
