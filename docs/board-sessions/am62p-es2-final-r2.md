<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES2 whole tree from origin/main (final-r2): blocked session

Status: **blocked before any build or device run. No numbers exist.**

## What was checked

- `git fetch origin` worked. The origin/main tip is `63efed8e`
  (Merge pull request #144, work/issue-128). This is SHA8 for the planned build.
- The workspace started at the integration tip `db3545fa`. It has no
  `docs/board-sessions/` directory, so the earlier records named in the task
  could not be read from the work tree. `git show origin/main:<path>` works
  for reading.
- The AM62P is reachable over ssh (the leased login answered `uname -a` with
  a Torizon aarch64 kernel 6.6.138).
- The UART bridge of the leased console adapter reports `connected`,
  lease mode exclusive, 0 bytes buffered.

## Blocking error

Every command that writes to the git work tree or creates a source tree was
refused by the harness with the same message:

```
This command requires approval
```

Refused commands (each tried once or twice, no change made):

- `git worktree add --detach build/wt origin/main`
- `git checkout --detach origin/main`
- `git clone --no-checkout . build/src`
- `git archive --format=tar --prefix=src/ -o build-src.tar origin/main`

Read-only git (`git fetch`, `git rev-parse`, `git show`, `git log`) and ssh
to the board were allowed. Without a checkout of origin/main there is no
source to build the 17 images and the ms4 image from, so
`tools/cts/am62p-deqp-images.sh` and `tools/board.sh am62p tree` were not run.
Building from the integration tip instead would not answer the question
(the images must come from origin/main), so it was not done.

## Not measured

- Per-image pass / fail / notsupported counts: none.
- Whole-tree TALLY and ms4 TALLY: none.
- Failing cases and first error lines: none.
- Comparison with `docs/es2-cts-failures.md`: not done.
- The 9 `shaders.invariance.*.common_subexpression_*` and the 45
  `texture.vertex.2d.*` regressions: not confirmed either way.
- gl_test cases 198, 197, 171: not run.

## To redo

Run this item again with permission for `git worktree add` / `git checkout`
(or `git archive`) in the workspace, and with `PVR_FW_BLOB`, `PCO_SHC`,
`GLSL2USC_CACHE` and the Zephyr workspace available. The board and console
were healthy at the time of this note.
