<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Release checklist (stage 3: public release)

Base: `7ae09845` (integration tip, 2026-11-05). Lines 6, 9, 11 and 12 were
read at this base; the other lines carry the date and commit of their own
reading in their State cell. This version replaces the one written at
`40dd210` (2026-10-12).

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
| 1 | CHANGELOG current | `CHANGELOG.md` says what the project is and has at the tip, in one `[Unreleased]` section with no stale entry; its dEQP-GLES2 line equals line 13. With a fresh one-commit history (#1606, decision 1) there is no merged-change list to keep; the first version section and the tag are #1606 step 8 | `CHANGELOG.md`, "Results"; `tools/cts/RESULTS.md` | **met** (2026-11-16, main `badc4a13`; `[Unreleased]` last changed at `b4a616f8`). One `[Unreleased]` section; every backticked path exists (`ls`, brace lists expanded; bare file names resolve to one tracked file each, `texture.rs` to three) and the section matches `tools/cts/RESULTS.md`, including the NanoPC-T6 dEQP-GLES2 whole-tree row (`37305608`, 17,165 cases, 5,456 Pass, 10,700 Fail, 973 NotSupported, 33 ResourceError, 3 QualityWarning, 0 Lost, `1db8fd3d`; four slice logs under `tools/cts/log/`). Its dEQP-GLES2 line is line 13's reading plus the re-runs RESULTS.md reads; line 13's own cell is #345's. `[0.1.0]` stands, dated 2026-10-27; the first version section and the tag are #1606 step 8. |
| 2 | README status tables | The dEQP-GLES2 table equals the newest whole tree of each board (line 13); the glmark2 table equals the benchmark records; no row is composed of several commits | `README.md` "Status"; `tools/cts/RESULTS.md` | **met** (2026-10-16). The dEQP-GLES2 table reads 0 failures on the three boards at `e9901869a`; the glmark2 table gives AM62P 462 and Pi 4 188 at `e9901869a`, i.MX8MP 43 at `a63fcc5b` (no console at `e9901869a`). |
| 3 | CONTRIBUTING.md | The build, test and gate commands in it run as written on a fresh host; it names `tools/check.sh` | `docs/release/readme-walkthrough.md` (`5b83614c`), table "Checklist: what can change" | **met** (2026-11-12, fresh clone of `570cd8c6`, `--quick` "181 ok, 0 failed, 0 gap(s)" in 3121 s; entry "2026-11-12: fresh clone of `570cd8c6`" under "Line 3: re-run (2026-11)"). The previous walk (2026-11-10, `c73c0664`) was partial only because of the `redaction_scan` false alarm, fixed by `b9234557`. |
| 4 | SECURITY.md | Names a reachable private contact, the scope and the response times; no `TODO(owner)` or placeholder left | `grep -n "example.invalid\|TODO(owner)" SECURITY.md` | **done**. Owner decision D5 (#1266): GitHub private vulnerability reporting (enabled when the repository goes public), no committed response time, only `main` supported until the first release. |
| 5 | DESIGN.md | Every path it names exists in the tree; every number in it equals the code or the records at the tip | `DESIGN.md`; the commands in the State column | **met** (2026-11-16, main `badc4a13`; `DESIGN.md` last changed at `c0d59a39`, full repository paths). Every backticked path exists, the `src/osal/` brace list expanded. DESIGN.md states no struct, field or backend-table counts, so none can drift (`src/abi_layout.rs` has 51 `size_of::<` and 656 `offset_of!` lines; the table is `struct yakogl_backend` in `src/backend.h`). What it states holds: four tables behind `g_backend` (`src/backend.rs`), the three `GLES_HEAP_*` heaps, layer 5a's three implementations (`src/shaderc/online/`, `src/shaderc/precompiled.rs`, `src/shaderc/offline.rs`), "no runtime dependencies". |
| 6 | README walkthrough | A new reader follows README then CONTRIBUTING on a host and reaches a passing `tools/check.sh --quick` with no gap | `docs/release/readme-walkthrough.md`; "Line 3: re-run" and "Line 6: re-read" below | **met** (2026-11-07, base `40eb8d00`; clang form at `14e93e64`, GCC form at `22265e87`). The criterion is that a new reader following README then CONTRIBUTING reaches a passing `tools/check.sh --quick` with no gap: a fresh clone does (line 3, 170 ok, 0 failed, 0 gap(s) at `14e93e64`), and both toolchain forms of README step 3 now build as README writes them (subsection "Line 6: clang form" below: clang exit 0 in 98.80 s, GCC exit 0 in 26.8 s, the CMake aarch64 build exit 0 on 2026-11-05). As of 2026-11-08 (base `cde4864d`), `baremetal.mk` defaults `DRVSRC` to `driver/src`, README's commands no longer pass it, and the GCC form exits 0 without it at `ccc51125`. `CONTRIBUTING.md` gives the `--quick` size and the `MESA_V3D_TREE` gap. |
| 7 | Licence / SPDX audit | A tool lists every tracked file without a valid `SPDX-License-Identifier` (or a covering REUSE entry) and every named licence missing from `LICENSES/`; its report has zero open findings and every finding is fixed or decided by the owner | `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch` (check 25 of `tools/cts/cts-selftest.sh`); `docs/release/license-audit.md` | **met** (2026-10-26, main `b12657619` plus the origin pass of `docs/release/license-audit.md`, "Remaining findings"). `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch,unused-license` exits 0: 6431 files, `missing-spdx` 0, `unknown-id` 0, `third-party-mismatch` 0, `generated-mismatch` 0, `unused-license` 0, `undecided` 0, findings 0. Every file once parked as `undecided` has a row, sidecar or header naming the source its licence was read from, or left the tree (the two glmark2 screenshots; the Qt-derived `qpu_corpus.txt` was regenerated). Not checked by the tool: holders inside headers (NOTICE is kept by hand). Re-run 2026-11-07 at `40eb8d00`: `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch` exits 0 (3914 files, 1843 with an SPDX line, 1863 allowlisted, findings 0); the same with `,undecided` appended exits 0 (`undecided` 0, `unused-license` 0, findings 0); `--self-test` passes. The flag `--fail-on undecided` already exists and passes. Whether the gate commands (check 25 of `tools/cts/cts-selftest.sh` and this line's command) include it is the owner's decision, set out in the plan #326 issue draft "license audit: make `undecided` a gate --fail-on class" (not yet filed). |
| 8 | Redaction audit, tree and history | No host name, IP address, user name, login, credential, board serial number or internal farm record in the working tree, nor in any commit reachable from the branch to publish; the report names the scan, its patterns and every hit with its disposition | `docs/release/redaction-audit.md`, "Part 4" (the scan tool's run; Part 3 has the classes) | **partial** (re-run 2026-11-07 with `tools/release/redaction-scan.py` on `a53b96b0`, `docs/release/redaction-audit.md` Part 4; earlier: 2026-10-27, main `1ccb1a1b9` plus the step-5 change, #1606 step 5). Tree: 0 H/M findings in the public classes (0 private or tailnet addresses, 0 logins at root, 0 SoM serials, 0 bridge ports, 0 tokens, keys or passwords; the 12 farm-worker home paths of the Pi 4 tallies are fixed and the preflight canary is a dropped look-alike). The owner-login, lab-host and private-login classes are `not run` without the private file, so the exit is 3: not a pass. History: by decision 1 the public history is one fresh commit of the cleaned tree, so this line is met when the snapshot commit passes `YAKOGL_REDACTION_PRIVATE_FILE=<file outside the repo> python3 -I tools/release/redaction-scan.py <snapshot>` with exit 0 (exit 3 means the private classes did not run: not a pass; the file holds the owner's login, the lab and runner names, the serial-bridge hosts and the device resource ids); the snapshot commit does not exist yet. |
| 9 | website/ consistent | Every number on `website/index.html`, `status.html` and `try-it.html` (from `website/data.js`) equals line 2's sources; `tools/website-check.sh` exits 0 | `grep -n checkedAgainst website/data.js`; `bash tools/website-check.sh` | **partial** (2026-11-05, base `7ae09845`). `bash tools/website-check.sh` exits 0: "website-check: ok (5 pages, 147 links and sources, 19 figures)". Item website-check-fix cleared the 6 (later 11) provenance problems and website-data-deqp (`755bd183`) moved the dEQP and status figures to RESULTS.md at main. The checker covers links, metadata, SPDX, commit names and the figures it has a source for. Not met: the glmark2 figures in `website/data.js` still follow the old table; they move with item website-data-refresh after the stage-2 one-commit run (line 2) lands, so the line stays partial until then. No gate row runs `website-check.sh`. |
| 10 | Public ROADMAP.md | `ROADMAP.md` is in the tree with the owner's five stages, a measurable criterion each, and State paragraphs read from `tools/cts/RESULTS.md`, `tools/cts/STATUS.md` and the open issues | `ROADMAP.md` | **met** (2026-11-16, main `badc4a13`; `ROADMAP.md` last changed at `badc4a13`, `docs: roadmap stages 3 and 5 read at e6e68ff7`). The five stages and their criteria are in the tree. The header says stages 3 and 5 were read on 2026-11-16 at `e6e68ff7`, and stages 1, 2 and 4 on 2026-11-05 at `08163414`, not re-read (stage 1 and 4 wording is #345's, stage 2's #347's). Stage 3 has no "being fixed": it says the `rust_eh_personality` failure is fixed (`fce3e6aa`, `2e949ed5`) and the last green full run is `c66bea9f` (222 ok, `4946e879`), for that commit only, with the gate re-run on the snapshot still open (line 12 itself is #344's). Stage 5 gives the NanoPC-T6 dEQP-GLES2 whole-tree row of `37305608` (5,456 Pass of 17,165, `1db8fd3d`), as RESULTS.md and CHANGELOG do. |
| 11 | Licence texts and NOTICE | `LICENSE` is Apache-2.0; every licence named in `NOTICE` has its text in `LICENSES/`; every vendored directory under `third_party/` is listed in `NOTICE` | `head -3 LICENSE`; `ls LICENSES/`; `NOTICE` lines 9-31 | **met** (2026-11-16, main `badc4a13`; `NOTICE` last changed at `40d11d45`). `head -3 LICENSE`: the Apache License Version 2.0. `ls LICENSES/`: 11 texts (Apache-2.0, BSD-2-Clause, BSD-3-Clause, BSL-1.0, CC-BY-ND-4.0, GPL-2.0-only, GPL-3.0-only, the two Slint texts, MIT, Zlib), the set the audit reads as used. `NOTICE` names all three directories of `third_party/` (the loop over `third_party/*/` prints nothing). `python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch` exits 0: 10623 files, 3927 with an SPDX line, 6088 allowlisted, `missing-spdx` 0, `unknown-id` 0, `undecided` 0, `unused-license` 0, `third-party-mismatch` 0, `generated` 76 with mismatch 0, findings 0 (13 `third-party-unlabelled` files, not a `--fail-on` class). |
| 12 | Gate is green | `tools/check.sh` (full) exits 0 on the commit to publish | `tools/check.sh`; CI as in `docs/ci.md` | **partial** (2026-11-14, `760aad3d`): full `tools/check.sh --jobs 2` exit 1 after 12734 s, with no summary line (see "Line 12" below). 206 rows ok, 0 gap(s), and 9 rows failed for one code defect: every `test host-test,...` row fails `tests/redaction_scan.rs` `redaction_scan_finds_nothing_in_this_tree` on 6 H/M findings (literal private IPv4, IPv6 link-local and tailnet host fixtures in `tools/mp2_uboot_record_check.py`'s redaction self-test, added at `3e5ca63c`). The host's disk then filled at the Zephyr image rows: the remaining 15 rows and the summary line failed with ENOSPC, so they are a host failure, not a result. The last green full run is `c66bea9f` (2026-11-11, 222 ok, 0 failed, 0 gap(s)). Green means "N ok, 0 failed, 0 gap(s)" from a full run on the commit to publish, so it is re-run there. The nightly `__rustc::rust_eh_personality` link failure no longer occurs: `fce3e6aa` documents how yakogl resolves it and `2e949ed5` is the pvr check side. Without `MESA_V3D_TREE` (a Mesa tree with `build-v3d`) the three v3d rows are one `gap`, and without `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC` or an aarch64 compiler the Zephyr, hal_nxp, pco_shc and aarch64 rows are `gap`s; none was a gap in the `760aad3d` run up to the disk-full point. |
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
- Until `9c736975`, `baremetal.mk:73` defaulted `DRVSRC` to `../src`, the
  old two-repository layout, and README told the reader to pass
  `DRVSRC=driver/src`; since `9c736975` the default is `driver/src`
  (`baremetal.mk:73`) and README no longer passes it. The README glmark2
  table is #1196.

### Line 3: re-run (2026-11)

- Walked: a fresh clone of origin/main `14e93e64`, 2026-11-06 (commit `340db7f1` wrote the result into CONTRIBUTING.md).
- `tools/check.sh --quick` (CONTRIBUTING.md): "170 ok, 0 failed, 0 gap(s)" in 2309 s with two compiler slots. The plan has 211 rows (210 ok and 1 gap on a host without `PCO_SPM_LOAD_CAPTURE`).
- Second walk, 2026-11-07, a worktree of origin/main `8e81ea9e` (a symlinked path, not a canonical one): `tools/check.sh --quick` is red, 166 ok, 3 failed, 1 gap(s), on a 4-core host. Cause: the symlinked path and the missing `MALI_CSFFW` file, not the code. This is not a pass; the green record above (`14e93e64`, a fresh clone at a canonical path with two compiler slots) is the last one.
- Not recorded here: the per-step exit codes and times of the README steps and of the other CONTRIBUTING.md commands in the 2026-11-07 walk. The walk's step tables did not reach this record, so none are stated; they stay open for line 3.
- To close line 3: run `tools/check.sh --quick` from a canonical path with `MALI_CSFFW` fetched (about 90 minutes on a 4-core host) and record each step's exit code and time as it runs.
- README steps 2 and 3 were run separately on 2026-11-06/07: see "Line 6: clang form".

#### 2026-11-08: fresh clone of `ccc51125`, every step recorded

`tools/check.sh --quick` as written: "170 ok, 0 failed, 1 gap(s)"; from
the fixed CONTRIBUTING.md text: "171 ok, 0 failed, 0 gap(s)".
A `git clone` of origin/main `ccc51125` into a canonical path (`realpath`
equal to the path, no symbolic link on it), host Linux 7.0 x86-64, 4 cores,
"2 compiler slot(s)" (check.sh), rustc 1.101.0-nightly (2026-10-16), clang
21.1.8, GCC 15.2.0. `MESA_V3D_TREE` unset. The host environment carried two
stale values from its project settings: `HAL_NXP_DIR` and
`DEQP_GLES3_TEST_DIR` named directories that do not exist, while
`tools/cts/deps-env.sh --check` reports every entry of the layout `ok`
(`MALI_CSFFW` included). Commands ran in the clone's root in this order;
times are wall seconds of the whole command.

| # | Step | Command | Exit | Time |
|---|---|---|---|---|
| 1 | clone | `git clone https://github.com/signal-slot/yakogl.git` | 0 | 4.0 |
| 2 | README, Rust version | `rustc --version` | 0 | 0.0 |
| 3 | README, host library | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 0.3 |
| 4 | README, host library | `cmake --build build` | 0 | 34.1 |
| 5 | README, cross build | `cmake -S . -B build/a64 --toolchain <own file: Zephyr SDK 1.0.1 aarch64-zephyr-elf GCC 14.3.0> -DYAKOGL_OSAL=baremetal` | 0 | 0.2 |
| 6 | README, cross build | `cmake --build build/a64` | 0 | 38.3 |
| 7 | README, bare metal | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-` | **2** | 1.0 |
| 7' | the same, the host's aarch64 GCC | `make -f baremetal.mk CROSS_COMPILE=<Zephyr SDK>/aarch64-zephyr-elf- BUILD=build/bm-zsdk` | 0 | 18.7 |
| 8 | README, bare metal, clang | `make -f baremetal.mk CC='clang --target=aarch64-none-elf --sysroot=<sysroot>' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy` | **2** | 1.4 |
| 8' | the same, versioned LLVM names | `... AR=llvm-ar-21 NM=llvm-nm-21 OBJCOPY=llvm-objcopy-21` | 0 | 18.8 |
| 9 | README, Zephyr | (a workspace from `west.yml`) | not run | |
| 10 | README, API coverage | `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table` | 0 | 10.2 |
| 11 | README, API coverage | `tools/api_coverage.py` | 0 | 0.1 |
| 12 | README, testing | `tools/check.sh --changed origin/main` | 0 | 4.7 |
| 13 | README, testing | `tools/check.sh --quick` | **1** | 2253.1 |
| 14 | README, testing | `cargo test --no-default-features --features host-test,gpu-powervr` | 0 | 43.7 |
| 15 | README, testing | `... host-test,gpu-powervr,report-es2` | 0 | 37.2 |
| 16 | README, testing | `... host-test,gpu-vivante` | 0 | 40.3 |
| 17 | README, testing | `... host-test,gpu-powervr,shaderc-online` | **101** | 101.3 |
| 18 | README, testing | `... host-test,gpu-vivante,shaderc-online` | **101** | 103.0 |
| 19 | README, testing | `cargo test -p yakogl-tools` | 0 | 9.3 |
| 20 | README, open defects | `git grep -n 'ignore = "BUG' -- src tests` | 1 (no match: none open) | 0.0 |
| 21 | README, host harness | `HH_TIMEOUT=60 tools/hostharness/compare.sh ...` | not run | |
| 22 | CONTRIBUTING step 1 | `tools/check.sh --changed origin/main` | 0 | 6.4 |
| 23 | CONTRIBUTING, branches | `git rebase origin/main` | 0 | 0.0 |
| 24 | CONTRIBUTING, the gate | `tools/check.sh --quick` (warm) | **1** | 852.2 |
| 25 | CONTRIBUTING, the gate | `YAKOGL_CHECK_PLAN=1 tools/check.sh --quick` | 1 (the plan holds the gap row) | 0.6 |
| 26 | CONTRIBUTING, the gate | `tools/check.sh --local origin/main` | 0 | 6.2 |
| 27 | CONTRIBUTING, the gate | `tools/check_scope_selftest.sh` | 0 | 28.9 |
| 28 | CONTRIBUTING, firmware | `tools/cts/deps-env.sh --build MALI_CSFFW` | 0 (`present`) | 0.1 |
| 29 | CONTRIBUTING, fixed text | `tools/cts/deps-env.sh --check` | 0 | 0.2 |
| 30 | CONTRIBUTING, fixed text | `export $(tools/cts/deps-env.sh); tools/check.sh --quick` | 0 | 1010.1 |
| 31 | README 17, re-run | `export $(tools/cts/deps-env.sh); cargo test ... host-test,gpu-powervr,shaderc-online` | 0 | 135.7 |
| 32 | README 18, re-run | `export $(tools/cts/deps-env.sh); cargo test ... host-test,gpu-vivante,shaderc-online` | 0 | 33.1 |
| 33 | README and CONTRIBUTING, full gate | `export $(tools/cts/deps-env.sh); tools/check.sh` | **1** | 3342.3 |
| 34 | CONTRIBUTING, fixed text | `mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/` (layout exported) | 0 | 0.2 |
| 35 | CONTRIBUTING, fixed text | `export $(tools/cts/deps-env.sh); tools/check.sh` | 0 | 2625.9 |

- `--quick` as written (steps 13 and 24): "170 ok, 0 failed, 1 gap(s)". The
  gap is `imx8mp regs.rs: no hal_nxp at <the stale HAL_NXP_DIR> (set
  HAL_NXP_DIR)`: `tools/check.sh` reads `HAL_NXP_DIR`, `ZEPHYR_WS` and
  `PCO_SHC` from the environment only (only `MALI_CSFFW` falls back to the
  layout), and CONTRIBUTING.md said these "need setup" without saying how.
  Its wording now gives `export $(tools/cts/deps-env.sh)`.
- `--quick` from the fixed text (step 30): "171 ok, 0 failed, 0 gap(s)" in
  1010 s (a warm tree; step 13, the cold run, took 2253 s). The plan is 171
  rows at `ccc51125` (step 25), one more than CONTRIBUTING.md's 170.
- Steps 17 and 18: the `es3_shaderc_corpus` test `corpus` panics with
  "DEQP_GLES3_TEST_DIR is readable: ... No such file or directory": a stale
  `DEQP_GLES3_TEST_DIR` fails the test (unset, it skips). With the layout
  exported both pass (steps 31 and 32: 3515 and 3408 passed, 0 failed).
- Steps 7 and 8 fail on the host's tool names, which README's own comments
  name: no `aarch64-none-elf-gcc` on `PATH`, and LLVM installed only as
  `llvm-ar-21` and the like. With this host's names both build (7', 8').
  The sysroot of 8' is the Zephyr SDK's (picolibc), not a newlib one;
  `baremetal.mk` builds without `DRVSRC`.
- Full gate (step 33): "208 ok, 0 failed, 5 gap(s)". All five gaps are the
  am62p Zephyr images: "driver/src/fw/pvr_fw_blob.h is missing (make it with
  driver/tools/mkfwblob.sh): not built". The layout holds the header
  (`PVR_FW_BLOB`), and the CI copies it into `driver/src/fw/`
  (`.github/workflows/gate.yml`); CONTRIBUTING.md did not say so. Its
  firmware section now gives the copy. From the fixed text (steps 34, 35):
  "213 ok, 0 failed, 0 gap(s)" in 2626 s.
- Not run: README's Zephyr build (step 9) and host-harness compare (step 21,
  40 to 60 minutes and up to 50 GB), `tools/board.sh`, `driver/tools/mkfwblob.sh`
  (TI's firmware), `git push --force-with-lease`.

#### 2026-11-09: fresh clone of `e320630d`, clean environment (partial: `--quick` not finished)

A `git clone` of origin/main `e320630d` into a canonical path, host Linux 7.0
x86-64, 4 cores, rustc 1.101.0-nightly (2026-10-16), clang 21.1.8. The
environment had `HAL_NXP_DIR`, `DEQP_GLES3_TEST_DIR`, `MESA_V3D_TREE` and the
other layout variables unset; no Arm GNU toolchain was on `PATH` (the download
from developer.arm.com was refused to the agent, so it was not fetched) and the
LLVM tools exist only versioned. Commands ran in the clone's root; times are
wall seconds. The worker restarted during `tools/check.sh --quick` and the
clone was lost, so that step has no summary line and a second walk at `3fa1e64d`
produced no result.

| # | Step | Command | Exit | Time |
|---|---|---|---|---|
| 1 | README, Rust version | `rustc --version` | 0 | 0.0 |
| 2 | README, host library | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 0.4 |
| 3 | README, host library | `cmake --build build` | 0 | 49.2 |
| 4 | README, bare metal | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-` | **2** | 1.1 |
| 5 | README, bare metal, clang | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy` (no sysroot, none on the host) | **2** | 0.3 |
| 6 | README, API coverage | `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table` | 0 | 10.2 |
| 7 | README, API coverage | `tools/api_coverage.py` | 0 | 0.2 |
| 8 | README, testing | `tools/check.sh --changed origin/main` | 0 | 5.4 |
| 9 | README, testing | `cargo test ... host-test,gpu-powervr` | 0 | 60.6 |
| 10 | README, testing | `... host-test,gpu-powervr,report-es2` | 0 | 54.0 |
| 11 | README, testing | `... host-test,gpu-vivante` | 0 | 56.1 |
| 12 | README, testing | `... host-test,gpu-powervr,shaderc-online` (clean env) | 0 | 149.3 |
| 13 | README, testing | `... host-test,gpu-vivante,shaderc-online` (clean env) | 0 | 140.4 |
| 14 | README, testing | `cargo test -p yakogl-tools` | 0 | 61.5 |
| 15 | README, open defects | `git grep -n 'ignore = "BUG' -- src tests` | 1 (no match) | 0.0 |
| 16 | CONTRIBUTING, firmware | `tools/cts/deps-env.sh --build MALI_CSFFW` | 0 | 0.1 |
| 17 | CONTRIBUTING, layout | `tools/cts/deps-env.sh --check` | 0 | 0.1 |
| 18 | CONTRIBUTING, the gate | `export $(tools/cts/deps-env.sh); tools/check.sh --quick` | not finished | |

- A clean environment removes failures (a) and (b) of the 2026-11-08 walk:
  both `shaderc-online` commands exit 0 (steps 12, 13).
- Steps 4 and 5 are the host: no `aarch64-none-elf-gcc` and no newlib sysroot
  on this worker. Not a wording fault of README.md.
- Not run: the CMake cross build (needs a toolchain file of the reader's own),
  Zephyr, the host harness, `tools/check.sh --local`,
  `tools/check_scope_selftest.sh`.

#### 2026-11-10: fresh clone of `8a5da05e`, Arm GNU toolchain on `PATH` (partial: 1 failed, 1 gap)

A `git clone` of origin/main `8a5da05e` (over HTTPS; main had moved to
`dfae7366` by the time of writing, checklist.md unchanged between them) into a
canonical path inside the workspace (`realpath` equal to the path, no symbolic
link), host Linux 7.0 x86-64, 4 cores, "2 compiler slot(s)". Toolchain: the
Arm GNU Toolchain 14.3.rel1 x86_64-hosted aarch64-none-elf, fetched from
developer.arm.com (sha256 `ebaf2d47...3d2be9` equals Arm's `.sha256asc`), on
`PATH`: `aarch64-none-elf-gcc (Arm GNU Toolchain 14.3.Rel1 (Build arm-14.174))
14.3.1 20250623`; `Ubuntu clang version 21.1.8`; `llvm-ar-21`, `llvm-nm-21`,
`llvm-objcopy-21`; rustc 1.101.0-nightly (2026-10-16). `HAL_NXP_DIR`,
`DEQP_GLES3_TEST_DIR`, `MESA_V3D_TREE`, `ZEPHYR_WS` and `PCO_SHC` were
unset. Commands ran in the clone's root; `build/bm` was removed before each
`baremetal.mk` form; times are wall seconds. The clang form is README's line
with `<arm-gnu>` replaced by the toolchain directory and the versioned tool
names README mentions.

| # | Step | Command | Exit | Time |
|---|---|---|---|---|
| 1 | README, Rust version | `rustc --version` | 0 | 0.4 |
| 2 | README, host library | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 0.6 |
| 3 | README, host library | `cmake --build build` | 0 | 54.3 |
| 4 | README, bare metal | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-` | 0 | 23.9 |
| 5 | README, bare metal, clang | `make -f baremetal.mk CC='clang --target=aarch64-none-elf --sysroot=<arm-gnu>/aarch64-none-elf' AR=llvm-ar-21 NM=llvm-nm-21 OBJCOPY=llvm-objcopy-21` | 0 | 21.5 |
| 6 | README, API coverage | `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table` | 0 | 10.0 |
| 7 | README, API coverage | `tools/api_coverage.py` | 0 | 0.2 |
| 8 | README, testing | `tools/check.sh --changed origin/main` | 0 | 7.0 |
| 9 | README, testing | `cargo test ... host-test,gpu-powervr` | 0 | 97.6 |
| 10 | README, testing | `... host-test,gpu-powervr,report-es2` | 0 | 90.4 |
| 11 | README, testing | `... host-test,gpu-vivante` | 0 | 82.0 |
| 12 | README, testing | `... host-test,gpu-powervr,shaderc-online` | 0 | 234.3 |
| 13 | README, testing | `... host-test,gpu-vivante,shaderc-online` | 0 | 240.4 |
| 14 | README, testing | `cargo test -p yakogl-tools` | 0 | 61.2 |
| 15 | README, open defects | `git grep -n 'ignore = "BUG' -- src tests` | 1 (no match) | 0.0 |
| 16 | CONTRIBUTING, firmware and corpus | `tools/cts/deps-env.sh --build MALI_CSFFW DEQP_GLES3_TEST_DIR PCO_SPM_LOAD_CAPTURE` | 0 | 105.0 |
| 17 | CONTRIBUTING, layout | `tools/cts/deps-env.sh --check` | 0 | 0.2 |
| 18 | CONTRIBUTING, firmware header | `export $(tools/cts/deps-env.sh); mkdir -p driver/src/fw && cp "$PVR_FW_BLOB" driver/src/fw/` | 0 | 0.1 |
| 19 | CONTRIBUTING, host check | `export $(tools/cts/deps-env.sh); tools/setup-gate-host.sh --check` | **1** (`lz4` missing; stale `CC_aarch64_unknown_none`) | 0.8 |
| 20 | CONTRIBUTING, the gate | `export $(tools/cts/deps-env.sh); tools/check.sh --quick` (host environment as it was) | **1** | 2955.4 |
| 21 | CONTRIBUTING, the gate | the same with `CC_aarch64_unknown_none` unset (warm tree) | **1** | 1379 |

- Step 20: `check.sh: 172 ok, 4 failed, 1 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 not run (passed on main) in 2955s`. Three of the four
  failures (the two aarch64 clippy rows, `osal-zephyr,gpu-powervr,precompiled-shaders`) and
  "no Mesa reference in the artefact" are one host fault: the worker's project settings export
  `CC_aarch64_unknown_none` naming a Bootlin toolchain directory that does not exist
  (`...is not an executable`; `setup-gate-host.sh --check` says so).
- Step 21 (the variable unset): `check.sh: 175 ok, 1 failed, 1 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 not run (passed on main) in 1378s`.
  The failed row is `test yakogl-mali-panthor`: 43 of 44 pass, `gltest_guard::the_verdict_line_is_out_before_the_next_case_starts`
  panics at `tools/mali/panthor/tests/gltest_guard.rs:152` ("case 2 started before case 1's verdict was out"); a timing test, not rerun in isolation here. The gap is
  `am62p second stage: ... no lz4 command`; the worker has no `lz4` (a package install, not done).
  No `MALI_CSFFW` gap: the firmware came from the layout.
- Steps 4 and 5, which failed on 2026-11-09 for want of the toolchain, exit 0 as written
  (only the placeholders and the versioned LLVM names substituted). Every README and
  CONTRIBUTING.md step except step 19 and the `--quick` rows exits 0; no wording fault found.
- Not run: the CMake cross build with a toolchain file, Zephyr, the host harness,
  `tools/check.sh --local`, `tools/check_scope_selftest.sh`.

#### 2026-11-10: fresh clone of `c73c0664`, `--quick` with lz4 (partial: 1 failed)

Item line3-quick, a fresh `git clone` at a real path, `lz4` v1.10.0 present,
`CC_aarch64_unknown_none` unset, `MESA_V3D_TREE` unset, layout exported,
`tools/setup-gate-host.sh --check` exit 0.
`tools/check.sh --quick`: "176 ok, 1 failed, 0 gap(s)" in 2948 s. The `am62p second stage` gap
is gone (lz4) and `gltest_guard` passed. The failed row is `host-test,gpu-powervr`:
`redaction_scan` flags the fixture path in `tools/cts/mali-glmark2-session-selftest.sh`
(added by `556ff8c9`); a false alarm of the scanner's fixture list in
`tools/release/redaction-scan.py`, not the host. Fix named for another plan: add
`("home-path", "tools/cts/mali-glmark2-session-selftest.sh", ["/ho", "me/yakogl-mali-glmark2"], ["/ho", "me/yakogl-other"])`
beside the `mali-glmark2-selftest.sh` entry (about line 810). Until it lands, `--changed origin/main`
reaches the same row. Also seen: a fresh clone has no `driver/src/fw/`, so the "PowerVR header copy"
fails with `cp: cannot create regular file` until `mkdir -p driver/src/fw`.

#### 2026-11-12: fresh clone of `570cd8c6` (met)

A fresh `git clone` of origin/main at `570cd8c6`, which contains the
redaction-scan fixture fix `b9234557`, followed README.md and then
CONTRIBUTING.md as written. Every step exited 0, apart from two expected
non-zero exits. `tools/check.sh --quick`: "181 ok, 0 failed, 0 gap(s)" in
3121 s. Line 3 is met; the `redaction_scan` failure of the 2026-11-10 walk
at `c73c0664` is gone.

### Line 6: clang form (2026-11-07)

README step 3, clang form, from a clean `BUILD` directory at `14e93e64`:
`make -f baremetal.mk CC='clang --target=aarch64-none-elf --sysroot=<Arm newlib sysroot>' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy DRVSRC=driver/src`
(shown as run at `14e93e64`; `DRVSRC=driver/src` is the default since `9c736975`, so README no longer passes it. The sysroot is the `aarch64-none-elf` directory of the Arm GNU Toolchain; its path is not recorded).

- Result: exit 0, wall 98.80 s. Both archive members: `architecture: aarch64`.
- Versions: clang 21.1.8, llvm-ar 21.1.8, newlib 4.5.0 (`_NEWLIB_VERSION` in the sysroot's `newlib.h`).
- GCC form, at `22265e87` (origin/main, 2026-11-06), Arm GNU Toolchain 15.2.Rel1 (`aarch64-none-elf-gcc` 15.2.1): README's `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf- DRVSRC=driver/src` (as run at that commit; `DRVSRC` has been the default since `9c736975`) exit 0, 26.8 s wall, objects `elf64-littleaarch64`.
- Without `DRVSRC=driver/src`: exit 2 after 19.2 s, `abi_layout_check.c:17:10: fatal error: pvr_render.h: No such file or directory` at `baremetal.mk:184`, because `baremetal.mk:73` defaults `DRVSRC` to `$(HERE)/../src`.
- Update, 2026-11-08: the default has since landed (`baremetal.mk:73` is `DRVSRC ?= $(HERE)/driver/src`) and README's commands no longer pass `DRVSRC`; the exit 2 above is the state before that change.
- CMake aarch64: no new record; the 2026-11-05 exit 0 below stands (compiler version not recorded).

### Line 6: cross builds (2026-11-05)

Item readme-cross-walk ran README's cross builds on a host with an aarch64
toolchain, (the item's handoff gives neither the commit nor the compiler versions, so
neither is given here; this table is read at base `7ae09845`):

| README step | Command | Result |
|---|---|---|
| 1, CMake toolchain build | `cmake` with a toolchain file for aarch64, then `cmake --build` | exit 0 |
| 2, `baremetal.mk` | `make -f baremetal.mk DRVSRC=driver/src` with the Zephyr SDK prefix | verified (exit code not recorded) |
| 3, `baremetal.mk`, clang form | needs `llvm-ar`, `llvm-nm`, `llvm-objcopy` and a newlib sysroot | **not verified** (tools absent; the build stopped part way) |
| 4, Zephyr build | Pi 4 `triangle` sample in the `safeui` workspace | verified (exit code not recorded) |

- Host quirks seen: a stale `ZEPHYR_BASE` was exported and `ZEPHYR_WS` held the
  literal string `unchanged`, not a path.
- At `7ae09845` (before `9c736975`) `baremetal.mk:73` still had
  `DRVSRC ?= $(HERE)/../src`; baremetal-drvsrc had not merged.
- To close the line: run step 3 on a host with `llvm-ar`, `llvm-nm`,
  `llvm-objcopy` and a newlib sysroot, and record the exit code, the compiler
  versions and the commit here.

### Line 12: full gate (2026-11-14)

Commit `760aad3d` (main), clean tree, 4 cores, 7.9 GB RAM, 2 compiler slots,
193 GB disk shared with the cargo target directory. Inputs as
`tools/setup-gate-host.sh --check` and `tools/cts/deps-env.sh --check`
reported them, both clean before the run: every `deps-env.sh --present` entry
(`ZEPHYR_WS`, `HAL_NXP_DIR`, `ZEPHYR_SDK_INSTALL_DIR`, `PCO_SHC`,
`PCO_SPM_LOAD_CAPTURE`, `MESA_PAN_TREE`, `GLMARK2_SRC`,
`DEQP_GLES3_TEST_DIR`, `LZ4` 1.10.0 from `--build LZ4`, `PVR_FW_BLOB`,
`MALI_CSFFW`), `driver/src/fw/pvr_fw_blob.h` copied from `PVR_FW_BLOB`
(untracked), `RUSTUP_TOOLCHAIN=nightly`, `MESA_V3D_TREE` and
`CC_aarch64_unknown_none` unset (the vendored v3d reference build).

Command: `export $(tools/cts/deps-env.sh --present); tools/check.sh --jobs 2`.

Result: exit 1 after 12734 s, no summary line. Before the disk filled: 206 ok,
9 failed, 0 gap(s). The 9 failed rows are the `test` rows of
`host-test,gpu-powervr`, `...,report-es2`, `host-test,gpu-vivante`,
`host-test,gpu-{powervr,vivante},shaderc-online`, each with `report-es2` and
with `precompiled-shaders`. All fail one test, with the same output:

```
FAIL yakogl::redaction_scan redaction_scan_finds_nothing_in_this_tree
panicked at tests/redaction_scan.rs:171:9:
6 H/M finding(s) in the tree:
tools/mp2_uboot_record_check.py:488 private-ipv4
tools/mp2_uboot_record_check.py:499 private-ipv4
tools/mp2_uboot_record_check.py:504 private-ipv4
tools/mp2_uboot_record_check.py:508 private-ipv4
tools/mp2_uboot_record_check.py:490 ipv6-local
tools/mp2_uboot_record_check.py:496 tailnet-dns
```

The lines are the literal fixture values of that script's own redaction
self-test (`3e5ca63c`). `tests/redaction_scan.rs` builds such values at run time
and never holds one literally; the script does not. After the `cc
yakogl_drivers.h layout pins` row the disk was full (the shared cargo target
directory had reached 180 GB, 80 GB of it `debug/incremental`). The Zephyr
image rows from `am62p gl_test image, -Werror` on (15 rows) failed with `No
space left on device (os error 28)` or "the row's process ended without a
verdict", and the summary line's own write failed. Those rows are a host
failure, not a result. Every v3d row, `v3d71 host checks`, the two pco_shc
rows, the aarch64 rows and `test yakogl-mali-panthor` (plan #331's flaky
row) passed.

## Decisions only the owner can make

| # | Decision | Why it is the owner's | Needed by |
|---|---|---|---|
| D1 | Licence of each component: YakoGL's own code (Apache-2.0 today, `LICENSE`); the Mesa-derived parts (MIT); the Imagination DDK material (MIT with the GPLv2 alternative); the NXP `hal_nxp` part (BSD-3-Clause); the Slint parts (GPL-3.0 / Slint Royalty-free 2.0 / Slint Software 3.0, which one applies to the samples); the Zephyr PR snapshot; and whatever line 7 finds. One licence per directory, stated in `LICENSE`/`NOTICE`: library (`src/`, `capi/`), kernel driver (`driver/`; `LICENSES/GPL-2.0-only.txt` exists), `third_party/`, `tools/`, `website/`, `zephyr/` | A legal choice; the audit only reports, and line 7 can only check files against that choice | before line 7 can reach zero |
| D2 | Data files without an SPDX line: decided on 2026-10-20 (#1265): own source gets a header, generated output, captures, test data and images go into `tools/license_audit.allow` by path or kind. The 580 `undecided` files (Qt and femtovg shaders, patches, images) once open are now 0 (line 7: each has a row, sidecar or header naming its source, or left the tree); whether `undecided` becomes a `--fail-on` class is the owner's decision (issue draft, not yet filed) | Upstream-derived or unknown-origin files get no licence guessed | line 7 reached "no open finding" (2026-10-26) |
| D3 | History: publish the existing history, a rewritten one (filter out what line 8 finds), or a fresh single-commit history | Rewriting changes every hash quoted in the docs; a fresh history loses provenance | after line 8 |
| D4 | What the README conformance table claims (`README.md` lines 37-66: per-board dEQP-GLES2 results, "17,165-case mustpass list"): per-board whole-tree counts from one commit each (as line 13 asks), the newest reading per row, or the failures listed explicitly; and whether a board with a known open regression may be published as "close to complete" | It is the public claim of the project | before lines 2, 5 and 9 close |
| D5 | Security contact address, response times and supported versions in SECURITY.md | They become public | **decided** 2026-10-20 (#1266): GitHub private vulnerability reporting, no committed response time, `main` only |
| D6 | Merge or rebase rule for contributors (walk finding N4: CONTRIBUTING.md and the project settings disagree) | A project policy | **decided** 2026-10-20 (#1267): rebase onto origin/main, `--force-with-lease` on one's own topic branch |
| D7 | Promote the branch to publish to `main` and make the repository public | Irreversible: copies and caches cannot be recalled | after every line above is met |
| D8 | Publication date | Depends on stage 1 and stage 2 | last |

No board address, login, user name or credential is written in this file.
