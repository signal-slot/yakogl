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
   runs `tools/check.sh --changed <merge base>` on the pull request (the
   merge of the branch into main), and every row nightly on main's tip and
   when started by hand; main is not run on each merge. It runs on
   self-hosted runners that have the environment below. The file's header
   says which rows each event runs.
3. The review reads the commit; it runs no check.
4. Merge when the CI is green and the review approves.

## The gate: `tools/check.sh`

This section describes what the CI runs, how to read its log, and what a
host needs for a gate walk. It is not a list of commands to run before a
pull request (step 1 above).

- A red `cargo fmt --check` or a single warning is a defect.
- The full run checks formatting, clippy (no warnings), the tests in every
  feature configuration, the C check programs, the generated files, the
  product archives, the Python tools, and the Zephyr image builds. It takes
  35 to 80 minutes, depending on the host (measured: 36 minutes from a cold
  tree and 49 minutes on another host, `docs/release/readme-walkthrough.md`;
  76 minutes on 4 cores with two compiler slots, after a `--quick` in the
  same clone of `e3e8d777`, 2026-11-08).
- It is green only at "N ok, 0 failed, 0 gap(s)". A step that could not run
  is a **gap**, and a gap makes the gate red. These need setup: `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (with `GLSLANG`, `GLSL2USC_CACHE`), the PowerVR
  firmware header, and the `aarch64-unknown-none-softfloat` Rust target
  (`rustup target add`). Without them you get a red run with named gaps;
  read which rows failed and which were gaps. The gate reads `ZEPHYR_WS`,
  `HAL_NXP_DIR` and `PCO_SHC` from the environment only, so a stale value
  (a moved checkout) is a gap too, and a stale `DEQP_GLES3_TEST_DIR` fails
  the `shaderc-online` tests: `tools/cts/deps-env.sh --check` says which
  entries this host's dependency layout has, and
  `export $(tools/cts/deps-env.sh)` sets them all from it before the run.
  On a new host, build the entries the layout lacks, first, as one command:
  `tools/cts/deps-env.sh --build MALI_CSFFW DEQP_GLES3_TEST_DIR PCO_SPM_LOAD_CAPTURE HAL_NXP_DIR LZ4`
  (the Mali firmware, "The PowerVR firmware" below; the dEQP GLES3 shader
  corpus, which the `shaderc-online` tests and the corpus test read; and
  `capture_spm_load` beside `pco_shc`, which the `spm_load` regenerate row
  compares against, a gap without it; only the full plan reads that row; the
  `hal_nxp` checkout, which the imx8mp `regs.rs` regenerate row reads, a
  gap without it; and the `lz4` program of the am62p second-stage vectors,
  built from source without root).
  On the `ed15e1be` walk (2026-11-09) the firmware and the corpus took
  seconds and the capture program 85 s (97 s on a retry here after a reset
  download); run the build once at a time, two at once share `PCO_SHC.src` and
  the second dies with `Text file busy`. Then, after the export and the
  PowerVR header copy, as the CI does, `tools/setup-gate-host.sh --check`
  says whether this host has every tool and file the gate needs (it reads
  the same variables, so run before the export it reports a stale one as
  missing; `tools/setup-gate-host.sh --install` installs the packages, `lz4`
  among them). The `lz4` command is a prerequisite of `--quick`: without it
  the `am62p second stage` row is a gap and the run is red (`apt install lz4`,
  Gentoo `app-arch/lz4`). A `CC_<target>` variable in the shell that names a
  tool that is not there (for example `CC_aarch64_unknown_none`) fails the
  aarch64 and `no Mesa reference` rows from a cold tree, in the build scripts
  of the crates; `--check` reports it and `unset` fixes it. `ZEPHYR_WS` may be given
  through a symbolic link: the gate resolves it to the real path first, and
  every Zephyr build rewrites its generated linker snippets' `#include` paths
  to absolute at the end of the configure (`zephyr/linker_snippets.cmake`), so
  the image rows build from a checkout at any depth (they once failed with
  `swi_tables.ld` not found in a worktree of another depth). A snippet that
  names a file no path resolves is left as it is and the configure warns with
  that line. The checkout itself is not resolved: run the gate from its
  canonical path (`cd "$(pwd -P)"` first), not through a symbolic link, until
  that is fixed; from a linked path the two artefact rows "no table" and "no test knob in
  the artefact" fail (`dep_info_within` compares the physical dep-info paths
  with the logical `$PWD`; `cd -P` at `tools/check.sh:107` is the fix, not
  applied, `docs/notes/gate-symlinked-path.md`).
