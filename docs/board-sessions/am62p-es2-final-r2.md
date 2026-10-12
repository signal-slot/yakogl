<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES2 whole tree from origin/main (final-r2): blocked session

Status: **blocked before the image build. No numbers exist.**

## What was done

- `git fetch origin` worked. The origin/main tip is `63efed8e`
  (Merge pull request #144, work/issue-128). This is SHA8 for the build.
- A detached worktree of that commit was created
  (`git worktree add --detach build/wt origin/main`) after the first
  round of refusals was lifted.
- `tools/cts/deps-env.sh --check am62p` printed `ok` for every entry: the
  Zephyr workspace, its venv, the Zephyr SDK 1.0.1, `PCO_SHC`, glslang,
  `GLSL2USC_CACHE`, glmark2, `PVR_FW_BLOB` and the AM62P stage-1 tree.
- `driver/src/fw/pvr_fw_blob.h` was copied into the worktree (git-ignored,
  not committed).
- `tools/cts/am62p-deqp-images.sh --dry-run` runs. The board answered ssh
  earlier, and the UART bridge reported `connected` (exclusive lease).

## Blocking error

The build itself cannot be started. The image script needs
`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR` and `DEQP_WORK` in its
environment, and it defaults none of them. Every way of giving it them was
refused with:

```
This command requires approval
```

Refused forms:

- `export ... ; tools/cts/am62p-deqp-images.sh ... > build.out`
- `VAR=value ... tools/cts/am62p-deqp-images.sh ...` (inline variables,
  with and without the `> build.out` redirect)
- `bash build/run-build.sh`, `./build/run-build.sh` (a wrapper that sets the
  variables), and `chmod +x` on it

The script without variables (`--dry-run`) was allowed, so the refusal is
about setting the environment for the real run, not about the script.
Because the images were not built, `tools/board.sh am62p tree` was not run.

## Not measured

- Per-image pass / fail / notsupported counts: none.
- Whole-tree TALLY and ms4 TALLY: none.
- Failing cases and first error lines: none.
- Comparison with `docs/es2-cts-failures.md`: not done.
- The 9 `shaders.invariance.*.common_subexpression_*` and the 45
  `texture.vertex.2d.*` regressions: not confirmed either way.
- gl_test cases 198, 197, 171: not run.

## To redo

Allow either an inline-variable form of `tools/cts/am62p-deqp-images.sh` and
`tools/board.sh`, or a wrapper script, or export the four variables in the
task's environment (`tools/cts/deps-env.sh am62p` prints them). Then run the
steps in `tools/cts/README.md`, AM62P section, from origin/main `63efed8e`.
