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

### Rehearsal on the origin/main tip (2026-11-23, real commit da11eb6b)

- **Source** (a real commit, no throw-away commit):
  `da11eb6b5881bf96bd64bffe9fe103d96e1ed515` ("tree: accidental gitlink
  `before` (a9ca11e1) removed, the snapshot tool accepts the tree again"),
  the commit that removed the `before` gitlink and a descendant of the
  redaction cleanup `3a0ef3d1` ("release: private redaction scan clean at
  the tip (one reworded resource id, one narrow lab-host rule)", the commit
  that last changed `tools/release/redaction-scan.allow` for the
  `docs/release/snapshot.md` lab-host rule; `git merge-base --is-ancestor
  3a0ef3d1 da11eb6b` holds). Its tree (`8bf46873`) has no mode-160000 entry.
- **Identity and date**: `yakogl rehearsal <rehearsal@invalid>`,
  `2026-01-01T00:00:00Z`, given as `--author` and `--date`.
- **Tool output** (exit 0, both runs): `source
  da11eb6b5881bf96bd64bffe9fe103d96e1ed515`, `snapshot
  b346c645c57c8e59b848bfde4274b817bc4f43da`, `files 15284`, `exclusions:
  none (tools/release/snapshot-exclude lists no path)`. The same command
  into a second output directory printed the same `snapshot` hash: the tool
  is deterministic. `--self-test` passed.
- **Shape** (read through `git fetch <outdir> HEAD`; the worker could not
  run `git -C <outdir>`): `git rev-list --count FETCH_HEAD` 1; the commit
  has no parent; author and committer `yakogl rehearsal
  <rehearsal@invalid>`, date `2026-01-01T00:00:00Z`; its tree `8bf46873` is
  the tree of the source commit. The output directory's `.git/config` has no
  `remote` section.
- **Licence audit** (check 25's five `--fail-on` classes plus `undecided`,
  `--root` the snapshot checkout): exit **0**: 15250 files, 5867 with an
  SPDX line, 7830 allowlisted; missing-spdx 0, unknown-id 0,
  unused-license 0, third-party-mismatch 0, undecided 0, generated 91
  (mismatch 0), third-party-unlabelled 13 (information); findings 0.
- **Redaction scan, public classes** (on the fetched snapshot commit with
  `--allow <outdir>/tools/release/redaction-scan.allow`): exit **3**, "0
  H/M finding(s), 3 class(es) not run, 115 informational, 145 look-alike(s)
  dropped: NOT COMPLETE (private classes not run)" (15061 files scanned;
  `third_party/` and `docs/release/redaction-audit.md` excluded).
- **Redaction scan, private classes** (same commit and allow file; one
  `python3 -I -c` wrapper process held the values in the environment
  variable `YAKOGL_REDACTION_PRIVATE` and checked that none of them appears
  in the scan's output: 0 did): a **partial** set from `ask_farm` and the
  resource list, as in Part 4 of `redaction-audit.md` (owner-login 1,
  public-name 1, login 1, lab-host 39: the nodes, gateway and laptop, the
  serial-bridge hosts under both names, the KVM hosts, the hardware ids of
  USB adapters; not the board and product names). Exit **0**: "0 H/M
  finding(s), 0 class(es) not run, 115 informational, 198 look-alike(s)
  dropped: clean". A broader set that also holds the resource ids that are
  at the same time public product or tool names (`armadillo-x2`,
  `verdin-am62p`, `verdin-imx8mp`, `rpi5-console`, `tailvision-am62p`, the
  `stm32mp257f-dk-*` ids; a login that is the owner's first name without
  its `public-name` entry) read exit 1 with 165 H/M (164 `lab-host`, 86
  files, 1 `owner-login`); all are those product, tool and public names,
  none a farm machine name. Whether any of those ids should leave the tree
  is the owner's call, not made here. No value was written to a file, a
  log or a commit. Line 8's complete private run with the owner's own file
  stays open.
- The snapshot directories sit under `$TMPDIR` (removed with the task); no
  remote was added and nothing was pushed.

The real snapshot of the commit to publish, line 8 on it with the owner's
private file, D7 (promote and make public) and D8 (the date) are the
owner's, with the first tag of #1606 step 8.
