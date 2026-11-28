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

1. Locally, run only the tests you added or changed (`cargo test --test
   <name>` with the feature sets in that test's header) and `cargo fmt
   --check` on the files you touched. Do not run `tools/check.sh` locally,
   in any mode: the CI gate is the check. The one exception is a task whose
   stated purpose is a gate walk (an acceptance run of the gate itself).
2. Open the pull request. The CI is the gate: `.github/workflows/gate.yml`
   runs `tools/check.sh --no-clippy --changed <merge base>` on the pull
   request (the merge of the branch into main), and every row nightly on
   main's tip and when started by hand; main is not run on each merge. It
   runs on self-hosted runners that have the environment below. The file's
   header says which rows each event runs. (`.github/workflows/pages.yml` only publishes the
   website on a push to main that touches `website/`; it checks nothing.)
   A pull request's run leaves out the clippy rows (owner, 2026-11-13: they
   doubled the build of each feature set); the nightly run on main runs
   them and is clippy's gate. Run clippy on the crates you touched when it
   is quick, and fix a red nightly's clippy rows first (docs/ci.md, "The
   nightly").
   A pull request from a fork gets no gate: the runners are the project's
   own machines and a run executes the code it checks, so `.github/workflows/gate.yml` runs
   the gate only for a pull request from a branch of this repository. A
   fork's pull request shows `check` skipped and one check instead, `outside`,
   on a GitHub-hosted runner, which says so and fails on purpose (nothing
   checked the pull request). A maintainer who takes such a change reviews
   the diff, pushes the contributor's branch to this repository (for
   example as `work/<topic>`) and opens a pull request from it, or retargets
   an existing one; that pull request gets the gate on runners A, B and C
   and lands as described here (docs/ci.md, section 3.5).
3. The review reads the commit; it runs no check.
4. Merge when the CI is green and the review approves.

## The gate: `tools/check.sh`

This section describes what the CI runs, how to read its log, and what a
host needs for a gate walk. It is not a list of commands to run before a
pull request (step 1 above).

- A red `cargo fmt --check` or a single warning is a defect.
- The options (the usage text at the top of `tools/check.sh`):
  - no option: every row. This is the nightly run and a run started by hand.
  - `--changed <base>`: the rows the diff from `<base>` can reach. A pull
    request's run is `--no-clippy --changed <merge base>`.
  - `--no-clippy`: leave out every clippy row; the summary counts them as
    "left to the nightly", never as gaps. `YAKOGL_CHECK_NO_CLIPPY=1` does
    the same.
  - `--local <base>`: like `--changed`, but a diff that reaches everything
    runs the local set only (what `tools/land.sh` runs).
  - `--quick`: most of the full plan's rows, for a tight edit loop; it
    leaves out the other feature configurations, the Zephyr images and the
    glmark2 corpus.
  - `--jobs N`: up to N rows at once (`YAKOGL_CHECK_JOBS`).
  The CI uses only `--no-clippy --changed` and the bare run; neither
  `--quick` nor `--local` is the landing gate.
- The full run checks formatting, clippy (no warnings), the tests in every
  feature configuration, the C check programs, the generated files, the
  product archives, the Python tools, and the Zephyr image builds. On a CI
  runner (8 cores, 3 compiler slots) a run that reaches every scope takes
  about 2 h 45 min (`.github/workflows/gate.yml`, comment above
  `timeout-minutes`); `--quick` takes from tens of minutes to over an hour
  depending on the host and the tree (cold or warm). No file keeps the row
  counts: `YAKOGL_CHECK_PLAN=1 tools/check.sh [--quick]` lists the rows and
  runs none, and its summary line gives the counts. A docs-only change
  reaches only a few rows.
- A self-test (a tracked file named `*selftest.sh` or `*selftest.py`) needs
  no row of its own: the gate gives one to each that no row names
  (`tools/check_selftests.py` lists them), under the scope of its path. The
  row runs it with no argument and a `TMPDIR` of its own. A self-test that
  takes an argument needs a row that names it.
- It is green only at "N ok, 0 failed, 0 gap(s)". A step that could not run
  is a **gap**, and a gap makes the gate red. These need setup: `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (with `GLSLANG`, `GLSL2USC_CACHE`), the PowerVR
  firmware header, `lz4` (the am62p second-stage row), `qemu-system-aarch64`
  (the uboot-mp2 row) and the `aarch64-unknown-none-softfloat` Rust target
  (`rustup target add`). Without them you get a red run with named gaps;
  read which rows failed and which were gaps. The gate reads these
  variables from the environment only, so a stale value (a moved checkout)
  is a gap too, and a stale `DEQP_GLES3_TEST_DIR` fails the `shaderc-online`
  tests. `tools/cts/deps-env.sh --check` says which entries the dependency
  layout has, `export $(tools/cts/deps-env.sh)` sets them, and
  `tools/cts/deps-env.sh --build <NAME>...` builds the ones it lacks
  (`MALI_CSFFW`, `DEQP_GLES3_TEST_DIR`, `DEQP_GLES2_TEST_DIR`,
  `DEQP_ES2_PROGRAMS`, `PCO_SPM_LOAD_CAPTURE`, `HAL_NXP_DIR`, `LZ4`,
  `MP2_QEMU`; the two dEQP-GLES2 entries feed the V3D 7.1 corpus rows, a
  gap without them); run one build at a time. Then, after
  the export and the PowerVR header copy (below),
  `tools/setup-gate-host.sh --check` says whether the host has every tool
  and file the gate needs (`--install` installs the packages).
- Host traps that `tools/setup-gate-host.sh --check` reports: a `CC_<target>`
  variable naming a tool that is not there (for example
  `CC_aarch64_unknown_none`) fails the aarch64 and `no Mesa reference` rows
  from a cold tree (`unset` it); an `LC_ALL` or `LANG` naming a locale the
  host has not installed fails the rows that run `tools/` scripts
  (`export LC_ALL=C`).
- Run the gate from the checkout's canonical path (`cd "$(pwd -P)"` first),
  not through a symbolic link: from a linked path the two artefact rows "no
  table" and "no test knob in the artefact" fail
  (`docs/notes/gate-symlinked-path.md`). `ZEPHYR_WS` itself may be a link.
- If `MESA_V3D_TREE` is set on your host to a Mesa tree without `build-v3d`,
  the three v3d rows (tex_check, cl_golden, job_check) become one `gap`
  naming `MESA_V3D_TREE`, and a gap keeps the run red: unset it
  (`env -u MESA_V3D_TREE tools/check.sh --quick`) for the vendored
  reference build, or point it at a built tree (`tools/v3d/README.md`).
- A walk of these commands in a fresh clone is recorded in GitHub issue
  1606 and `docs/release/readme-walkthrough.md`.

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
pull requests. The full gate builds the am62p images only with the header
in that place; a host whose dependency layout holds it (`PVR_FW_BLOB`)
copies it there first, as the CI does:
`mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/`.

The Mali CSF firmware (`mali_csffw.bin`, Arm's; the NanoPC-T6's Armbian file
is linux-firmware's) is never committed either; `driver/tools/mkmalifw.sh`
turns it into `driver/src/fw/mali_csffw_blob.h`. The gate reads it at
`MALI_CSFFW`; `tools/cts/deps-env.sh --build MALI_CSFFW` fetches the file
pinned in `tools/cts/deps.toml`. Without it the firmware row is a gap.

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
