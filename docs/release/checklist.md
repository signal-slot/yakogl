<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Release checklist (stage 3: public release)

Base: `40dd210` (main, 2026-10-12). Every state below was read at this base.
The older text of this file described the integration branch at `89d518e`;
this version replaces it.

Criterion (stage 3 of the owner's roadmap): the repository is public with a
current CHANGELOG, a README, CONTRIBUTING.md and SECURITY.md, a licence/SPDX
audit with no open finding, a redaction audit showing no board address,
credential or internal record in the tree or history, a website consistent
with the status sources, and the roadmap published.

State words: **met** = the check passes at the base; **partial** = some of it
is done; **open** = a known defect; **pending** = the work has not landed.
The hash is the last commit that touched the proof (`git log -1 -- <path>`).

## Checklist

| # | Line | Measurable check | Proof | State at `40dd210` |
|---|---|---|---|---|
| 1 | CHANGELOG current | `CHANGELOG.md` has an `[Unreleased]` section; every merged item is listed; it does not claim zero board failures before the final ES2 trees are recorded | `git log -1 -- CHANGELOG.md`; `grep -n -i "zero\|0 fail" CHANGELOG.md` | **partial**, `02d3e51`. The section exists. Line 89 says Pi 4 and i.MX8MP read 0 failures and the AM62P's 45 vertex-texture cases await the board; that matches the README. It is not final: re-read it after the last ES2 trees. |
| 2 | README status tables | The dEQP-GLES2 table equals the latest recorded tree of each board; the glmark2 table equals the benchmark records | `README.md` "Status"; `tools/cts/STATUS.md`; `tools/cts/RESULTS.md`; `tools/cts/log/board-sessions/am62p-es2-r1c.md` | **open**, `82776b7`. README reports AM62P 54 failures (tree `32778b57`), i.MX8MP 0, and Pi 4 0 but "not yet one tree from one commit". The AM62P record (`c2dfdc0`) found 47 failures at a later tip (45 vertex-texture, 2 ms4 constancy). Redo the table from the final trees. Stage 1 is not met. |
| 3 | CONTRIBUTING.md | Build, test and gate commands in it run as written; it points to `tools/check.sh` | `tools/check.sh --quick`; read with `docs/release/building.md` | **met** (text), `02d3e51`. It names `tools/check.sh` and `--changed origin/main`. Commands were read, not re-run for this line. See line 6 for a known gap. |
| 4 | SECURITY.md | States how to report a problem and what is in scope; names no private address | read `SECURITY.md`; owner confirms the contact | **partial**, `b4ec427`. Text is merged. The contact is an owner decision (below). |
| 5 | DESIGN.md | Every path it names exists in the tree | list the paths in `DESIGN.md`; `ls` each | **met** (text), `02d3e51`. Not re-run path by path for this line. |
| 6 | README/CONTRIBUTING walkthrough | A new reader follows README then CONTRIBUTING on a host and reaches a passing `tools/check.sh --quick` with no gap | `docs/release/readme-walkthrough.md` | **partial**, `34bf2aa`. The walk is merged and covers commit `ea984992`. Its own status note says two findings are still in the text: `aarch64.cmake` (README.md) and "about 45 minutes" (CONTRIBUTING.md). The others were not re-checked. |
| 7 | Licence / SPDX audit | A tool lists every file without a valid `SPDX-License-Identifier` and every licence text missing from `LICENSES/`; zero open findings | `tools/license_audit*`; report in `docs/release/*audit*` | **pending (plan 18)**, item release-license-audit. Neither exists at the base. Known input: `LICENSES/` has 8 texts (`afb1633`), including two Slint and one Imagination DDK `LicenseRef-`. Vendored code is an owner question (below). |
| 8 | Redaction audit, tree and history | No board address, login, credential or internal record in the working tree or in any commit reachable from the branch to publish | audit tool and report in `docs/release/*audit*` | **pending (plan 18)**, item release-redaction-audit. Neither exists at the base. `tools/cts/log/investigations/console-map.md` (`18d29af`) has no IPv4 address now, but a grep of the tree is not an audit: history is not scanned. |
| 9 | website/ consistent | Every number on `website/index.html`, `status.html` and `try-it.html` equals the status sources of line 2; `tools/website-check.sh` passes | `tools/website-check.sh`; read the pages against `tools/cts/STATUS.md` | **open**, `ea98499`. The checker (`49b1f4d`) covers links, metadata and SPDX, not numbers. The failure table on `index.html` repeats the README's AM62P history and must follow line 2. |
| 10 | Public ROADMAP.md | `ROADMAP.md` is in the tree with the five stages and their criteria | `ls ROADMAP.md` | **pending**. `ROADMAP.md` and `docs/plan-status.md` are not at the base. Item pub-roadmap is in review. |
| 11 | Gate is green | `tools/check.sh --quick` exits 0 on the tree to publish | run it | **met for this change**: only this file changed. The full `tools/check.sh` runs in CI (`docs/ci.md`, `1cf74cf`). |

Lines 1, 2 and 9 depend on stage 1 (zero ES2 failures on all three boards).
Do not close them before the final trees are recorded.

## Decisions only the owner can make

| Decision | Why it is the owner's | Needed by |
|---|---|---|
| Promote the integration branch to `main` | `main` is the branch to publish; ccfarm does not push it | before lines 7 and 8 are final |
| Make the repository public | Irreversible: copies and caches cannot be recalled | after all lines above are met |
| Licence for vendored code (Slint licences, Imagination DDK MIT text, and anything line 7 finds) | A legal choice, not a code fix | before line 7 can reach zero |
| Publication date | Depends on stage 1 and 2 results | last |
| Security contact address in SECURITY.md | It becomes public | before line 4 |
| Publish rewritten history or a fresh history, if line 8 finds a leak in old commits | Rewriting changes every hash quoted in the docs | after line 8 |

No board address, login or credential is written in this file.
