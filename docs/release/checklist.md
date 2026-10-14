<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Release checklist (stage 3: public release)

Base: `79c73ea2` (integration tip, 2026-10-14). Every state below was read at
this base. This version replaces the one written at `40dd210` (2026-10-12).

Criterion (stage 3 of the owner's roadmap): the repository is public with a
current CHANGELOG, a README whose status tables match the board records,
CONTRIBUTING.md and SECURITY.md, a licence/SPDX audit with no open finding, a
redaction audit showing no host name, address, login, credential, serial
number or internal record in the tree or history, a website consistent with
the status sources, and the roadmap published. Stage 3 follows stage 1 (zero
dEQP-GLES2 failures on the three boards); lines 1, 2, 9 and 13 cannot close
before it.

State words: **met** = the check passes at the base; **partial** = some of it
is done; **open** = a known defect or a check that fails; **pending** = the
work has not landed at the base. Hashes in the Evidence column are the last
commit that touched the file (`git log -1 --format=%h -- <path>`).

## Checklist

| # | Line | Measurable criterion | Evidence (command or file) | State at `79c73ea2` |
|---|---|---|---|---|
| 1 | CHANGELOG current | `CHANGELOG.md` has an `[Unreleased]` section; every merged change is listed; its ES2 conformance line equals the final trees of line 13 | `git log -1 -- CHANGELOG.md` (`02d3e514`); `grep -n -i "0 failures\|zero" CHANGELOG.md` | **partial**. The section exists (line 8). Line 89 says the Pi 4 and the i.MX8MP read 0 failures and the AM62P's 45 vertex-texture cases await the board. Both are out of date: the i.MX8MP tree on `835cf9cc` and the Pi 4 rows 0-14 on `20a48791` read 46 failures each (line 13). Item es2-docs-final holds the file. |
| 2 | README status tables | The dEQP-GLES2 table equals the newest whole tree of each board (line 13); the glmark2 table equals the benchmark records; no row is composed of several commits | `README.md` "Status" (lines 32-68); `docs/HANDOFF.md` "ES 2.0, per board" (lines 44-66) | **open**, `b450b45f`. AM62P: 54 failures on `32778b57`, although `docs/board-sessions/am62p-es2-final-r2.md` has 0 on `63efed8e`. i.MX8MP: 46 on `835cf9cc` (matches the record). Pi 4: 0, "not yet one tree from one commit", although `docs/board-sessions/pi4-es2-final-r2.md` has one tree on `d56e79e5` with 0 and `pi4-es2-final.md` 46 on `20a48791`. Item es2-docs-final holds the file. |
| 3 | CONTRIBUTING.md | The build, test and gate commands in it run as written on a fresh host; it names `tools/check.sh` | `docs/release/readme-walkthrough.md` (`5b83614c`), table "Checklist: what can change" | **partial**. The walk ran the commands at `a63fcc5b`: README steps 4-6 fail as written, and `tools/check.sh --quick` is clean only with `MESA_V3D_TREE` unset (N9). N4 (merge vs rebase rule) and N10 (untracked files widen `--changed`) are open. |
| 4 | SECURITY.md | Names a reachable private contact, the scope and the response times; no `TODO(owner)` or placeholder left | `grep -n "example.invalid\|TODO(owner)" SECURITY.md` | **open**. Line 16 is still `<security-contact@example.invalid>`; the walk's N5 counts three `TODO(owner)` lines. Owner decision D5. |
| 5 | DESIGN.md | Every path it names exists in the tree; its conformance paragraph equals line 13 | `docs/release/readme-walkthrough.md` table "Paths checked" (paths); `sed -n 21,26p DESIGN.md` (numbers) | **partial**. Paths: all exist (walk at `a63fcc5b`; DESIGN.md unchanged since `5b83614c`). Numbers: lines 21-26 still say the i.MX8MP has 0 and the AM62P 54 on `32778b57` (walk finding N7). |
| 6 | README walkthrough | A new reader follows README then CONTRIBUTING on a host and reaches a passing `tools/check.sh --quick` with no gap | `docs/release/readme-walkthrough.md`, "README findings, all open" | **open**. README findings 1-10 are open at the base, for example `grep -n "aarch64.cmake\|PowerVR build only\|Five are open" README.md` still prints lines 192, 307 and 357. |
| 7 | Licence / SPDX audit | A tool lists every tracked file without a valid `SPDX-License-Identifier` (or a covering REUSE entry) and every named licence missing from `LICENSES/`; its report has zero open findings and every finding is fixed or decided by the owner | `ls tools/license_audit* docs/release/license-audit.md` | **pending**. Neither exists at the base (item pub-license-audit is running). Input for it, not a result: `git ls-files -z \| xargs -0 grep -L SPDX-License-Identifier \| grep -v '^third_party/' \| wc -l` prints 1016 at the base (799 of them under `tools/shaders/`), and there is no `REUSE.toml`. `LICENSES/` holds 8 texts, `NOTICE` lists them. |
| 8 | Redaction audit, tree and history | No host name, IP address, user name, login, credential, board serial number or internal farm record in the working tree, nor in any commit reachable from the branch to publish; the report names the scan, its patterns and every hit with its disposition | `ls docs/release/redaction-audit.md` | **pending**. The report does not exist at the base (item pub-redaction-audit is running). `1e2f0d9a` removed `docs/farm/` and `docs/plan-status.md` from the tree, but they remain in history; only the audit can say what else does. |
| 9 | website/ consistent | Every number on `website/index.html`, `status.html` and `try-it.html` (from `website/data.js`) equals line 2's sources; `tools/website-check.sh` exits 0 | `grep -n checkedAgainst website/data.js`; `bash tools/website-check.sh` | **open**. `data.js` (`0a7607b2`) says it was checked against `1cf946a9` (2026-10-11) and gives the AM62P 54 failures on `32778b57` and the i.MX8MP 0. `bash tools/website-check.sh` exits 1 at the base with 6 problems, all "commit ... is not a commit in this repository" in `data.js` (`d60b9d76`, `d8583f20`, `fe5e8e73`, `1fa941e3` twice, `031de9a2`): hashes of the history before `afb16335`, which README line 67 says the logs may quote but the checker refuses. The checker covers links, metadata, SPDX and commit names, not numbers. Item pub-website holds `website/`. |
| 10 | Public ROADMAP.md | `ROADMAP.md` is in the tree with the owner's five stages and a measurable criterion each | `ls ROADMAP.md` | **pending**. Not at the base; item pub-roadmap is ready, not started. |
| 11 | Licence texts and NOTICE | `LICENSE` is Apache-2.0; every licence named in `NOTICE` has its text in `LICENSES/`; every vendored directory under `third_party/` is listed in `NOTICE` | `head -3 LICENSE`; `ls LICENSES/`; `NOTICE` lines 9-31 | **partial**. `LICENSE` is the Apache License 2.0; the 8 texts `NOTICE` names are in `LICENSES/` (read by hand at the base). The third_party listing is for line 7's tool to check, not checked here. |
| 12 | Gate is green | `tools/check.sh` (full) exits 0 on the commit to publish | `tools/check.sh`; CI as in `docs/ci.md` | **partial**. Not run in full for this line. The row that stopped the previous attempt's review, `cargo clippy -p yakogl-precompile --features gpu-vivante --all-targets`, printed no warning at the base with clippy 0.1.101 (2026-10-11), after `touch src/lib.rs`. Item clippy-gate-fix holds the clippy work in `src/gl/draw.rs` and `src/gl/texture.rs`. |
| 13 | Stage 1 met (input) | One whole dEQP-GLES2 tree per board, from one commit, newer than every GL-core change, reads 0 Fail over the 17,165 mustpass cases (AM62P with its ms4 image) | `docs/board-sessions/{am62p,imx8mp,pi4}-es2-final*.md`; `docs/HANDOFF.md` lines 44-66 | **open**. 0 Fail on 2026-10-12: AM62P `63efed8e`, i.MX8MP `c345ab74`, Pi 4 `d56e79e5` (the three `*-es2-final-r2.md`). Since then the `GL_BGRA_EXT` renderbuffer regression: i.MX8MP 46 Fail on `cd738b1a` (twice) and on `835cf9cc`, Pi 4 rows 0-14 46 Fail on `20a48791`. AM62P images 1-9 on `cd738b1a` were built but not run (console silent, `am62p-es2-final.md`). `git log --grep=BGRA` shows no fix at the base. |

## Decisions only the owner can make

| # | Decision | Why it is the owner's | Needed by |
|---|---|---|---|
| D1 | Licence of each component: YakoGL's own code (Apache-2.0 today, `LICENSE`); the Mesa-derived parts (MIT); the Imagination DDK material (MIT with the GPLv2 alternative); the NXP `hal_nxp` part (BSD-3-Clause); the Slint parts (GPL-3.0 / Slint Royalty-free 2.0 / Slint Software 3.0, which one applies to the samples); the Zephyr PR snapshot; and whatever line 7 finds | A legal choice; the audit only reports | before line 7 can reach zero |
| D2 | Data files without an SPDX line (the 799 under `tools/shaders/`, test fixtures): a header in each, or a `REUSE.toml` that covers them | Changes many files or adds a licensing convention | before line 7 |
| D3 | History: publish the existing history, a rewritten one (filter out what line 8 finds), or a fresh single-commit history | Rewriting changes every hash quoted in the docs; a fresh history loses provenance | after line 8 |
| D4 | What the README conformance table claims: per-board whole-tree counts from one commit each (as line 13 asks), or the newest reading per row; and whether a board with a known open regression may be published as "close to complete" | It is the public claim of the project | before lines 2, 5 and 9 close |
| D5 | Security contact address, response times and supported versions in SECURITY.md | They become public | before line 4 |
| D6 | Merge or rebase rule for contributors (walk finding N4: CONTRIBUTING.md and the project settings disagree) | A project policy | before line 3 |
| D7 | Promote the branch to publish to `main` and make the repository public | Irreversible: copies and caches cannot be recalled | after every line above is met |
| D8 | Publication date | Depends on stage 1 and stage 2 | last |

No board address, login, user name or credential is written in this file.
