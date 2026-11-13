<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The public snapshot commit (release stage 3, #1606 step 7)

Defines what the first commit of the public repository holds, from what the
tree records. Read with `docs/release/checklist.md` (line 8, D3, D7, D8) and
`docs/release/redaction-audit.md` (Parts 1 to 4, which rest on `da99b2d7` and
`fb51ce43`). The exclusion list the tool reads is
`tools/release/snapshot-exclude`.

Values are masked as in the audit: first character, `***`, length.

## What the snapshot is

- One root commit, no parent, of one revision's tree of this repository. Owner
  decision 1 of #1606 (2026-10-21): the public repository is a new one and its
  history starts with one fresh commit of the cleaned tree. Checklist D3
  ("publish the existing history, a rewritten one, or a fresh single-commit
  history") is answered that way; the private history is never published
  (`redaction-audit.md`, Part 3, "History").
- Because the tree is the whole history, the scan of the tree is the scan of
  the history. Checklist line 8 is met only when the snapshot commit passes
  `YAKOGL_REDACTION_PRIVATE_FILE=<file outside the repository> python3 -I tools/release/redaction-scan.py <snapshot>`
  with exit 0. Exit 3 means the private classes did not run: that is not a
  pass. The public classes alone read 0 H/M on `fb51ce43` (exit 3); a private
  value set derived in memory read 0 H/M, exit 0 (`redaction-audit.md`,
  Part 4). No private-values file exists on the farm; the owner holds the full
  set.
- Making the snapshot commit does not push, create a remote or publish.
  Promoting it and making the repository public is D7, the publication date
  D8, and the tagged first version with its CHANGELOG section is step 8: all
  the owner's.

## The cleaned tree

The cleaned tree is the tracked tree of the revision, minus the paths of
`tools/release/snapshot-exclude`. At present that list is empty: every
decision that removes something from the public tree has been applied to the
tree itself.

Already out of the tree (nothing to exclude):

- **Scope cut, decision 3 of #1606** (applied 2026-10-26,
  `redaction-audit.md`, Part 3, "Scope cut"): `tools/mali/board-ref/` and
  `docs/board-sessions/` are gone. The six excerpts the Mali tests read live
  in `tools/mali/board-dump/`. References to removed files are hash
  citations (decision 4, `docs/README.md`, "Removed records").
- **Gitignored records**: `records/*` (except `records/README.md`) is in
  `.gitignore`; the run directories of `tools/board.sh` are never tracked, so
  they are in no tree. Likewise `build/`, `/target`, `/work/`, `.claude/`,
  `driver/src/fw/pvr_fw_blob.h` (never committed, not redistributable,
  `building.md` section 6) and the other `.gitignore` entries.
- `docs/farm/`, `.ccfarm/`, `.ccfarm-workspace.md`, `.ccfarm-scripts/`
  (dropped 2026-10-20) and `docs/handoff/` are not tracked.

Kept, on purpose:

- **`third_party/`** with its `UPSTREAM` files, as `NOTICE` lists it (Mesa,
  the TI/Imagination DDK material, the Zephyr PR snapshot) and `LICENSES/`;
  checklist D1 and line 7 decide their licences, and `CLAUDE.md` forbids
  touching them.
- **Hash citations** of the non-public repository (`ab43a20c5:<path>`,
  `0dce654a8:<path>`): kept as they are (decision 4; `CONTRIBUTING.md`,
  "Branches and commits").
- **Generated files** (`src/pvr/internal.rs`, `src/vivante/internal.rs`,
  `tests/reference/*.rs`, `tests/fixtures/`): the gate compares their bytes,
  so the tree keeps them as committed.
- **L-class lab layout and `redaction-scan.allow` look-alikes**: not
  redaction findings; Part 3 leaves them in the tree and Part 4 counts them
  as information.

## Undecided

Nothing below is in `snapshot-exclude`: no recorded owner decision says it
leaves the public tree. Each is a question for the owner. Counts are at
`f7a525e6`.

1. **`tools/cts/log/**`** (356 tracked files in 23 slice directories plus
   `README.md`). `docs/README.md` ("Removed records") says the directory was
   removed on 2026-10-16, but board slices have been committed there since
   (its `README.md` defines a slice as the evidence of a `RESULTS.md` row).
   Part 2 T3/T5 list lab names in files there under "owner decision" or
   "rename and mask with the owner's other log moves". Keep, exclude whole,
   or keep only the slices the public tables cite?
2. **Farm and lab-description docs.** Part 2 T5 asks, for
   `docs/farm/devices.md`, "drop the file from the release or keep it
   internal"; the file is no longer tracked. Still named in the same rows as
   owner decisions: `docs/ci.md` (runner names; Part 3 replaced them),
   `docs/es3/board-recipe-pi4.md`, `driver/docs/am62p-display-dump-linux.txt`
   (host name with a zeroed serial), `docs/notes/x2-direct-boot.md`. Part 3
   found 0 H/M in them; does the owner want any of them out?
3. **Part 3's L-class lab layout.** Defaults under
   `$HOME/com/github/<org>/<repo>` (`tools/check.sh` `HAL_NXP_DIR`,
   `tools/cts/*.sh`, `tools/es100_oracle.py`, `tools/cts/README.md`) and
   `PCO_SHC_CANDIDATES` in `tools/yakogl-tools/src/glsl2usc.rs`. Part 3
   leaves them (each is overridable; #1606 step 7 and #1264 own them). This
   is a code change, not an exclusion; listed so it is not mistaken for one.
4. **Internal process files.** `.github/workflows/gate.yml` (self-hosted
   runner label), `tools/board.sh`, `tools/cts/board-stage.sh`,
   `tools/board_selftest.sh` and the `docs/notes/` board bring-up notes
   describe the lab's process. No decision names any of them; the scan finds
   0 H/M in them.
5. **`CLAUDE.md`** (tracked). Part 1 M2 lists it as "keep it, if the owner
   agrees" (for the ref; the filter-repo option that drops it was not chosen,
   decision 1 chose a fresh history). Its text names the farm tooling
   (`ccfarm`, `ask_farm`); `building.md` and `CONTRIBUTING.md` cite it.
6. **`fresh/yakogl`**, a gitlink (mode 160000, `5b***[40]`, 40 characters)
   with no `.gitmodules`, added by `13a8fba6` ("Fresh clone of origin/main",
   a session's scratch clone). In a public tree it is a dangling submodule
   pointing at a commit of the non-public repository. No decision names it.
   Not a redaction hit, but the tool carries it into the snapshot unless told
   otherwise.
7. **`website/`** and **`.github/workflows/pages.yml`**: the stage-3
   criterion asks for "a website consistent with the status sources", which
   implies they stay; no decision records it. Presumed kept; the owner
   should confirm.

For each: keep, or exclude (the path then goes into
`tools/release/snapshot-exclude` with the decision cited), or keep and
change the file in an ordinary commit.

## Procedure and rehearsal

Placeholder. The snapshot-rehearsal item fills this in: how to run
`tools/release/make-snapshot.py` and the scan, and the result of the
rehearsal on the origin/main tip.
