<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Contributing to YakoGL

YakoGL does not accept external pull requests. GitHub issues are welcome for
reproducible defects and other technical problems; security issues must be
reported through the process in `SECURITY.md`.

This file documents the maintainer development workflow. YakoGL is an OpenGL
ES 2.0 + EGL library for embedded GPUs, with no OS GPU driver. Start with
`README.md`, then this file. The open work, the known defects and the next
board sessions are the repository's GitHub issues.

## How a change reaches main

main moves only through pull requests: a topic branch, a pull request, the
CI, a review, then the merge. Never push main by hand.

1. Locally, run the rows of the gate your change reaches:
   `tools/check.sh --changed origin/main`. It maps each changed path to the
   parts it can break and skips the rest (a docs-only change runs two rows).
   The scope comes from `git diff <base> HEAD` plus `git status --porcelain`,
   so an untracked file counts: one outside the ignored paths (for example
   `build/`) that maps to no narrower scope widens it to "all" and runs every
   row. Keep the tree clean, and keep scratch files in `build/`; the first
   lines the script prints name each path that forces "every row".
2. Open the pull request. The CI is the gate: `.github/workflows/gate.yml`
   runs `tools/check.sh` on the pull request (the merge of the branch into
   main), again on main after the merge, and nightly, on self-hosted runners
   that have the environment below. The file's header says which rows each
   event runs.
3. The review reads the commit; it runs no check.
4. Merge when the CI is green and the review approves.

## The gate: `tools/check.sh`

- A red `cargo fmt --check` or a single warning is a defect.
- The full run checks formatting, clippy (no warnings), the tests in every
  feature configuration, the C check programs, the generated files, the
  product archives, the Python tools, and the Zephyr image builds. It takes
  35 to 50 minutes, depending on the host (measured: 36 minutes from a cold
  tree and 49 minutes on another host;
  `docs/release/readme-walkthrough.md`).
- It is green only at "N ok, 0 failed, 0 gap(s)". A step that could not run
  is a **gap**, and a gap makes the gate red. These need setup: `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (with `GLSLANG`, `GLSL2USC_CACHE`), the PowerVR
  firmware header, and the `aarch64-unknown-none-softfloat` Rust target
  (`rustup target add`). Without them you get a red run with named gaps;
  read which rows failed and which were gaps. `ZEPHYR_WS` may be given
  through a symbolic link: the gate resolves it to the real path first, and
  every Zephyr build rewrites its generated linker snippets' `#include` paths
  to absolute at the end of the configure (`zephyr/linker_snippets.cmake`), so
  the image rows build from a checkout at any depth (they once failed with
  `swi_tables.ld` not found in a worktree of another depth). A snippet that
  names a file no path resolves is left as it is and the configure warns with
  that line.
- `tools/check.sh --quick` (about 120 rows, fewer than the full run; 14
  minutes in one walk, 4 to 10 minutes on a warm tree in others) is for the
  edit loop. If `MESA_V3D_TREE` is set on your host to a Mesa tree without
  `build-v3d`, two rows fail; run `env -u MESA_V3D_TREE tools/check.sh --quick`
  (measured: 119 ok, 0 failed, 0 gaps, 236 s). `tools/check.sh --local <base>` runs the local set (13
  rows) when a change reaches everything. Neither is the landing gate: the
  CI is.

## Branches and commits

- Work on a short topic branch (for example `work/<topic>`). Bring main in
  by rebasing onto it (`git rebase origin/main`), then push your own topic
  branch with `git push --force-with-lease`. Never rewrite or force-push
  main or a branch someone else works on.
- Commit subject: `<area>: <what changed, as a result>`. Lower case, no type
  prefix (not `feat:` or `fix:`). Example: `shaderc: the back ends' own
  programs come from our compiler`.
- The body explains the design. It names the records and commits the change
  rests on.
- Cite only commits of this repository. Hashes in the tree from before the
  first public commit (for example `ab43a20c5:<path>` in a comment, or a
  commit in `tools/cts/RESULTS.md`) name commits of the non-public working
  repository the project was developed in; they are kept as they are and
  cannot be read here.
- Regenerate generated files in the same commit as the change that moves
  them (Mesa packers, the internal shader tables, bindgen output, register
  and core tables). The gate regenerates them and requires the committed
  bytes. Keep `third_party/` in its upstream form (`third_party/README.md`).

## Docs go with results

When a change produces a result, update the docs in the same pull request:

- the GitHub issues it touches: a comment with the commit and the record on
  each issue it advances, the issue closed when it is done, a new issue for
  each new defect or open item (open work is never kept in a file),
- `tools/cts/RESULTS.md` (current results only: a new run replaces its row,
  with the commit it ran; no run log),
- `tools/cts/STATUS.md` (the summary, current state only),
- `README.md` status tables, where they apply.

No per-session or investigation records go in the tree. What a session or
an investigation found goes into the commit message, the issue and
RESULTS.md; a design or a finding that code relies on goes into
`docs/notes/` (`docs/README.md`).

## Board results

- Every number comes from a real GPU. Software rendering is out of scope.
- A board runs only an image built from a commit on `origin/main`. Run
  `tools/board.sh` from a detached checkout of main.
- One GL client per board. Check that the board is idle first.
- A failed board row is not always a regression. Compare it with the known
  defects (the open issues labelled `bug`).
- Board addresses, UART paths, credentials and paths of local workspaces
  stay in the environment (`BOARD_HOST`, `BOARD_UART`, `ZEPHYR_WS`, ...).
  Never put them in a file, a record, a commit message or an issue. Run
  directories under `records/` are gitignored, and `tools/board.sh` redacts
  addresses in its copies.

## The PowerVR firmware

Never commit it, in any form. `rgx.fw` comes from TI's Processor SDK under
its own licence. Turn it into a header locally with `driver/tools/mkfwblob.sh`
(the header is `driver/src/fw/pvr_fw_blob.h`). Do not attach it to issues or
pull requests.

## Licensing

- YakoGL's own code is Apache-2.0 (`LICENSE`). Every source file carries an
  SPDX line, like the one at the top of this file.
- Code derived from Mesa or other outside sources names the upstream file,
  version and copyright holder in its header, with the combined identifier
  (for example `Apache-2.0 AND MIT`). Add the origin to `NOTICE`.
- Each licence used has its text in `LICENSES/`. Add a text when you add a
  new licence.
- Do not copy from GPL-only drivers (see `docs/source-form.md`). By
  contributing you agree that your work is licensed under Apache-2.0.

## Security

Report vulnerabilities as described in `SECURITY.md`, not in public issues.
