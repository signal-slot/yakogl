<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Redaction audit of the tree and the history (release stage 3)

Audit for checklist line 8 (`docs/release/checklist.md`), in three
passes:

- **Part 3** (next section): the re-run on the tree after the scope cut of
  #1606 step 5 (2026-10-26). It is the current state of the tree.
- **Part 1** ("Part 1", down to "Notes for other items"): read-only
  audit of the tree and of every ref of the clone. Base: `5f8fff01`
  (ccfarm/integration tip, equal to `origin/main` in that clone), audited
  on 2026-10-15. Part 1 edited nothing.
- **Part 2** ("HEAD-only re-scan and tree fixes", below): the
  pub-redaction-audit item's re-scan of the tree and of every blob
  reachable from `HEAD`. It replaced values with environment-variable names
  in four doc files (listed under "Applied in this item"). No history was
  rewritten by either pass.

Where parts 1 and 2 disagree on the tree, the status after the merge is
noted at the finding (H2, H3, M1). Their `file:line` references are at
their own bases; many of those files have left the tree since.

# Part 3: re-run on the cut tree (2026-10-27)

| Item | Value |
|---|---|
| Date | 2026-10-27 |
| Base | `origin/main` `0dce654a8` ("before" below) |
| Scanned tree | main `1ccb1a1b9` with the step-5 change applied: the base, minus the scope cut, plus the fixes below ("after"); 3,316 tracked files (6,529 at the base) |
| History | not scanned again. Owner decision 1 of #1606: the public repository gets a fresh history, one commit of the cleaned tree, so the scan of that tree is the scan of its history. Parts 1 and 2 describe the private history, which is never published |
| Method | `git grep -n -I <pattern> <rev> -- ':!third_party' ':!docs/release/redaction-audit.md'` per class (part 2's method and exclusions), the same patterns on both trees, every hit read by hand |

Values are masked as in parts 1 and 2 (first character, `***`, length).

## Scope cut (decision 3 of #1606)

Removed from the tree: `tools/mali/board-ref/` (3,136 files of Mali board
captures) and `docs/board-sessions/` (2 session records).
`docs/handoff/` was no longer in the tree at the base.

- The six excerpts that the Mali tests read with `include_str!` moved to
  `tools/mali/board-dump/` (`fvgin-2026-10-21.txt`, `fvgfc-2026-10-21.txt`,
  `fvgfcs-2026-10-22.txt`, `fcfbo-2026-10-22.txt`, `fcpb-2026-10-22.txt`,
  `query-2026-10-20.txt`), with a header that names the source record and
  the sha256 of the dump. Nothing else in the two directories was read by
  a build, a test or a gate row.
- Every other reference to a removed file is a hash citation,
  `0dce654a8:tools/mali/board-ref/<path>` (decision 4; `docs/README.md`
  "Removed records"). The probe programs (`*.c`) and the READMEs of the
  records are readable there with `git show` in the working repository.
- `tools/mali/sweep_passsets.py` takes the records directory as a required
  argument instead of defaulting to the removed one.
- Decision 4's note that cited hashes name commits of the non-public
  working repository: `CONTRIBUTING.md` ("Branches and commits"),
  `README.md` and `docs/README.md`.

## Findings per class, before and after

"Lines" counts matching lines after the look-alikes were dropped (section
numbers such as `13.10.3.1.2`, the RFC 5737 / RFC 1918 fixtures of
`tools/board_selftest.sh`, `NOPASSWD:`, placeholders such as `<user@host>`,
`root@<t6>`, `root@BOARD`, `/home/<...>`).

| Class | Pattern (extended regex) | Before: lines (in the cut dirs) | After | What was done |
|---|---|---|---|---|
| Home path | `/(home\|Users)/[A-Za-z0-9_.-]+` | 774 (771) | 0 | 771 left with `tools/mali/board-ref/`. `tools/precompile/measure-image.sh:45` defaulted `ZEPHYR_WS` to the owner's checkout: `ZEPHYR_WS` is now required (exit 9 with a message when unset). `driver/tools/kexec-zephyr/board-sudo-setup.sh:72-73` named the vendor image's default user in `KO_DIR` / `KEXEC_DIR`: the user is now `${BOARD_USER:-<default>}` (same value unless `BOARD_USER` is set) |
| Owner login | the login name, case-insensitive, not the public name or e-mail | 900 (899) | 0 | the one outside the cut is the `measure-image.sh` line above |
| Private / tailnet IPv4 | 10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16 | 0 | 0 | none |
| IPv6 link-local / ULA | `(fe80\|fd..):...` | 0 | 0 | none |
| Login at a real host | `<login>@<name>` for root, the vendor images' default user and the lab and farm logins | 7 (7) | 0 | all 7 were `root@<the T6's tailnet name>` in `tools/mali/board-ref/2026-10-19-z1*/*/record.txt`, with the owner's login as the person at the power switch: left with the directory. What remains in the tree are placeholders (`root@<t6>`, `torizon@<board>`, `user@host`) |
| Lab machine names | the three gate runners `n***[3]`, `s***[8]`, `h***[4]`, the laptop `L***[6]`, the Pi 4 resource `pi4-e***[8]`, the bridge hosts `u***[4]` and `u***-s***-1`, as whole words | 144 (105) | 0 | in `docs/ci.md`, `.github/workflows/gate.yml` and the comments of `tools/check.sh`, `tools/check_pool_selftest.sh`, `tools/setup-gate-host.sh`, `tools/cts/deps_env.py`, `driver/tools/kexec-zephyr/board_lock*.sh`, `tools/mali/panthor/tests/board_lock.rs`, `zephyr/linker_snippets.cmake`: the runners are "runner A/B/C" (`docs/ci.md` says so once), the laptop "a developer laptop". `docs/es3/board-recipe-pi4.md:221-224`: `<pi4 resource>`. `tools/cts/RESULTS.md:414`: "the UART bridge in `$BOARD_UART`" |
| SoM serial in a host name (part 1 M3) | `verdin-(am62p\|imx8mp)-<8 hex>`, not the zeroed `-00000000` | 0 | 0 | already gone at the base (the files that held it left the tree on 2026-10-16); the tree has only the zeroed form |
| Serial-bridge ports | `3593[0-9]` | 3 (0) | 0 | `tools/board_selftest.sh:162,165,168` used a real bridge port as fixture data: now `40001` (the test checks only that the value is passed through) |
| Tailnet DNS name | `\.ts\.net` | 0 | 0 | none |
| Token / key | `ghp_`, `github_pat_`, `gho_`, `AKIA`, `xox?-`, `AIza`, `sk-`, PEM private-key headers | 0 | 0 | none |
| Password / secret value | `(password\|passwd\|passphrase\|pwd\|secret\|api_?key)\s*[:=]\s*<value>` | 0 | 0 | none (the words appear only in prose and in the sudoers `NOPASSWD:` rule) |
| E-mail addresses | `<local>@<domain>.<tld>` | upstream authors (Slint, KDAB, freedesktop, a Mesa author), `.invalid` placeholders, a self-test's `t@example.org` | same | no private address |

Not counted as findings, as in part 2 T4: the product default host names
`verdin-am62p-00000000` and `verdin-imx8mp-00000000` (zeroed serial, in
fixtures and one display dump), `armadillo.local` (the vendor's mDNS
default, `docs/notes/x2-direct-boot.md:218`), `git@github.com` (the
project's own clone URL), the gate's runner label `yakogl-gate`, and
`ccfarm` (the name of the farm tool).

Left, not a redaction class (no user name, address or host name), but a
lab layout that a public reader cannot use:

| Where | What | Why left |
|---|---|---|
| `tools/check.sh:3329` (`HAL_NXP_DIR`), `tools/cts/cts-build.sh`, `cts-build-zephyr.sh`, `gen-gles3-mustpass.sh`, `host-deqp-run.sh`, `cts-selftest.sh`, `cts-summary.py`, `cts_resume.py`, `tools/es100_oracle.py`, `tools/cts/README.md` (`CTS`) | defaults under `$HOME/com/github/<org>/<repo>`, the lab's checkout layout | every one is overridable by its variable; changing the default changes the gate hosts' runs. Step 7 of #1606 (public CI rows report `gap`) and #1264 (README walkthrough, `docs/release/readme-walkthrough.md:128`) own them |
| `tools/yakogl-tools/src/glsl2usc.rs:69-73` | `PCO_SHC_CANDIDATES`: two lab-host search paths, one into the non-public `yakogl-in-rust` checkout | `PCO_SHC` comes first; the candidates are only a fallback on the lab hosts. Same owner as the row above |

## Verdict of part 3

The scanned tree has no finding of class H or M. The only remaining finding
is the L-class lab layout above. Part 1's H1 to H6 and the history
findings of both parts concern the private history; decision 1 (fresh
history in a new repository) leaves them out of the publication. Re-run
this scan on the snapshot commit before it is pushed: later changes can
bring values back.

# Part 1: tree and every ref (2026-10-15)

## Verdict

**The history cannot be published as is**, neither the whole GitHub
repository nor `main` alone.

- **Every ref (what GitHub holds today):** one branch carries the PowerVR
  firmware image (finding H1). Other branches carry lab tailnet and LAN
  addresses (H4 to H6).
- **`main` alone:** it carries a tailnet address of the Pi 4 serial bridge
  and the Pi 4's lab login (H2, H3), both still in the tree. It also carries
  a farm node's tailnet address in an old file (H4), home paths, and 47
  versions of a ccfarm workspace file (M1, M2).
- **The tree at `5f8fff01`** needs edits before publication: H2, H3, M1,
  M3 and M4.

Recommended path: clean the tree with the edits listed below, then publish
it as a **fresh history** (one initial commit, or a squash at the release
point) in a **new** public repository, and keep the current one private.
This option is the cheapest one that is safe. Two reasons rule out
deleting branches and then flipping the current repository to public:

- GitHub keeps commits readable by SHA after their branch is deleted, and
  pull-request refs keep them too.
- `main` itself needs a rewrite anyway.

The alternative is a `git filter-repo` rewrite (section "History rewrite"),
which changes every commit id that the docs quote. The checklist's owner
decision "Publish rewritten history or a fresh history" is now due.

## Scope and method

| Item | Value |
|---|---|
| Refs scanned | all 496 refs of this clone: 139 `heads`, 355 remote-tracking (`origin/*`, i.e. what GitHub held at snapshot time), and the workspace branch and its base. `origin` itself was not reachable from this machine (ls-remote needs approval), so refs pushed after the snapshot are not covered |
| Commits | 1943 in all refs, 1240 reachable from `HEAD`. 287 remote-tracking branches are not merged into `HEAD`. `origin/ccfarm/integration` (`3b41437a`) has diverged from `HEAD`: it has 120 commits that `HEAD` does not |
| Blobs | all 6549 blobs in `git rev-list --all --objects`, text and binary. Five generated `blobs.rs` versions over 20 MB were scanned in a separate pass |
| Commit metadata | author and committer names and e-mails, and every commit message (same patterns) |
| Tree | `git ls-files` at `HEAD` with file:line. `third_party/` was scanned too, and its hits were triaged separately |
| Patterns | IPv4 (all, then triaged), IPv6 (link-local, ULA, global prefixes), `user@host`, `ssh/scp/rsync` targets, `/dev/tty*`, `/dev/serial/by-id/*`, `/home/<u>` and `/Users/<u>`, token shapes (`ghp_`, `github_pat_`, `sk-`, `AKIA`, `xox?-`, `AIza`, PEM private-key headers), `password/secret/api_key/token =` assignments, e-mail addresses, lab and worker host names, `pvr_fw_blob`, `rgx.fw`, `rgx.sh`, hex arrays over 2000 bytes, binary blobs, Imagination copyright lines with and without a licence grant, and "confidential/proprietary/NDA/internal use only" near TI, NXP, Vivante, Broadcom or Imagination |

**Masking.** Every value is masked as its first character, `***` and the
length, e.g. `s***[11]@e***[10]`. IPv4 addresses are given as their class
only, e.g. "tailnet (100.64/10)". Commit ids and file paths are not
secrets and are given in full.

## Findings

Severity: **H** = a project rule is broken (firmware, board address,
login) and the finding must be gone before publication. **M** = lab or
personal detail that should go. **L** = identifying but harmless; the
owner may accept it.

"Where" names the refs that hold the value. "main" means reachable from
`origin/main` = `HEAD`. "tree" means present in the files at `5f8fff01`.

### H1. PowerVR firmware image committed (class: firmware blob)

| Where | Detail | Action |
|---|---|---|
| commit `2584915e` (2026-10-08, ccfarm worker, "AM62P board session: per-case dEQP-GLES3 failure list … (turn 2)"), path `rgx.fw.36.53.104.796` at the repository root, blob `c9561b04`, 229376 bytes | The blob is a 32-bit ELF for machine 0xF3 (RISC-V), the BXS-4-64 firmware processor. Its name is the DDK's `RGX_FW_FILENAME "rgx.fw"` plus the BVNC. It is the Imagination firmware that the project rule says is never committed, and it is not covered by `LicenseRef-Imagination-DDK-MIT` | **History rewrite / ref removal.** Only one ref holds it: `origin/ccfarm/01a11ab1-df5e-7058-9bec-0b619ff3a607`. It is not on `main` or `ccfarm/integration`. Delete that branch on GitHub. Before any visibility change, also ask GitHub Support to purge the cached objects, or publish in a new repository (recommended). Check that no pull request references `2584915e` |

`driver/src/fw/pvr_fw_blob.h` was **never committed** in any ref: there is
no path matching `*pvr_fw_blob*` and no `driver/src/fw/` directory in any
commit. No hex array over 2000 bytes exists outside
`tests/reference/vivante_blobs.rs`, which holds GC7000 shader words made by
`tools/vivante/gen_shader_table.py` and is not firmware.

### H2. Tailnet address and port of the Pi 4 serial bridge (class: IPv4, host name)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/pi4-common-int.md:27-28` (bridge host `u***[4]`, tailnet (100.64/10) `1***[14]`, port `359**`), `:35` (host and port again). After the merge with part 2 the values are still there, at `:21` and `:30` | tailnet IPv4 + bridge host + port | **Edit:** replace them with "the host:port in `$BOARD_UART`". The file belongs to the running item pi4-common-int, so the edit is left to that item or a later docs item. **Still open** |
| history: commit `76b3242d` (main, 2026-10-12) adds it; 12 blob versions of the same file in other refs (`fca07828`, `7c724433`, `1b952480`, `d20e174d`, `0d1285dd`, `86be8aeb`, …), in about 130 remote branches | same | **History rewrite** (or a fresh history) |

### H3. Pi 4 lab login (class: user@host)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/pi4-common-int.md:12,34,79`, `docs/board-sessions/pi4-plan1-int.md:12,13,49` | `s***[11]@e***[10]` | **Edit:** `"$BOARD_HOST"` or `<user@board>`. The first file is owned by the running pi4-common-int item. After the merge: `pi4-plan1-int.md` **fixed** by part 2; `pi4-common-int.md` **still open** (`:28`, `:30`) |
| tree: lease/resource id `pi4-e***[8]` at `docs/es3/board-recipe-pi4.md:221,223,224`, `docs/board-sessions/pi4-plan1-int.md:22` (now `:24`), `docs/board-sessions/pi4-common-int.md:9`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:27`, `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:28` | the board's host name inside the id | **Edit:** `<pi4 resource>` |
| history: commits `76b3242d`, `9c2c893a`, `04ed90de` on main (2026-10-12), plus copies in other refs | same | **History rewrite** |

### H4. Tailnet address and port of a farm node (class: IPv4, host name)

| Where | Masked value | Action |
|---|---|---|
| history on main: `docs/farm/investigations/replica-zero.md`, added in `2e3da436` (2026-10-08) and removed from the tree in `1e2f0d9a` (2026-10-10). Blob versions `f91b2e35`, `90fab2f7`, `572ea045`; the file also names worker hosts and `/home/t***[6]` | tailnet (100.64/10) `1***[13]` with port `98**` | **History rewrite** for main |
| tree of `origin/ccfarm/integration` (`3b41437a`, on GitHub): the file is still there (blob `f91b2e35`), and so is `docs/farm/ccfarm-locations.md` (worker names) | same | That branch must not be published as it stands. Delete it from the public remote, or reset it to the cleaned `main` |

### H5. Console map with tailnet and LAN addresses (class: IPv4)

| Where | Masked value | Action |
|---|---|---|
| `docs/boards/console-map.md` (blobs `f28ffdaf`, `31ed10cd`), `docs/investigations/am62p-console-empty.md` (blobs `28506a2b`, `bccb68d8`, `e75427c7`), added in `638682b7` and `935f5fb8` (2026-10-09) | two tailnet (100.64/10) addresses `1***[15]` and `1***[14]` with ports `359**`, two LAN (192.168/16) addresses `1***[12]` | Not on main. They **are in the tree of `origin/ccfarm/integration`** and in 33 other remote branches. **Remove the refs** from the published remote, or rewrite. Main's copy, `tools/cts/log/investigations/console-map.md`, already uses `<tailnet address …>` placeholders |
| `docs/benchmarks-baseline/am62p.md`, blob `34d7125e` (commits `03fc61d3`, `fe12a7ed`) | tailnet `1***[15]` | one remote branch only (`origin/ccfarm/01a116f1-3165-7516-b00b-21a6b68412c0`): **delete the ref** |

### H6. Earlier, unmerged drafts of this audit carry the values in clear (class: IPv4, user@host, e-mail, serial)

| Where | Masked value | Action |
|---|---|---|
| `docs/release/redaction-audit.md` in commits `3a4e55f7` and `c33d3d40` (2026-10-07), blobs `a7841c10`, `d823eb07`, on 4 remote branches (`origin/ccfarm/01a1158b-c0f3-…`, `origin/ccfarm/01a115b1-7466-…`, `origin/ccfarm/01a115f0-60bd-…` and one more) | six LAN (192.168/16) addresses, two tailnet addresses, `t***[7]@<LAN address>`, e-mail `t***[6]@s***[14]` (not the public maintainer address), Debug Probe serial `E***` (partly masked) | **Delete those refs** from the published remote. The later draft `1b952480` (on `origin/ccfarm/01a13a41-1769-…`) masks its values but edits other files; this report supersedes it |

### M1. Home-directory paths (class: home path)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/imx8mp-deqp-stage1-gles3.md:21`, `docs/board-sessions/pi4-common-int.md:14`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:9`, `docs/es3/zephyr-env.md:23`, `tools/cts/RESULTS.md:16868,16877`, `tools/cts/log/investigations/gc7000-femtovg-imx8mp.md:67-69`, `tools/precompile/measure-image.sh:45` (a script default) | `/home/t***[6]/…` (the owner's login) | **Edit:** use `$CCFARM_DEPS_DIR`, `$ZEPHYR_WS`, `$WEST_VENV` and `$ZEPHYR_SDK_INSTALL_DIR`. In the script, use `${ZEPHYR_WS:?}`. After the merge: the three `docs/board-sessions/*` / `docs/es3/zephyr-env.md` lines are **fixed** (part 2 T1) and `pi4-common-int.md` no longer has one; the `tools/` lines are still open |
| history: 226 blob versions in all refs (`tools/cts/RESULTS.md`, `.ccfarm-workspace.md`, `.ccfarm-scripts/*.py`, board-session docs, `docs/farm/…`, `tools/hostharness/glmark2/FINDINGS-*.md` and others). Also commit message `0f1bad11` (main) | same | **History rewrite** (`--replace-text`), or a fresh history |
| history, other refs only: `docs/farm/investigations/nuc-health-and-quota.md` (commits `f9e0264b` … `b1fdae97`, one remote branch `origin/ccfarm/01a11bcb-5310-…`) | `/home/s***[4]` (a second person's login) and a worker host name | **Delete the ref** |

### M2. Committed ccfarm worker files (class: home path, host name)

| Where | Detail | Action |
|---|---|---|
| history on main: `.ccfarm-workspace.md`, 47 commits (`1e0087f5` … `5abb436d`), 79 blob versions in all refs | ccfarm's workspace notes: `/home/t***[6]` paths, the node name `n***[3]`, the origin URL | **History rewrite:** `--path .ccfarm-workspace.md --invert-paths`, or a fresh history |
| history, other refs only: `.ccfarm-scripts/{extract2,extract_all,fetch2,fetch_all,fix_zephyr_git,rowD,setup_ws,stage1}.py` | `/home/t***[6]` paths | delete the refs or rewrite |
| history, other refs only: `CLAUDE.md` (`2f1ddf47`) | agent instructions; no address, path or credential found in it | delete the ref, or keep it if the owner agrees |
| tree: `.ccfarm/project.toml` (worker names at :170,:179; it also documents the board-address rule), history `.ccfarm/CYCLE.md` | farm configuration, no address | Dropped from the tree on 2026-10-20 (the settings live in ccfarm); history as below |

### M3. Module serial in a board host name (class: host name)

| Where | Masked value | Action |
|---|---|---|
| tree: `tools/cts/log/investigations/console-map.md:34,36`, `tools/cts/log/board-sessions/am62p-pr133.md:35` | `v***[12]-0864****` (an AM62P SoM's serial, from a login prompt) | **Edit:** `-<serial>`. Other places already use the zeroed form `-00000000` |
| history on main: `18d29afc`, `75f8407b` (2026-10-12) | same | rewrite, or accept (low value to an attacker) |

### M4. Serial bridge host names and ports (class: host name, UART path)

| Where | Masked value | Action |
|---|---|---|
| tree: bridge host `u***[4]` in `docs/es2-cts-failures.md:285`, `tools/cts/STATUS.md:18`, `tools/cts/RESULTS.md:1008,9641,12702,17204`, `tools/cts/log/handoff-2026-10-07-to-10-11.md:63`, `tools/cts/log/investigations/{console-map.md,am62p-console-map.md,integration-main-overlap.md}`; bridge host `u***-s***-1` in `docs/board-sessions/am62p-es2-final.md:5,25,34,37`, `docs/boards/am62p-console-lease.md:9,41,49`, `tools/cts/RESULTS.md` (14 lines, 1008 … 16079), `tools/cts/log/**` (about 20 lines) | TCP serial-bridge host names on the lab tailnet | **Edit:** "the bridge in `$BOARD_UART`". Most of these files are records (`tools/cts/log/**`, `RESULTS.md`), so a bulk replace by a docs item is enough |
| tree: ports `3593*` (four ports) in `tools/cts/RESULTS.md` (16 lines), `tools/cts/log/benchmarks-baseline/am62p.md:8,13,15,31`, `tools/cts/log/board-sessions/{am62p-pr133-baseline,am62p-pr133,am62p-gles3-percase-2026-10-13,farm-console-blocked-2026-10}.md`, `tools/cts/log/investigations/{console-map,am62p-console-map,pr133-report-draft,gc7000-table-pr129-merge-gate}.md`. `tools/board_selftest.sh:161-167` uses one port as fixture data | bridge ports | edit with the line above; the self-test fixture may stay |
| history: the same lines in earlier versions | same | covered by the rewrite or the fresh history |

### L1. Worker host names in commit metadata and messages (class: host name)

| Where | Detail | Action |
|---|---|---|
| author/committer of about 2000 commit fields: `ccfarm (<host>) <ccfarm@<host>>` for six worker hosts (`L***[6]`, `s***[8]`, `X***[9]`, `n***[3]`, `h***[4]`, `L***[12]`) | farm machine names | **Owner decision.** With a rewrite, use `--mailmap` to map them to one `ccfarm <noreply@…>`; a fresh history removes them |
| commit messages: worker names in about 75 messages (60 on main); "tailscale" in 3 (`f2a13877`, `28fb55ae` on main); `/dev/ttyACM9` (fixture) and `/dev/ttyS2` (board-side Linux name) | no address | accept, or drop with the history |
| tree: worker and runner names in `docs/ci.md`, `.github/workflows/gate.yml:54-64`, `tools/check.sh:309,313,2301`, `tools/setup-gate-host.sh:89,207`, `tools/cts/STATUS.md:13,26`, `docs/es3/zephyr-env.md`, `tools/cts/log/zephyr-env-lepton-2026-10-08.md` (two file paths, this one and `nuc-health-and-quota.md` in M1, contain a worker name; paths are shown in full) | self-hosted runner labels and the documentation of where the gate runs | accept (they document the CI), or generalise to "the gate host" |

### L2. Default board logins and product host names (class: user@host)

| Where | Masked value | Action |
|---|---|---|
| tree: `tools/cts/RESULTS.md:15875-15881`, `tools/cts/log/board-sessions/imx8mp-common-int.md:11,40`, `tools/cts/log/benchmarks-baseline/am62p.md:12,31`, `tools/cts/RESULTS.md:15636` and `docs/notes/x2-direct-boot.md:218-224` (`r***[4]@a***[15]` and `root@BOARD`) | the vendor images' default user and mDNS names (`t***[7]@v***[13]`, `t***[7]@v***[12]`, `r***[4]@a***[15]`) | low risk: they are product defaults, not lab addresses. Replace them with `$BOARD_HOST` if no lab name should remain |
| tree: `/home/t***[7]` (the board's default user home) in `tools/cts/RESULTS.md:5376,5475,5555,5736,5833,8405,15878,15879` and `driver/tools/kexec-zephyr/board-sudo-setup.sh:72,73` | on-board paths | no action (overridable defaults on the board) |

### L3. Serial device paths (class: UART path)

| Where | Detail | Action |
|---|---|---|
| tree: `/dev/ttyS2`, `/dev/ttyS3` and others (board-side Linux names) in AM62P session docs; `/dev/ttyUSB0`, `/dev/ttyACM1`, `/dev/ttymxc1` in `tools/cts/RESULTS.md` and `docs/notes/x2-direct-boot.md`; `/dev/ttyACM0` in `docs/board-sessions/pi4-common-int.md:20` | generic kernel device names; no lab machine can be told from them | no action |
| tree: `/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01` at `docs/board-sessions/pi4-plan1-int.md:50`, `docs/es3/board-recipe-pi4.md:33`, `tools/cts/README.md:553` | placeholder only; no ref in history holds the real serial except the partly masked one in H6 | no action |

## Checked and clean

| Class | Result |
|---|---|
| Tokens, keys | No PEM private key and no `ghp_`/`github_pat_`/`gho_`/`AKIA`/`xox?-`/`AIza` value in any blob or message. The two `sk-` hits are a panel overlay name in `tools/cts/log/investigations/pvr-bringup/am62p-desk-research.md` (and its earlier path `driver/docs/gpu-recipes/research.md`) |
| Passwords | Only false positives: "sudo: a password is required" log lines in `tools/cts/RESULTS.md`, the sudoers `NOPASSWD:` line in `driver/tools/kexec-zephyr/board-sudo-setup.sh`, and `secret` variables in `third_party/mesa-26.2.1/src/util/xxhash.h` |
| IPv6 | none |
| Other IPv4 | `192.0.2.0/24` (RFC 5737 documentation range) and `10.11.12.*`/`172.16.0.9` fixtures in `tools/board.sh`, `tools/board_selftest.sh` and `tools/testdata/*.uart.ts` (the redaction self-test checks that they are masked); `127.0.0.1`; and dotted numbers that are not addresses: the BVNC `36.53.104.796`, TRM section numbers in `driver/rust/src/ip/*.rs` and `driver/docs/*-cold.md`, and version strings |
| E-mail addresses | In commits: the maintainer (`tasuku.suzuki@…`, GitHub noreply), GitHub's `noreply`, the bot and `noreply@anthropic.com`. In the tree: upstream authors in vendored headers (`driver/src/vivante/hw/*.xml.h`, `driver/src/v3d/v3d_tex.*`, `examples/nostd/slint-nostd.patch`, `third_party/`), and placeholders on `.invalid` (`SECURITY.md`, `tools/check.sh:2120`). `.inputs/nostd/*/Cargo.toml` (other refs only) has upstream crate authors. None is private, except the address in H6 |
| Large or generated blobs | `tests/reference/blobs.rs` and the older `src/shaderc/blobs.rs` (35-38 MB) were made by Mesa's open `pco_shc` and glslang; no path, address or marking in them. `tools/cts/RESULTS.md` (about 1.4 MB per version) is covered above. Binary blobs: two PNGs, website images, `.inputs/*.crate` (open-source crates, other refs) and H1 |
| Imagination DDK material | `third_party/ti-img-rogue-ddk-25.2/` (50 files, one commit, `afb16335`) comes from TI's public `ti-img-rogue-driver` (`UPSTREAM`). 45 files carry the DDK's "Dual MIT/GPLv2" or "MIT" grant. The rest are `MIT-COPYING`, `GPL-COPYING`, `README`, `UPSTREAM` and the generated `config/am62p_linux_release/config_kernel.h` with its README. All of it is covered by the DDK's own `MIT-COPYING`, vendored with it (NOTICE 4). No other DDK file exists in any ref. Files with an Imagination copyright outside `third_party/` (`driver/src/pvr_*`, `driver/rust/src/gpu/pvr/render.rs`, `src/shaderc/online/powervr/*`, `tests/reference/pvr_internal.rs`, `tools/pco_shc/*`) are Mesa-derived and carry `Apache-2.0 AND MIT` |
| TI / NXP confidential markings | none. "internal use only" in `third_party/ti-img-rogue-ddk-25.2/include/pvrsrv_memallocflags.h:293,386` describes a flag in the upstream MIT file. "TI's proprietary" in `am62p-desk-research.md:31` describes TI's public binary stack and quotes its public build guide |
| `records/` | gitignored; no `records/` path in any commit of any ref |

## History rewrite (if the owner keeps the history)

Not run. If a rewrite is chosen instead of a fresh history:

1. Edit the tree first (H2, H3, M1, M3, M4) and land it on `main`.
2. Run `git filter-repo` on a fresh mirror:
   - drop `.ccfarm-workspace.md`, `.ccfarm-scripts/`, `.ccfarm/CYCLE.md`,
     `CLAUDE.md`, `docs/farm/`, `docs/boards/console-map.md`,
     `docs/investigations/am62p-console-empty.md` and
     `rgx.fw.36.53.104.796` (`--invert-paths`);
   - add `--replace-text` rules for the values in H2-H6, M1, M3 and M4.
     Keep the rules file outside the repository, because it contains the
     values;
   - add `--mailmap` for the worker identities (L1).
3. Push only `main` (and tags, if any) to a **new** repository. Push none of
   the `ccfarm/*`, `work/*`, `land/*` or `fix/*` branches.
4. Re-run this audit's scan over `git rev-list --all --objects` of the
   result.
5. Every commit id quoted in the docs (the GitHub issues,
   `tools/cts/RESULTS.md`, board-session headers, this file) then points
   to the old history. README already says the hashes predate publication.

## Notes for other items

- Licence (release-license-audit): `src/shaderc/online/powervr/cf.rs` says
  its construction is "transcribed" from Mesa `pco_cf.c` (MIT, Copyright
  Imagination), but it carries `SPDX-License-Identifier: Apache-2.0` only.
  The other transcribed pvr files use `Apache-2.0 AND MIT`.
  `driver/rust/src/gpu/pvr/freelist.rs` follows DDK functions and layouts
  under `Apache-2.0` only. Both should be checked.
  `third_party/ti-img-rogue-ddk-25.2/config/am62p_linux_release/config_kernel.h`
  has no SPDX header.
- A gate step that runs these patterns over `git ls-files` (and over the
  diff of a change) would stop new session logs from bringing the values
  back. It is not part of this item.

# Part 2: HEAD-only re-scan and tree fixes

Audit of the working tree and of every blob reachable from `HEAD`
(1257 commits, 5086 blobs; the secret, token-prefix and tailnet-name
patterns were re-run over every blob with `git rev-list --objects HEAD`,
`git cat-file --batch` and `grep -a`, with no hit) for values that must not become public. The earlier audit
(`4b37a25`) never merged and is not reachable from this clone, so this one was
redone from scratch on the current tip.

Values are **masked** here: first character, `***`, and the length in
brackets, e.g. `t***[7]`. Placeholders such as `<user@board>` are not findings.
This file has to pass its own scan; nothing below is a real value.

## Scope and method

| Class | Pattern looked for |
| --- | --- |
| user@host | `name@host` in ssh/scp/`BOARD_HOST` context or with a known login name |
| address | IPv4 in 10/8, 172.16/12, 192.168/16, 100.64/10 (tailnet), 169.254/16; IPv6 full form and link-local |
| tailnet | `*.ts.net`, "tailnet" with a host name |
| serial | `/dev/serial/by-id/<value>` with a real (non-placeholder) serial |
| home path | `/home/<name>` and `/Users/<name>` (not `user`, `you`, `me`) |
| secret | private-key headers, `ghp_`/`github_pat_`/`AKIA`/`sk-`/`xox` tokens, `token`/`password`/`secret`/`api_key` assignments |
| firmware header | the generated PowerVR firmware header under `driver/src/fw/` |
| lab names | the farm's board and worker host names, lease ids, bridge host and ports |

Skipped: `third_party/` (upstream copyright e-mail addresses), `*.xml.h`,
`*.patch`, `tools/testdata/` (recorded fixtures). Binary blobs were not
scanned. Only the history reachable from `HEAD` was scanned, not the other
refs of the clone (part 1 covers those and the binary blobs). Worker names were searched as whole words, case-insensitive
(`git grep -nwi`, and `git log -S` for history).

## Result in one view

| Class | Tree (HEAD) | History only | Verdict |
| --- | --- | --- | --- |
| firmware header | not tracked, ignored by `driver/.gitignore` | never committed (no object with that path in `rev-list --objects HEAD`; `git log --all -- <path>` is empty) | **clean** |
| private key / token / credential | none | none (see T4 for look-alikes) | **clean** |
| `/dev/serial/by-id` real serial | none (files show `<serial>` placeholders only) | none | **clean** |
| tailnet name (`*.ts.net`) | none | none | **clean** |
| home paths | 17 lines, 5 files remain (T1 rows marked open / no change); 3 doc files fixed by this item | 34 file/value pairs, 43 commits | fix tree docs, history owner decision |
| lab login `s***[11]@e***[10]` | fixed in `pi4-plan1-int.md` by this item; still in `pi4-common-int.md` after the merge (part 1 H3) | 3 commits | fix tree doc, history owner decision |
| tailnet IPv4 + bridge host/port | still in `pi4-common-int.md` after the merge (part 1 H2) | 1 commit | fix tree doc, history owner decision |
| lab names (lease id, bridge host, ports, SoM host names) | 21 files | n/a | owner decision |
| worker host names (4 nodes) | 12 files, 86 lines (T5); 4 doc files cleaned by this item | 38 / 34 / 6 commits (H table) | owner decision |
| committed ccfarm worker files | none (`.ccfarm/project.toml` dropped 2026-10-20) | `.ccfarm-workspace.md`, `.ccfarm-scripts/*` | owner decision |

## Tree findings

### T1 Home paths (`/home/<user>`), masked as `/home/<t***[6]>` and `/home/<t***[7]>`

`t***[6]` is the owner's login, `t***[7]` is the board's default Toradex
login.

| File:line | Value | Fix | Status |
| --- | --- | --- | --- |
| `docs/board-sessions/imx8mp-deqp-stage1-gles3.md:21` | `/home/<t***[6]>/.local/share/ccfarm/deps/...` | `$CCFARM_DEPS_DIR` (already named in the line) | **applied** |
| `docs/board-sessions/pi4-deqp-stage1-gles3.md:9` | same | `$CCFARM_DEPS_DIR` | **applied** |
| `docs/es3/zephyr-env.md:23` | `/home/<t***[6]>/.local/share/ccfarm-local/deps/...` | `$CCFARM_DEPS_DIR` | **applied** |
| `docs/board-sessions/pi4-common-int.md` | was `/home/<t***[6]>/.local/share/ccfarm/deps/...` | `$CCFARM_DEPS_DIR` | already clean at the tip (rewritten by an earlier merge) |
| `tools/precompile/measure-image.sh:45` | default for `ZEPHYR_WS` is an owner checkout path | default to `${ZEPHYR_WS:?set ZEPHYR_WS}` or a path relative to `$CCFARM_DEPS_DIR` | open, script (not a doc) |
| `tools/cts/log/investigations/gc7000-femtovg-imx8mp.md:67-69` | `ZEPHYR_WS`, `WEST_VENV`, SDK dir under `/home/<t***[6]>/org/...` | `$ZEPHYR_WS`, `$WEST_VENV`, `$ZEPHYR_SDK_INSTALL_DIR` | open, outside this item's file set |
| `tools/cts/RESULTS.md:16868,16877` | `/home/<t***[6]>/org/yakogl-cts/...` | `$CTS_OUT`-style variable name | open, outside this item's file set |
| `tools/cts/RESULTS.md:5376,5475,5555,5736,5833,8405,15878,15879` | `/home/<t***[7]>/{gcgpu,kexec,vendor-probe}/...` (board's default login home) | low risk: product default; replace with `<board home>` if the owner wants none | open, owner decision |
| `driver/tools/kexec-zephyr/board-sudo-setup.sh:72,73` | `KO_DIR=/home/<t***[7]>/gcgpu`, `KEXEC_DIR=/home/<t***[7]>/kexec` | code default on the board; keep (it is the on-board path, overridable by `KO_DIR`/`KEXEC_DIR`) | no change |

Agent-written `docs/board-sessions/*.md` and `docs/benchmarks-baseline/*.md`
at the tip hold no `/home/<name>` worker path any more (re-checked with
`git grep -n '/home/'` on both directories); the older versions are in
history (H table).

### T2 Lab login and bridge (`docs/board-sessions/`)

| File:line (before this item) | Value | Fix | Status |
| --- | --- | --- | --- |
| `pi4-plan1-int.md:12,13,49` | `s***[11]@e***[10]` (lab login of the Pi 4) | `"$BOARD_HOST"` / `<user@board>` | **applied** |
| `pi4-common-int.md` | same login, plus tailnet host `u***[4]`, tailnet IPv4 `1***[14]` (CGNAT range), TCP port `359**` | `"$BOARD_HOST"`, "host and port as in `$BOARD_UART`" | was clean at this item's base; main's later rewrite of the file brought the values back (`:21`, `:28`, `:30`), so **open** (part 1 H2, H3; owned by pi4-common-int) |

### T3 Lab and board names still in the tree (owner decision)

These identify the lab's hardware (lease ids, bridge host, ports, SoM host
names that include a module serial). None is a credential, but each tells a
reader where the lab's gear is.

| Class | Masked value | Files |
| --- | --- | --- |
| Pi 4 lease / resource id | `pi4-e***[8]` | `docs/es3/board-recipe-pi4.md:221-224`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:27`, `docs/board-sessions/pi4-plan1-int.md:24`, `docs/board-sessions/pi4-common-int.md:9`, `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:28` |
| serial bridge host | `u***[4]`, `usb-s***[8]-1` | 21 files; most lines in `tools/cts/RESULTS.md` (18), `tools/cts/log/investigations/console-map.md` (12) |
| bridge ports | `3593*` (three ports) | `tools/cts/log/benchmarks-baseline/am62p.md:31`, `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md:26`, `tools/cts/log/investigations/*console*` |
| board host login | `t***[7]@v***[12]`, `t***[7]@v***[13]` | `tools/cts/log/benchmarks-baseline/am62p.md:12,31`, `tools/cts/log/board-sessions/imx8mp-common-int.md:11,40`, `tools/cts/RESULTS.md:15875-15881` |
| SoM host name with serial suffix `v***[N]-0864****` | 8-hex module serial | `driver/docs/am62p-display-dump-linux.txt:11`, `tools/cts/RESULTS.md` (3), `tools/cts/log/board-sessions/am62p-pr133.md:35`, `docs/notes/imx8mp-gc-bringup.md:155`, `tools/cts/log/investigations/console-map.md:34,36` |
| root login on dev board | `r***[4]@a***[15]`, `r***[4]@l***[9]`, `r***[4]@B***[5]` | `tools/cts/RESULTS.md:15636`, `docs/notes/x2-direct-boot.md:218-224`, `docs/direct-boot.md` |
| `.ccfarm/project.toml:195` (dropped 2026-10-20) | placeholder `<user@host>` only | none needed |

Proposed fix for the whole class: replace with `$BOARD_HOST`, `$BOARD_UART`,
`<board host>` and drop module serial suffixes (`...-0864****` becomes
`-<serial>`). `tools/cts/RESULTS.md` and `tools/cts/log/**` are another
item's files, so no edit was made here.

### T5 Worker host names (ccfarm nodes) still in the tree

Four worker nodes appear: `l***[6]` (also `L***[11]`, its local-model twin),
`s***[8]`, `h***[4]`, `n***[3]` (the owner's checkout machine), and one
laptop name `X***[8]`. These are machine names, not credentials, but they
name the farm. Counts are lines matched by
`git grep -nwi` on the four names (the fifth, `X***[8]`, matches only in
`docs/farm/devices.md`).

| File:line | Masked value | Context | Proposed fix |
| --- | --- | --- | --- |
| `.ccfarm/project.toml:170,179` (dropped 2026-10-20) | `s***[8]`, `l***[6]` | comments on measurements | none in the tree; history only |
| `.github/workflows/gate.yml:54,55,64` | `h***[4]`, `n***[3]`, `s***[8]` | runner list in a comment | owner decision; runner names show in Actions logs anyway |
| `tools/check.sh:309,313,2301` | `h***[4]`, `l***[6]`, `n***[3]` | measurement comments | replace with "a 16-thread host" etc., owner decision |
| `tools/setup-gate-host.sh:89,207` | `n***[3]`, `s***[8]` | history comments | same |
| `docs/ci.md` (19 lines, e.g. 13,18-21,32-34,299-332) | `h***[4]`, `s***[8]`, `n***[3]` | CI timing report, includes a hardware model name for `h***[4]` | owner decision; the measurements stay valid with "runner A/B/C" |
| `docs/farm/devices.md` (44 lines) | all five names | the farm device report itself; its subject is the worker nodes | owner decision: drop the file from the release or keep it internal |
| `tools/cts/RESULTS.md:16979,17120`, `tools/cts/STATUS.md:13,26` | `n***[3]` | "run on the owner's machine" | replace with "the lead's machine" |
| `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:32`, `pi4-pr133-baseline.md:69`, `investigations/pr133-texel-levels.md:295` | `l***[6]` (in a file name), `n***[3]` | prose | owner decision |
| `tools/cts/log/zephyr-env-<l***[6]>-2026-10-08.md` (6 lines; the name is in the file name too) | `l***[6]` | copy of `docs/es3/zephyr-env.md` | rename and mask with the owner's other log moves |

Applied by this item (doc files it touched, neutral wording "a farm node" /
"the worker node"): `docs/es3/zephyr-env.md` (five places),
`docs/board-sessions/pi4-deqp-stage1-gles3.md` (two places),
`docs/board-sessions/pi4-plan1-int.md:52`. The earlier Pi 4 session notes
`pi4-common-int.md` had none left. Everything else in this table is left as
an **owner decision**: the names are the farm's own and some files report
measurements per machine.

### T4 Look-alikes that are not findings

| File:line | Why |
| --- | --- |
| `tools/board_selftest.sh:422-428,590,598` | recorded fixtures for the redaction self-test: RFC 1918 addresses `1***`, a locally administered MAC, a made-up `token=` value; the test checks that they are masked |
| `tools/check.sh:2120` | `t***[7]@g***[12]` uses the reserved `.invalid` TLD |
| `driver/tools/kexec-zephyr/board-sudo-setup.sh:188,200` | `PASSWD`-style match is the sudoers line `NOPASSWD:` (no secret) |
| `SECURITY.md`, `docs/release/readme-walkthrough.md` | `<security contact placeholder>` placeholder, a TODO for the owner |
| `.github/workflows/*.yml` | `actions/...@vN` action pins |
| `docs/notes/pi4-v3d-bringup.md:390` | version string `1***[6]` (QEMU), not an address |

## History findings (reachable from `HEAD`)

Only paths whose blob differs from the one at `HEAD` are listed; "blobs" is
the number of distinct historic versions that hold the value.

| Class | Path | Masked value | Blobs | Commits that introduced it (sample) |
| --- | --- | --- | --- | --- |
| home path | `.ccfarm-workspace.md` | `/home/<t***[6]>` | 43 | `ce25451`, `524836b`, `8d0bb13`, `1e0087f` (+39) |
| home path | `.ccfarm-scripts/{extract2,extract_all,fetch2,fetch_all,fix_zephyr_git,rowD,setup_ws,stage1}.py` | `/home/<t***[6]>` | 1 each | same group (2026-10-07) |
| user@host | `.ccfarm-scripts/{fix_zephyr_git,setup_ws}.py` | `c***[6]@i***[7]` | 1 each | same group |
| home path | `tools/cts/RESULTS.md` | `/home/<t***[6]>`, `/home/<t***[7]>` | 27 + 39 | many |
| home path | `docs/board-sessions/{am62p,imx8mp,pi4}-deqp-stage1.md`, `zephyr-ws-setup.md`, `imx8mp-gc-case10.md`, `imx8mp-common-int.md`, `pi4-es2-r1.md`, `am62p-plan1.md`, `pi4-common-int.md` | `/home/<t***[6]>` | 1-6 each | board-session merges |
| home path | `docs/benchmarks-baseline/imx8mp.md` | `/home/<t***[6]>` | 1 | one commit |
| home path | `docs/es3-host-env.md`, `docs/es3-host-toolchain.md`, `docs/investigations/es2-host-deqp-{baseline,v3d}.md`, `docs/farm/investigations/replica-zero.md`, `tools/hostharness/glmark2/FINDINGS-*.md`, `tools/precompile/measure-image.sh` | `/home/<t***[6]>` | 1-8 | |
| lab login | `docs/board-sessions/pi4-common-int.md` (also `pi4-plan1-int.md`) | `s***[11]@e***[10]` | 3 commits: `76b3242`, `9c2c893`, `04ed90d` | `e393fe6`, `afb1633` add the login form |
| tailnet IPv4 | `docs/board-sessions/pi4-common-int.md` | `1***[14]` | 1 | `76b3242` |
| private IPv4 | `docs/farm/investigations/replica-zero.md`, `driver/docs/rpi4-v3d-backend-plan.md` | `1***[13]`, `1***[6]` | 3 / 2 | LAN addresses of the lab, owner to confirm |
| board login | `docs/benchmarks-baseline/imx8mp.md`, `docs/board-sessions/{am62p-common-int,am62p-plan1,imx8mp-common-int,imx8mp-gc-case10,imx8mp-gltest-final}.md`, `tools/cts/log/benchmarks-baseline/am62p.md`, `tools/cts/RESULTS.md` | `t***[7]@v***[12/13]` | 1-39 | |
| worker host name | `docs/ci.md`, `docs/farm/devices.md`, `docs/es3/zephyr-env.md`, `tools/check.sh`, `tools/setup-gate-host.sh`, `.github/workflows/gate.yml`, `.ccfarm/project.toml` (dropped 2026-10-20), `tools/cts/**`, many board-session docs | `s***[8]` | 38 commits touch it (`git log -i -S`) | `ae81d86`, `83115f9`, `1cf74cf`, `3f0bcd7`, ... |
| worker host name | same group | `l***[6]` | 34 commits | `ee9404e`, `d043fa1`, `cb476f6`, ... |
| worker host name | `docs/ci.md`, `tools/check.sh`, `.github/workflows/gate.yml` | `h***[4]` | 6 commits | `ee9404e`, `a5f396e`, `d043fa1`, ... |
| worker host name | `docs/farm/devices.md` and others | `n***[3]`, `X***[8]` | too short or generic to count by pickaxe | owner decision |
| fixture addresses | `tools/board_selftest.sh` | RFC 1918 / MAC / token fixtures | 13-15 | (see T4) |
| `website/try-it.html` | `u***[4]@y***[10]` | placeholder `user@yakogl...` | 3 | none needed |

No private key, token, `ghp_`/`AKIA`/`sk-` value, tailnet DNS name or
`/dev/serial/by-id` serial exists anywhere in reachable history. The firmware
header was never committed: `git rev-list --objects HEAD` has no entry for it,
and `git log --all -- <path>` is empty.

## Owner decision: history

Nothing was rewritten. A rewrite changes every commit id since the first
touched commit (the repository has 1257 reachable commits and many branch and
ccfarm references to them), so it needs the owner. Options, cheapest first:

1. **Do not rewrite; publish a squashed tree.** Publish `HEAD`'s tree as a
   single initial commit (or a fresh history from the release point) and keep
   the current repository private. Removes every history finding at once.
   Recommended: the history contains 43 versions of ccfarm snapshot files
   and many lab-specific session logs that have no value publicly.
2. **Rewrite with `git filter-repo`** (not run here):
   - drop paths `.ccfarm-workspace.md`, `.ccfarm-scripts/`;
   - `--replace-text` rules for the home path of the owner and the board
     default login, the lab login, the bridge host and ports, the tailnet
     address, the worker host names and the SoM host-name serial suffix
     (rules file kept outside the repository, since it would itself contain
     the values);
   - `git filter-repo --path driver/src/fw/ --invert-paths` is not needed
     (never committed) but is cheap insurance.
3. **Accept** the home paths, default Toradex names and worker names as
   non-sensitive and rewrite only the lab login, tailnet address and
   `.ccfarm-*` files.

After any of these, force-push to a *new* public remote only; never to the
current one, and re-run this audit (`git grep`, `git rev-list --objects`) on
the result. Also decide the other refs (branches in this clone) separately:
this audit did not scan them.

## Applied in this item (tree, docs only)

- `docs/board-sessions/pi4-plan1-int.md`: lab login to
  `$BOARD_HOST`/`<user@board>`; worker name to "a farm node".
- `docs/board-sessions/imx8mp-deqp-stage1-gles3.md`,
  `pi4-deqp-stage1-gles3.md`, `docs/es3/zephyr-env.md`: owner home path
  dropped in favour of `$CCFARM_DEPS_DIR`; worker node names made neutral.

## Left for other items

- `tools/precompile/measure-image.sh:45` default path (script change).
- `tools/cts/RESULTS.md`, `tools/cts/log/**`, `docs/es2-cts-failures.md`,
  `tools/cts/STATUS.md`: lab names and home paths (T1, T3, T5).
- `docs/ci.md`, `docs/farm/devices.md`, `.github/workflows/gate.yml`,
  `tools/check.sh`, `tools/setup-gate-host.sh`: worker names (T5), owner
  decision.
- `docs/es3/board-recipe-pi4.md`: lease id `pi4-e***[8]` (T3).
- `docs/board-sessions/pi4-common-int.md`: lab login, tailnet address,
  bridge host and port (part 1 H2, H3), owned by the pi4-common-int item.
- Add a gate step that runs these patterns over `git ls-files` (a small
  python scanner next to `tools/check.sh`), so new session logs cannot
  reintroduce the values.
