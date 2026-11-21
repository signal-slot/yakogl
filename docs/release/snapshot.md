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
`34200a05`.

1. **`tools/cts/log/**`** (865 tracked files in 38 slice directories plus
   `README.md`). `docs/README.md` ("Removed records") records the 2026-10-16
   removal of the directory's old per-session logs, and files committed
   there again since `e839242b` (2026-10-27): the slices RESULTS.md rows
   cite, which its `README.md` defines as the evidence of a `RESULTS.md`
   row. Part 2 T3/T5 list lab names in files there under "owner decision"
   or "rename and mask with the owner's other log moves". Keep, exclude
   whole, or keep only the slices the public tables cite?
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
6. **`fresh/yakogl`**: settled. The gitlink is no longer in the tree
   (`git ls-files -s` lists no mode 160000 entry); `a59c5c87` removed it.
   The number stays so that the Rehearsal's reference to it holds.
7. **`website/`** and **`.github/workflows/pages.yml`**: the stage-3
   criterion asks for "a website consistent with the status sources", which
   implies they stay; no decision records it. Presumed kept; the owner
   should confirm.

8. **Commit hashes in text.** Backticked 7-12 hex hashes name commits of
   the non-public history, which a one-commit repository does not hold:
   `CHANGELOG.md` 145 (in 112 lines), `tools/cts/log/` 122 in 34 files, the
   rest of `docs/` 2165 in 246 files (`git grep -hoE
   '`[0-9a-f]{7,12}`'`). They stay readable but no `git show` resolves them.
   Decision 4 keeps the `<hash>:<path>` citations; it does not speak of
   these. Leave them as provenance of the private history, or reword?
9. **Issue and pull request references.** `#NNNN` (2 to 4 digits) in tracked
   Markdown outside `third_party/`: 1780 occurrences in 324 files, 25 of
   them `#1606`. They point at the private repository's issues, which a
   public reader cannot open, and in the public repository `#N` links to a
   different issue or pull request. Keep, or reword?
10. **Links that assume the repository name.** `github.com/signal-slot/yakogl`
    in 18 tracked files (49 lines) outside `third_party/`. Will the public
    repository have the same name and the branch `main`? If not, they change.
11. **Fork pull requests and self-hosted runners.**
    `.github/workflows/gate.yml` has `on: pull_request` (branches
    `[main, ccfarm/integration]`), `schedule` and `workflow_dispatch`, and
    one job with `runs-on: [self-hosted, Linux, X64, yakogl-gate]` (the
    other has `runs-on: ubuntu-latest`). In a public repository a fork's
    pull request can reach a self-hosted runner. Does the owner restrict
    fork pull requests in the repository settings at D7, or change the
    workflow (also the `ccfarm/integration` branch name)?
12. **Step 8, the first tag.** `CHANGELOG.md` has `## [Unreleased]` (line 14)
    and `## [0.1.0] - 2026-10-27` (line 322); no version is tagged. The
    public history starts later than that date. `ROADMAP.md` ("State")
    says the public repository's history starts with one fresh commit, and
    `SECURITY.md` says nothing of the private history. What does the
    `[0.1.0]` section say and date, what is the tag's name, and when is it
    made, and do `ROADMAP.md` and `SECURITY.md` mention the private history?
13. **Lab tooling names.** `git grep -il -e <VM-lab tool> -e ccfarm -e ask_farm`
    outside `third_party/` finds 56 files (4 name the VM-lab tool, 53 `ccfarm` or
    `ask_farm`): 22 in `docs/notes/`, 7 in `tools/cts/`, 5 each in
    `docs/release/` and `docs/es3/`, the rest elsewhere. `CLAUDE.md`
    (item 5) is one of them. Is the VM-lab tool (name withheld; the owner knows it) public? If it is,
    the names stay, with an allow rule for the scan; if not, the text is
    reworded.

For each: keep, or exclude (the path then goes into
`tools/release/snapshot-exclude` with the decision cited), or keep and
change the file in an ordinary commit.

## Procedure and rehearsal

### Procedure

From a clean checkout of this repository (the tool refuses uncommitted
changes to tracked files, exit 4), with `OUT` an absent or empty directory
outside the checkout and under `$TMPDIR`, never `/tmp`:

```sh
python3 -I tools/release/make-snapshot.py --self-test
GIT_AUTHOR_NAME='<name>' GIT_AUTHOR_EMAIL='<email>' GIT_AUTHOR_DATE='<date>' \
  python3 -I tools/release/make-snapshot.py <rev> "$OUT"
# or: python3 -I tools/release/make-snapshot.py \
#       --author '<name> <<email>>' --date '<date>' <rev> "$OUT"
git -C "$OUT" rev-list --count HEAD    # 1
git -C "$OUT" log --format=%P          # empty: no parent
git -C "$OUT" remote                   # empty: no remote
```

The tool prints `source`, `snapshot`, `files` and `exclusions:` (its
`--help` gives the exit codes). Then, inside `$OUT`, with the snapshot's own
copies of the tools:

```sh
python3 -I tools/release/redaction-scan.py HEAD
YAKOGL_REDACTION_PRIVATE_FILE=<file outside the repo> \
  python3 -I tools/release/redaction-scan.py HEAD
python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch,unused-license,undecided
```

