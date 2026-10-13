<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pub-walkthrough, 2026-10-14, base a63fcc5b (item gc-msaa-record)

## Open work

Stage 3 (publication), release checklist `docs/release/checklist.md`:

- Line 5 (DESIGN.md, every named path exists): can change to **met**. Three stale
  references were fixed (`gc_kick.h`, `v3d_kick.h`, three old-repository commits);
  132 paths checked, list in `docs/release/readme-walkthrough.md`.
- Line 3 (CONTRIBUTING.md commands run as written): stays **partial**. The README
  commands were run at `a63fcc5b` (exit status and time in the walkthrough doc):
  CMake host build and the five `cargo test` rows pass; `--quick` is clean (119 ok,
  0 failed, 0 gaps, 236 s) only with `MESA_V3D_TREE` unset, otherwise 2 rows fail.
  The full gate was not re-run; its 35 to 50 minutes come from earlier walks.
  CONTRIBUTING.md now says so and names the `env -u` form.
- Line 6 (walkthrough): stays **partial**. `docs/release/readme-walkthrough.md` is
  rewritten for `a63fcc5b`. README findings 1-10 are open (steps 4-6 of its Building
  block fail as written) and are listed with line and proposed text; the item that
  holds README.md (pub-roadmap) should apply them. It can become met when they are
  applied and a `tools/` item makes `--quick` exit 0 with `MESA_V3D_TREE` set to a
  tree without `build-v3d`.
- Line 4 (SECURITY.md): unchanged; the contact and two other `TODO(owner)` lines
  are the owner's.
- `docs/release/building.md`: ten drifted `file:line` references corrected.
- Outside this item, for a `tools/` item: `tools/check.sh:17` (`--quick` wording),
  `tools/check.sh:2519` (symbolic `ZEPHYR_WS` breaks the AM62P Zephyr rows, cause not
  confirmed by a build).
- Owner: CONTRIBUTING.md says a pushed branch is never rebased; the ccfarm
  `conventions` text says branches are rebased onto origin/main.

Note: `tools/handoff-notes.sh --check` exits 0 at the base and exits 1 while this
note waits; it passes again after the fold.