- `tools/check.sh --quick` runs most of the full plan's rows (the CI does
  not use it). `YAKOGL_CHECK_PLAN=1 tools/check.sh --quick` and
  `YAKOGL_CHECK_PLAN=1 tools/check.sh` list the rows and run none; their
  summary lines give the counts, ok plus gaps (a row whose tool or file the
  host lacks is listed as a gap). No file keeps the count: a row added by
  one pull request edits no number that another one edits too. The runs
  below were made with 177 rows in `--quick`. Measured on
  2026-11-10 from a fresh clone of `0f020431` at a canonical path on a 4 core
  host with two compiler slots, the layout exported, `lz4` on PATH and the
  stale `CC_aarch64_unknown_none` unset: "177 ok, 0 failed, 0 gap(s)" in
  4603 s from a cold tree and 1710 s warm (at `28611536`, 4703 s cold). From
  a fresh clone of `18353e05` the same: "177 ok, 0 failed, 0 gap(s)" in
  971 s. With the variable and the missing `lz4` as the host had them, the
  host check exits 1 and the run is red (173 ok, 4 failed at `18353e05`).
  At `8a5da05e` a run read "175 ok, 1 failed, 1 gap(s)" in 1379 s: the
  failed row was `test yakogl-mali-panthor`
  (`gltest_guard::the_verdict_line_is_out_before_the_next_case_starts`, a
  timing test; it passed in the walks above) and the gap `lz4`. Earlier, with
  fewer rows: "176 ok, 0 failed, 0 gap(s)" in 2950 s at `ed15e1be`
  (2026-11-09), "173 ok, 0 failed,
  0 gap(s)" in 2334 s with two compiler slots (4 cores) at `e3e8d777`
  (2026-11-08), "170 ok, 0 failed, 0 gap(s)" in 2309 s from a fresh clone of
  `14e93e64` (2026-11-06), and "146 ok, 0 failed, 0 gap(s)" in 2747 s at
  `8198a56f` (2026-10-25). It takes minutes on a warm tree (190 s on four lanes,
  `tools/check.sh` header, 2026-10-25) and up to an hour from a cold one (59
  minutes with two compiler slots, `CLAUDE.md`). If `MESA_V3D_TREE` is set on
  your host to a Mesa tree without `build-v3d`, the three v3d rows (tex_check,
  cl_golden, job_check) become one `gap` naming `MESA_V3D_TREE`, and a gap
  keeps the run red: unset it (`env -u MESA_V3D_TREE tools/check.sh --quick`)
  for the vendored reference build, or point it at a built tree
  (`tools/v3d/README.md`). `tools/check.sh --local <base>` runs the local set
  (about twenty rows, `tools/check_scope_selftest.sh`) when a change reaches
  everything. Neither is the landing gate: the CI is.
- The commands above, run in this order in a fresh clone (over HTTPS) at a
  canonical path of `31e07d90` (2026-11-11), the layout exported with
  `export $(tools/cts/deps-env.sh)` and `lz4` built from source on PATH
  (node with 4 cores, two compiler slots, 7 GB; rustc 1.99.1, cargo 1.99.1,
  clippy 0.1.99, a stable toolchain), exit code and wall time:

  | command | exit | wall |
  | --- | --- | --- |
  | `tools/cts/deps-env.sh --check`, layout missing entries | 1 | 1.1 s |
  | `tools/cts/deps-env.sh --build MALI_CSFFW DEQP_GLES3_TEST_DIR PCO_SPM_LOAD_CAPTURE HAL_NXP_DIR` | 0 | 286.5 s |
  | the PowerVR header copy | 0 | 0.1 s |
  | `tools/setup-gate-host.sh --check` | 0 | 12.7 s |
  | `tools/check.sh --changed origin/main` (no change) | 0 | 56.3 s |
  | `tools/check.sh --quick`, cold tree | stopped at the 2 h limit, no summary line | about 7200 s |
  | `tools/check.sh --quick`, rerun, warm tree | 0 | 3416.7 s |
  | `tools/check.sh --local origin/main` (no change) | 0 | 12.9 s |
  | `tools/check_scope_selftest.sh` (137 checks, 0 failed) | 0 | 56.7 s |

  The `--quick` summary line: "check.sh: 177 ok, 0 failed, 0 gap(s) (a gap was
  not run: it is not a pass), 0 skipped (every row), 0 not run (passed on
  main) in 3416s". The layout there lacked `HAL_NXP_DIR` too, so the build
  named it. This host has no cold-tree time at that commit (more than two
  hours); the last cold one is 4603 s at `0f020431` (2026-11-10, rustc
  1.101.0-nightly, the same step list without `HAL_NXP_DIR`, where the host
  check first exited 1 for a missing `lz4` and a stale
  `CC_aarch64_unknown_none`, 3.0 s, and 0 once both were fixed). The
  `gltest_guard` timing test did not fail in either walk (it did at
  `8a5da05e`). Record: GitHub issue 1606; a walk at a symlinked path is
  `docs/notes/gate-symlinked-path.md`.

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
