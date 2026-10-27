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
| 1 | CHANGELOG current | `CHANGELOG.md` says what the project is and has at the tip, in one `[Unreleased]` section with no stale entry; its dEQP-GLES2 line equals line 13. With a fresh one-commit history (#1606, decision 1) there is no merged-change list to keep; the first version section and the tag are #1606 step 8 | `CHANGELOG.md`, "Results"; `tools/cts/RESULTS.md` | **met** (2026-10-26, main `5ac5ac23c` plus #1606 step 6). The section was rewritten as an inventory: results, what the library, compilers, boards, tools and docs have, known limits. Its dEQP-GLES2 line is line 13's (one tree per board at `e9901869a`, 0 Fail; AM62P 17,140, i.MX8MP 17,318, Pi 4 17,126) plus the later re-runs RESULTS.md gives (`25cce714`, `d0c5dc48`, `46f93767`, the same counts). Its glmark2 line gives the README table (`445c5653`) and the Pi 4's newer 195 (`c4af4917`); one commit for all three is #1196. Every path it names exists (checked with `ls`). The 0.1.0 section exists, dated 2026-10-27; the tag follows. |
| 2 | README status tables | The dEQP-GLES2 table equals the newest whole tree of each board (line 13); the glmark2 table equals the benchmark records; no row is composed of several commits | `README.md` "Status"; `tools/cts/RESULTS.md` | **met** (2026-10-16). The dEQP-GLES2 table reads 0 failures on the three boards at `e9901869a`; the glmark2 table gives AM62P 462 and Pi 4 188 at `e9901869a`, i.MX8MP 43 at `a63fcc5b` (no console at `e9901869a`). |
| 3 | CONTRIBUTING.md | The build, test and gate commands in it run as written on a fresh host; it names `tools/check.sh` | `docs/release/readme-walkthrough.md` (`5b83614c`), table "Checklist: what can change" | **partial**. README steps 4-6 fail as written (walk at `a63fcc5b`). N4 is decided (D6: rebase); N10 is documented in CONTRIBUTING.md step 1. Re-run evidence: "Line 3: re-run" below. |
| 4 | SECURITY.md | Names a reachable private contact, the scope and the response times; no `TODO(owner)` or placeholder left | `grep -n "example.invalid\|TODO(owner)" SECURITY.md` | **done**. Owner decision D5 (#1266): GitHub private vulnerability reporting (enabled when the repository goes public), no committed response time, only `main` supported until the first release. |
| 5 | DESIGN.md | Every path it names exists in the tree; every number in it equals the code or the records at the tip | `DESIGN.md`; the commands in the State column | **met** (2026-10-26, main `5ac5ac23c` plus #1606 step 6). Paths: every path of the layout block and the text exists (`ls` over the names, brace lists expanded); `src/mali/` and `src/osal/mali.rs` were missing from the layout and are added. Numbers: the dEQP-GLES2 paragraph now gives line 13 (`e9901869a`, 0 Fail, 17,140 / 17,318 / 17,126) instead of the old `32778b57` figures (walk finding N7); the layout pins are 38 structs and 590 fields (the distinct `size_of` and `offset_of` lines of `src/abi_layout.rs`; it said 35 and 563); the backend table has 21 entries (`src/backend.h`; it listed 9 as the whole table); `g_backend` names one of four tables (`src/backend.rs`); "no dependencies" is now "no runtime dependencies" (`Cargo.toml` has the build dependency `cc`); layer 5a has three implementations, not two. Also removed: the unexplained "SafeUI" (renamed away by #1224) and "F1". |
| 6 | README walkthrough | A new reader follows README then CONTRIBUTING on a host and reaches a passing `tools/check.sh --quick` with no gap | `docs/release/readme-walkthrough.md`; "Line 3: re-run" and "Line 6: re-read" below | **partial** (2026-10-26, main `5ac5ac23c` plus #1606 step 6). README findings 1-10 of the walkthrough are fixed (`grep -n "PowerVR build only\|Five are open" README.md` prints nothing; the toolchain argument is now `<your-toolchain.cmake>`), and the README host steps run (below). A fresh clone reaches a clean `tools/check.sh --quick` (line 3). Not met: the cross builds (README steps 4-6) need an aarch64 GCC or a newlib sysroot that this host does not have, and were not run; `CONTRIBUTING.md:52-56` still gives the old `--quick` size and the `MESA_V3D_TREE` failure that is now a gap. |
| 7 | Licence / SPDX audit | A tool lists every tracked file without a valid `SPDX-License-Identifier` (or a covering REUSE entry) and every named licence missing from `LICENSES/`; its report has zero open findings and every finding is fixed or decided by the owner | `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch` (check 25 of `tools/cts/cts-selftest.sh`); `docs/release/license-audit.md` | **met** (2026-10-26, main `b12657619` plus the origin pass of `docs/release/license-audit.md`, "Remaining findings"). `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch,unused-license` exits 0: 6431 files, `missing-spdx` 0, `unknown-id` 0, `third-party-mismatch` 0, `generated-mismatch` 0, `unused-license` 0, `undecided` 0, findings 0. Every file once parked as `undecided` has a row, sidecar or header naming the source its licence was read from, or left the tree (the two glmark2 screenshots; the Qt-derived `qpu_corpus.txt` was regenerated). Not checked by the tool: holders inside headers (NOTICE is kept by hand). Open for the owner: `undecided` is not yet a `--fail-on` class, so a new file could still be parked. |
| 8 | Redaction audit, tree and history | No host name, IP address, user name, login, credential, board serial number or internal farm record in the working tree, nor in any commit reachable from the branch to publish; the report names the scan, its patterns and every hit with its disposition | `docs/release/redaction-audit.md`, "Part 3" (its `git grep` per class) | **partial** (re-run 2026-10-27 on main `1ccb1a1b9` plus the step-5 change, #1606 step 5). Tree: met. After the scope cut (`tools/mali/board-ref/`, `docs/board-sessions/`) and the fixes of Part 3, the scan has no H or M finding: 0 private or tailnet addresses, 0 logins at a real host, 0 lab machine names, 0 SoM serials, 0 bridge ports, 0 tokens, keys or passwords; 0 home-path lines (the board image's default user in `driver/tools/kexec-zephyr/board-sudo-setup.sh:72-73` is now `${BOARD_USER:-...}`). Before: 774 home-path lines, 7 logins at a real host, 144 lab-name lines, 3 bridge ports. History: by decision 1 the public history is one fresh commit of the cleaned tree, so this line is met when the same scan passes on that snapshot commit, which does not exist yet. |
| 9 | website/ consistent | Every number on `website/index.html`, `status.html` and `try-it.html` (from `website/data.js`) equals line 2's sources; `tools/website-check.sh` exits 0 | `grep -n checkedAgainst website/data.js`; `bash tools/website-check.sh` | **open**. `data.js` (`0a7607b2`) says it was checked against `1cf946a9` (2026-10-11) and gives the AM62P 54 failures on `32778b57` and the i.MX8MP 0. `bash tools/website-check.sh` exits 1 at the base with 6 problems, all "commit ... is not a commit in this repository" in `data.js` (`d60b9d76`, `d8583f20`, `fe5e8e73`, `1fa941e3` twice, `031de9a2`): hashes of the history before `afb16335`, which README line 67 says the logs may quote but the checker refuses. The checker covers links, metadata, SPDX and commit names, not numbers. Item pub-website holds `website/`. |
| 10 | Public ROADMAP.md | `ROADMAP.md` is in the tree with the owner's five stages, a measurable criterion each, and State paragraphs read from `tools/cts/RESULTS.md`, `tools/cts/STATUS.md` and the open issues | `ROADMAP.md` | **met** (2026-10-26, main `5ac5ac23c` plus #1606 step 6). The five stages and their criteria are in the tree. The State paragraphs were re-read: stage 1 adds the later re-runs; stage 2 gives the `445c5653` table, the Pi 4's 195, what is left (pictures by eye on the Pi 4 and the i.MX8MP, #1196) and the slow scenes with their issues; stage 3 gives #1606's decisions and the open checklist lines; stage 4 the AM62P's 31-image table, the Pi 4's 9 of 91 images at `925dbca9` and the i.MX8MP not run; stage 5 the NanoPC-T6 Mali port (98 of 256 `gl_test` cases PASS on its Linux, 2026-10-24). Stage 1's heading no longer says "conformance". |
| 11 | Licence texts and NOTICE | `LICENSE` is Apache-2.0; every licence named in `NOTICE` has its text in `LICENSES/`; every vendored directory under `third_party/` is listed in `NOTICE` | `head -3 LICENSE`; `ls LICENSES/`; `NOTICE` lines 9-31 | **partial**. `LICENSE` is the Apache License 2.0; `LICENSES/` holds the 11 texts files name (Apache-2.0, BSD-2-Clause, BSD-3-Clause, BSL-1.0, CC-BY-ND-4.0, GPL-2.0-only, GPL-3.0-only, the two Slint texts, MIT, Zlib; 2026-10-26), and line 7's tool fails on a text no file names (owner decision 5 of #1606). The third_party listing is for line 7's tool to check, not checked here. |
| 12 | Gate is green | `tools/check.sh` (full) exits 0 on the commit to publish | `tools/check.sh`; CI as in `docs/ci.md` | **partial**. Not run in full for this line. The row that stopped the previous attempt's review, `cargo clippy -p yakogl-precompile --features gpu-vivante --all-targets`, printed no warning at the base with clippy 0.1.101 (2026-10-11), after `touch src/lib.rs`. Item clippy-gate-fix holds the clippy work in `src/gl/draw.rs` and `src/gl/texture.rs`. |
| 13 | Stage 1 met (input) | One whole dEQP-GLES2 tree per board, from one commit, newer than every GL-core change, reads 0 Fail over the 17,165 mustpass cases (AM62P with its ms4 image) | `tools/cts/RESULTS.md`, "ES 2.0" | **met** (2026-10-16). One tree per board at `e9901869a`, 0 Fail: AM62P 17,140 (1x and ms4), i.MX8MP 17,318, Pi 4 17,126 (issue #870). |

### Line 3: re-run (2026-10-25)

- A `MESA_V3D_TREE` without `build-v3d` is one `gap` (`tools/check.sh:2110`), no longer a failure; a gap keeps the gate red.
- `tools/check.sh --quick` (workspace checkout at the main tip `8198a56f` plus this change, host `MESA_V3D_TREE`): exit 0, "146 ok, 0 failed, 0 gap(s)" in 2747 s. An earlier run failed only `mali pan_shc` because the host's `build-pan` pointed at a moved path; `tools/mali/build_pan_shc.sh --update <MESA_PAN_TREE>` cleared it.
- "A docs-only change runs two rows" (CONTRIBUTING.md:16) holds: `tools/check.sh --changed origin/main` on a one-file docs diff exit 0, "2 ok, 0 failed, 0 gap(s), 174 skipped" in 5 s (`cargo fmt --check`, `sha256 third_party`).
- A fresh `git clone` of the checkout (`3e878c89`, no build directory), then `tools/check.sh --quick`: exit 0, "145 ok, 0 failed, 0 gap(s)" in 1491 s.
- Not done: a run with `MESA_V3D_TREE` set to a tree without `build-v3d` (the gap is read from the code, not run). Line 6 stays open: the README host steps were not re-run.

### Line 6: re-read (2026-10-26)

On main `5ac5ac23c` plus #1606 step 6, in a worktree with warm caches;
host Linux 7.2.8 x86-64, rustc 1.98.0-nightly (2026-07-01):

- `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux`: exit 0, 1 s.
  `cmake --build build`: exit 0, 36 s.
- `cargo test -p yakogl-tools`: exit 0, 119 passed, 23 s.
- `python3 -I tools/api_coverage.py`: exit 0, source fallback (no archive),
  186 declared, 304 exported, 304 resolvable, 0 declared but not exported.
- `git grep -n 'ignore = "BUG' -- src tests`: no match. README said two
  were open (#1289, #1292, both closed); it now says none on 2026-10-26.
- Also fixed in README.md: `build.rs` exists (README said "no build
  script"); `gpu-mali` is a fourth GPU feature; the PowerVR driver programs
  still made by Mesa's pco are the two tile-buffer end-of-tile programs and
  the self-test's clear shader (`src/shaderc/SHADERC_NOTES.md`), not all
  end-of-tile programs and the self-test's vertex shader; the image sizes
  carry their date (2026-10-06); the `--quick` size and times are line 3's
  (145 and 146 rows, 1491 s and 2747 s); "conformance" is not used as a claim.
- Not run: README steps 4-6 (no `aarch64-none-elf-gcc` and no newlib sysroot
  on this host), the Zephyr build, the host-harness compare, and the cargo
  test rows (line 3's `--quick` runs them).
- Left for another change: `baremetal.mk:73` defaults `DRVSRC` to `../src`,
  the old two-repository layout, so README tells the reader to pass
  `DRVSRC=driver/src`; the README glmark2 table is #1196.

## Decisions only the owner can make

| # | Decision | Why it is the owner's | Needed by |
|---|---|---|---|
| D1 | Licence of each component: YakoGL's own code (Apache-2.0 today, `LICENSE`); the Mesa-derived parts (MIT); the Imagination DDK material (MIT with the GPLv2 alternative); the NXP `hal_nxp` part (BSD-3-Clause); the Slint parts (GPL-3.0 / Slint Royalty-free 2.0 / Slint Software 3.0, which one applies to the samples); the Zephyr PR snapshot; and whatever line 7 finds. One licence per directory, stated in `LICENSE`/`NOTICE`: library (`src/`, `capi/`), kernel driver (`driver/`; `LICENSES/GPL-2.0-only.txt` exists), `third_party/`, `tools/`, `website/`, `zephyr/` | A legal choice; the audit only reports, and line 7 can only check files against that choice | before line 7 can reach zero |
| D2 | Data files without an SPDX line: decided on 2026-10-20 (#1265): own source gets a header, generated output, captures, test data and images go into `tools/license_audit.allow` by path or kind. Still open: the 580 `undecided` files (Qt and femtovg shaders, patches, images; `docs/release/license-audit.md`, "Remaining findings") | Upstream-derived or unknown-origin files get no licence guessed | before line 7 can reach "no open finding" |
| D3 | History: publish the existing history, a rewritten one (filter out what line 8 finds), or a fresh single-commit history | Rewriting changes every hash quoted in the docs; a fresh history loses provenance | after line 8 |
| D4 | What the README conformance table claims (`README.md` lines 37-66: per-board dEQP-GLES2 results, "17,165-case mustpass list"): per-board whole-tree counts from one commit each (as line 13 asks), the newest reading per row, or the failures listed explicitly; and whether a board with a known open regression may be published as "close to complete" | It is the public claim of the project | before lines 2, 5 and 9 close |
| D5 | Security contact address, response times and supported versions in SECURITY.md | They become public | **decided** 2026-10-20 (#1266): GitHub private vulnerability reporting, no committed response time, `main` only |
| D6 | Merge or rebase rule for contributors (walk finding N4: CONTRIBUTING.md and the project settings disagree) | A project policy | **decided** 2026-10-20 (#1267): rebase onto origin/main, `--force-with-lease` on one's own topic branch |
| D7 | Promote the branch to publish to `main` and make the repository public | Irreversible: copies and caches cannot be recalled | after every line above is met |
| D8 | Publication date | Depends on stage 1 and stage 2 | last |

No board address, login, user name or credential is written in this file.