Where commands cannot run inside `$OUT`, run them from the source checkout
instead. Use the same tools (byte-equal), the snapshot's commit (after
`git fetch "$OUT" HEAD`, scan `FETCH_HEAD`), `--allow
"$OUT/tools/release/redaction-scan.allow"` for the scan and `--root "$OUT"`
for the audit.

Line 8 needs exit 0 from the second command (all classes ran, 0 H/M); exit 3
from the first is "private classes not run", not a pass. The snapshot
directory is removed after a rehearsal; nothing is pushed and no remote is
added.

### Rehearsal on the origin/main tip (2026-11-17)

- **R** (source): `239b538dce0420f96a6dfbe46f7be60d62cf9cdd`, origin/main's
  tip as held by the farm's read-only copies of the remote's refs (the two
  copies of `main` both resolve to it; commit date 2026-11-17). A fresh
  `git fetch origin` was not possible from the worker (host key
  verification). R is a commit of origin/main, not of the integration
  branch, and a descendant of the previous rehearsal's source
  `cc2cc536c12a6778b348a28f3ef215127f509b78`.
- **Identity and date**: `yakogl rehearsal <rehearsal@invalid>`,
  `2026-01-01T00:00:00Z`, given as `--author` and `--date`.
- **Tool output**: `source 239b538dce0420f96a6dfbe46f7be60d62cf9cdd`,
  `snapshot fa5e7d71e70e115991cd6396a6904321430c2953`, `files 11098`,
  `exclusions: none (tools/release/snapshot-exclude lists no path)`; exit 0.
  The same command into a second output directory printed the same
  `snapshot` hash: the tool is deterministic. Its `--self-test` is the
  `tools/check.sh` row (held item snapshot-gate-row), not part of this
  rehearsal.
- **Shape**: `git rev-list --count fa5e7d71` 1; `git log --format=%P` empty
  (one root commit, no parent); the snapshot's `.git/config` has no
  `[remote]` section (no remote; none was added). Author and committer are
  the rehearsal identity at `2026-01-01T00:00:00Z`. The snapshot's tree
  (`37a1d7cb`) is R's tree byte for byte (equal `{tree}` ids), so a kept
  submodule gitlink — which the tool refuses, exit 8 — never arose.
- **Redaction scan, public classes**: exit **3**, "0 H/M finding(s), 3
  class(es) not run, 98 informational, 132 look-alike(s) dropped: NOT
  COMPLETE (private classes not run)" (10875 files scanned; `third_party/`
  and `docs/release/redaction-audit.md` excluded, as Part 3 does). Exit 3
  is not a pass for checklist line 8. The scan ran on the snapshot commit
  through the Procedure's fallback (the worker cannot `cd` outside its
  checkout): the snapshot was fetched into a clean source checkout
  (`git fetch <outdir> HEAD`), `FETCH_HEAD` was scanned with
  `--allow <outdir>/tools/release/redaction-scan.allow`, and the scan's
  `redaction-scan.py` and allow file are byte-equal to the snapshot's
  copies (blob `1e5a1d94`, blob `de757d79`, both unchanged at R).
- **Redaction scan, private classes**: not run on this snapshot; no
  private-values file exists on the farm and none was built. This is a gap,
  not a pass. The last private run — a partial set held in process memory
  only, as in `da99b2d7`, on the snapshot of an earlier tip (`2b94031f`) —
  found the 7 `lab-host` hits below. `docs/notes/x2-glmark2-slow-scenes.md`
  changed since (`6ca3428f` appended an eighth-attempt section), but the
  three hit lines and their values are unchanged at R; a private run on R
  still waits on the owner's full set.
- **Licence audit** (`--root` the snapshot checkout, byte-equal script,
  blob `74f70bcc`): exit **0** with the five `--fail-on` classes, exit **0**
  with `,undecided` appended: 11064 files, 4145 with an SPDX line, 6208
  allowlisted; missing-spdx 0, unknown-id 0, unused-license 0,
  third-party-mismatch 0, undecided 0, generated 77 (mismatch 0),
  third-party-unlabelled 13 (information); findings 0.

H/M hits on the snapshot (masked; paths and lines in the snapshot): none on
the public classes. R's allow file carries the rules for the three fixture
values of `tools/probes/rpi5_display/verdicts_selftest.sh`; the scan dropped
them, as in earlier runs. From the earlier private run, re-read at R:

| Class | Where | Value | Disposition |
|---|---|---|---|
| `lab-host` (M, private) | `docs/notes/x2-glmark2-slow-scenes.md:41` | `L***[6]` ×2, `h***[4]` | open: farm machine names in a board note; plan #363's notes-hostnames item rewords the file |
| `lab-host` (M, private) | `docs/notes/x2-glmark2-slow-scenes.md:42` | `n***[5]`, `n***[3]`, `s***[8]` | open: as above |
| `lab-host` (M, private) | `docs/notes/x2-glmark2-slow-scenes.md:43` | `n***[5]` | open: as above |

The same note also names a worker's host (`o***[5]`) at lines 32, 42, 52,
62, 71, 79, 87 and 97; the partial set did not hold it, so whether the
private class counts it is the owner's set to decide. The notes-hostnames
item covers the file. None was fixed in the rehearsal. R does not meet line
8: the private classes have not passed on it. Checklist row 8 is unchanged.

The real snapshot of the commit to publish, line 8 on it with the owner's
private file, D7 (promote and make public) and D8 (the date) are the
owner's, with the first tag of #1606 step 8.
