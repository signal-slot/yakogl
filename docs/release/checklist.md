<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
> Record landed from `ccfarm/integration` (merge `b98a03ab0`, ccfarm `01a12a25-0f00-703b-93fc-9f7ba52367dc`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# Release checklist (stage 3: public release)

> Status added when landed on main (2026-10-12): the "State at base" column is as at integration `89d518e` (2026-10-11), not at main. Several items it calls pending are named here by files that are only on the integration branch (`ROADMAP.md`, `docs/plan-status.md`, `docs/benchmarks.md`, `tools/license_audit*`); main has none of them. Re-derive each state before using the list.

Base: `89d518e` (the integration tip when this file was written).
Criterion (docs/plan-status.md, "Stage 3"): the repository is public with a
current CHANGELOG, a README, CONTRIBUTING.md and SECURITY.md, a licence/SPDX
audit with no unresolved finding, a redaction audit showing no board address,
credential or internal record in the tree or history, a website consistent
with the status sources, and the roadmap published.

State words: **met** = the check passes at the base; **partial** = some of it
is done; **open** = known defect; **pending** = the work has not landed.
The hash is the last commit that touched the proof, or the base if the line
is about the whole tree.

## Checklist

| # | Line | Measurable check | Proof (command or document) | State at base |
|---|---|---|---|---|
| 1 | CHANGELOG current | `CHANGELOG.md` has an `[Unreleased]` section; every merged item since `e6b84f7` is listed; it makes no "zero board failures" claim until the final ES2 trees are recorded | `git log e6b84f7..HEAD --oneline` against `CHANGELOG.md`; `grep -n -i "zero\|0 fail" CHANGELOG.md` | **partial**, `e6b84f7`. Written to that tip. Newer ES2 work (error rules, preprocessor, offline table, invariance) is not in it yet. Final entry waits for item es2-docs-final. |
| 2 | README status tables | The conformance table equals the latest recorded trees, board by board; the glmark2 table equals `docs/benchmarks.md` | `tools/cts/STATUS.md`, `tools/cts/RESULTS.md`, `tools/cts/log/board-sessions/am62p-es2-r1c.md`, `docs/benchmarks.md` | **open**, `f20d481`. README says 0 failures on AM62P. The AM62P record `f749bc7` found 47 failures at the tip (45 `texture.vertex.2d.{filtering,wrap}`, 2 ms4 constancy cases). README says Pi 4 has 25 cube-map failures, which the Pi 4 records do not yet confirm. i.MX8MP is also not remeasured. Fix after the final trees (es2-docs-final). |
| 3 | CONTRIBUTING.md | Build, test and gate commands in it run as written; it points to `tools/check.sh` | `tools/check.sh --quick`; read against `docs/release/building.md` | **met** (text), `c7425b4`. Brought up to date by pub-contrib-docs. The commands were read, not re-run for this line. |
| 4 | SECURITY.md | States how to report a problem and what is in scope; names no private address or person's mailbox that is not meant to be public | read `SECURITY.md`; owner confirms the contact address | **partial**, `b4ec427`. Draft is merged. The contact is an owner decision (see below). |
| 5 | DESIGN.md | Layer and module names in it exist in `src/` | `grep` each path named in `DESIGN.md` with `ls` | **met** (text), `c7425b4`. |
| 6 | README/CONTRIBUTING walkthrough | A new reader follows README then CONTRIBUTING on the host and reaches a passing `tools/check.sh --quick` without a gap | `docs/release/readme-walkthrough.md` | **pending**. The file is not at the base. The README walk belongs to plan #136. |
| 7 | Licence / SPDX audit | A tool lists every file without a valid `SPDX-License-Identifier` and every licence text missing from `LICENSES/`; zero open findings | `tools/license_audit*` and a report in `docs/release/*audit*` | **pending (plan 18)**, item release-license-audit. Neither exists at the base. Known input: `LICENSES/` has 8 texts, including two Slint and one Imagination DDK `LicenseRef-`; vendored code is the owner's question (below). |
| 8 | Redaction audit, tree and history | No board address, login, credential or internal record in the working tree or in any commit reachable from the branch to publish | an audit tool and a report in `docs/release/*audit*` | **pending (plan 18)**, item release-redaction-audit. Neither exists at the base. A first look found two private network addresses in `tools/cts/log/investigations/console-map.md` (`935f5fb`); the audit must decide whether to remove or rewrite them, and it must scan history too. |
| 9 | website/ consistent | Every number on `website/index.html` and `try-it.html` equals the status sources used in line 2 | read both pages against `tools/cts/STATUS.md` and `docs/benchmarks.md` | **open**, `fcb7998`. The page carries its own pass/fail table and benchmark figures, not yet checked against the new records. The refresh (the old pub-website work) was not merged. |
| 10 | Public ROADMAP.md | `ROADMAP.md` is in the tree with the five stages and their criteria; it matches `docs/plan-status.md` | `ls ROADMAP.md`; compare with `docs/plan-status.md` | **pending**. `ROADMAP.md` is not at the base. Item pub-roadmap is in review. |
| 11 | Gate is green | `tools/check.sh --quick` exits 0 on the tree to publish | run it | **met for this change** at the base plus this file (`tools/check.sh` is unchanged). The full `tools/check.sh` is not part of this item. |

Lines 1, 2 and 9 depend on stage 1 (zero ES2 failures on all three boards).
Do not close them before the final trees in items am62p-es2-final-a/b,
imx8mp-es2-final and pi4-es2-final-a are recorded.

## Decisions only the owner can make

| Decision | Why it is the owner's | Needed by |
|---|---|---|
| Promote the integration branch to `main` | `main` is the branch to publish; ccfarm does not push it | before lines 7, 8 are final |
| Make the repository public | Irreversible: copies and caches cannot be recalled | after all lines above are met |
| Licence for vendored code (Slint licences, Imagination DDK MIT text, and anything else line 7 finds) | A legal choice, not a code fix | before line 7 can reach zero |
| Publication date | Depends on the owner's plan and on stage 1 and 2 results | last |
| Security contact address in SECURITY.md | It becomes public | before line 4 |
| Whether to publish rewritten history or a fresh history, if line 8 finds a leak in old commits | Rewriting history changes every hash quoted in the docs | after line 8 |

## Not in this file

No board address, login or credential is written here. Line 8 refers to the
addresses by file only.
