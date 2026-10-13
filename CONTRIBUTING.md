<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Contributing to YakoGL

YakoGL is an OpenGL ES 2.0 + EGL library for embedded GPUs, with no OS GPU
driver. Start with `README.md`, then `docs/HANDOFF.md` ("How to work here"),
which also holds the process details.

## How a change reaches main

main moves only through pull requests: a topic branch, a pull request, the
CI, a review, then the merge. Never push main by hand.

1. Locally, run the rows of the gate your change reaches:
   `tools/check.sh --changed origin/main`. It maps each changed path to the
   parts it can break and skips the rest (a docs-only change runs two rows).
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
  about 45 minutes on a warm tree.
- It is green only at "N ok, 0 failed, 0 gap(s)". A step that could not run
  is a **gap**, and a gap makes the gate red. These need setup: `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (with `GLSLANG`, `GLSL2USC_CACHE`), the PowerVR
  firmware header, and the `aarch64-unknown-none-softfloat` Rust target
  (`rustup target add`). Without them you get a red run with named gaps;
  read which rows failed and which were gaps.
- `tools/check.sh --quick` (the PowerVR row and the AM62P archive only) is
  for the edit loop. `tools/check.sh --local <base>` runs the local set (13
  rows) when a change reaches everything. Neither is the landing gate: the
  CI is.

## Branches and commits

- Work on a short topic branch (for example `work/<topic>`). Bring main in
  with a merge (`git merge origin/main`); a pushed branch is never rebased.
- Commit subject: `<area>: <what changed, as a result>`. Lower case, no type
  prefix (not `feat:` or `fix:`). Example: `shaderc: the back ends' own
  programs come from our compiler`.
- The body explains the design. It names the records and commits the change
  rests on.
- Regenerate generated files in the same commit as the change that moves
  them (Mesa packers, the internal shader tables, bindgen output, register
  and core tables). The gate regenerates them and requires the committed
  bytes. Keep `third_party/` in its upstream form (`third_party/README.md`).

## Docs go with results

When a change produces a result, update the docs in the same pull request:

- a hand-over note `docs/handoff/<YYYYMMDD>-<topic>.md` saying what
  `docs/HANDOFF.md` should say after the change (state of main, open work,
  known defects; `docs/handoff/README.md` has the shape). Never edit
  `docs/HANDOFF.md` in a result's pull request: every result did, and two
  results in the same days always conflicted there. ccfarm's `handoff-fold`
  stage folds the notes in after the merge,
- `tools/cts/RESULTS.md` (the measurement log; cite the commit and record),
- `tools/cts/STATUS.md` (the summary),
- `README.md` status tables, where they apply.

## Board results

- Every number comes from a real GPU. Software rendering is out of scope.
- A board runs only an image built from a commit on `origin/main`. Run
  `tools/board.sh` from a detached checkout of main.
- One GL client per board. Check that the board is idle first.
- A failed board row is not always a regression. Compare it with the known
  defects in `docs/HANDOFF.md`.
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
