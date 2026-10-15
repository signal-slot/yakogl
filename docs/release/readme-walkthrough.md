<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# README and CONTRIBUTING walkthrough

Base commit: `a63fcc5b48c5` (main, 2026-10-13); the commands were run there. Main has
since moved to `20a487918c74`; the 8 files that changed in between (a shader
preprocessor fix, CTS notes, a Pi 4 record) touch no path or command named here, and
this change was rebuilt on that tip. Written 2026-10-14. This file
replaces the earlier version, which recorded two walks (`ea984992`, 2026-10-11, and
`ca4288a8`, 2026-10-13). Their measured numbers are kept below, marked with the walk
they come from.

## What was done in this pass

- **Done:** every repository path, script, cargo feature and subcommand named in
  `DESIGN.md`, `SECURITY.md`, `CONTRIBUTING.md` and `docs/release/building.md` was
  checked against the tree at the base (`git ls-files`, `ls`, `Cargo.toml`,
  `.gitignore`, file modes). Every earlier finding was re-read against the text at
  the base. Stale references in `DESIGN.md`, `CONTRIBUTING.md` and
  `docs/release/building.md` were fixed.
- **Commands run (second part of this pass).** The host commands of the README were
  run as written, one after the other, in the workspace (a fresh checkout of the base
  with no build directory). Each has its exit status and wall-clock time in "Steps run".
  Host: Linux 7.2.8 x86-64 (Gentoo), rustc 1.98.0-nightly (2026-07-01). The cargo and
  build caches were warm from other sessions, so the test and gate times are warm times.
  `tools/check.sh --quick` was run to its end twice.
- **Result in one line:** README steps 4, 5 and 6 fail as written; the CMake host build
  and the five `cargo test` rows pass; `tools/check.sh --quick` fails on this host with
  its environment as it is (two rows, both caused by `MESA_V3D_TREE`) and is **clean, 119
  ok, 0 failed, 0 gaps, 236 s**, with that variable unset.
- **Not run:** the Zephyr build, the host-harness compare (about an hour and 17 to 50 GB,
  S12), the full `tools/check.sh`, `--changed` (see row 18), board steps, serial consoles,
  credentials, the PowerVR firmware.

## Paths checked (DESIGN.md, SECURITY.md, CONTRIBUTING.md)

132 distinct paths were checked (brace lists expanded), plus the cargo features, the
`yakogl-tools` subcommands and the scripts' modes. Result at the base:

| File | Checked | Result |
| --- | --- | --- |
| `DESIGN.md` | the layout block (lines 131-190): `include/` headers (GLES2, GLES3, KHR, EGL, `gles_*.h`), `Cargo.toml`, `src/lib.rs`, `src/util.rs`, `capi/` and `capi/assemble.cmake`, `src/gl/*.rs` (all 23 names), `src/backend.rs`/`.h`, `src/pvr/*`, `src/vivante/*`, `src/v3d/*`, `src/osal/*`, `src/shaderc/*` and `online/`, `tests/reference/{blobs,vivante_blobs}.rs`, the five C headers, `src/abi_layout*`, `tools/{yakogl-tools,precompile,vivante,cts,hostharness}`, `tools/cts/{RESULTS,STATUS}.md`, `tools/hostharness/{compare,run}.sh`, `driver/{src,docs,tools,rust}`, `driver/tools/pvrlab/gl_test.c`, the three command generators and `pvr_fw_boot.c` in `driver/src/`, `CMakeLists.txt`, `baremetal.mk`, `zephyr/`, `README.md`, `docs/HANDOFF.md`, `docs/es2-cts-failures.md`, `docs/online-shaderc/{abi,gc7000-abi}.md`, `driver/docs/{vivante-kernel-rust,rpi4-v3d}.md` | all exist |
| `DESIGN.md` | `src/osal/gc_kick.h`, `src/osal/v3d_kick.h` (as read in the layout block) | **stale**: the files are `src/vivante/gc_kick.h` and `src/v3d/v3d_kick.h`. Fixed: full paths now. Also `render.rs` is `src/osal/render.rs`; made explicit |
| `DESIGN.md` | commits `55de113`, `a444fe9`, `bfab616` | **not in this repository** (they are commits of the old C repository). Reworded to say so. `5ad2fa0e` exists and holds the quoted sections ("Incremental plan", "The Rust crate") |
| `DESIGN.md` | `docs/HANDOFF.md` headings "Goal, in order", "State of main", "Open work", "Known defects" | "Goal, in order", "State of main", "Open work, in order" are headings; "Known defects" is item 3 inside "Open work", not a heading. Left as is: the name is findable |
| `DESIGN.md` | `yakogl-tools` subcommands `glsl2usc`, `usc-blob`, `femtovg-shaders`, `gl-consts`, `abi-layout`, `record` | all in `tools/yakogl-tools/src/main.rs` |
| `DESIGN.md` | cargo features `gpu-powervr`, `gpu-vivante`, `gpu-v3d`, `osal-linux`, `osal-zephyr`, `osal-baremetal`, `host-test`, `shaderc-online`, `precompiled-shaders`, `mesa-reference-table` | all in `Cargo.toml` |
| `DESIGN.md` | `YAKOGL_SHADERC_PRECOMPILED_ONLY`, `YAKOGL_RUST_LINKAGE` | the first is a Zephyr option (`zephyr/CMakeLists.txt`), the second a CMake cache variable; both exist |
| `SECURITY.md` | `tools/`, `driver/tools/`, `docs/HANDOFF.md` "Known defects" | exist. The text has no other path |
| `SECURITY.md` | `<security-contact@example.invalid>` and three `TODO(owner)` lines (contact, response times, supported versions) | placeholders by design: owner decisions, not path problems. They block checklist line 4 |
| `CONTRIBUTING.md` | `README.md`, `docs/HANDOFF.md` ("How to work here" exists, line 641), `tools/check.sh` (executable), `.github/workflows/gate.yml`, `docs/handoff/README.md`, `tools/cts/RESULTS.md`, `tools/cts/STATUS.md`, `tools/board.sh` (executable), `driver/tools/mkfwblob.sh` (executable), `docs/source-form.md`, `LICENSE`, `NOTICE`, `LICENSES/`, `third_party/README.md`, `SECURITY.md` | all exist |
| `CONTRIBUTING.md` | `driver/src/fw/pvr_fw_blob.h`, `records/` | not in git by design: `driver/.gitignore` ignores `src/fw/`, `.gitignore` ignores `records/*` |
| `CONTRIBUTING.md` | `tools/check.sh --changed`, `--quick`, `--local` | all three modes exist (`tools/check.sh:12-17`) |
| `docs/release/building.md` | all repository paths; the `file:line` references | all paths exist; 10 line references had moved and were corrected (see `building.md`, "How this file was checked") |

Also executable: `tools/check.sh`, `tools/board.sh`, `tools/api_coverage.py`,
`tools/handoff-notes.sh`, `tools/hostharness/{compare,run}.sh`, `driver/tools/mkfwblob.sh`.

## Steps run (README order, then CONTRIBUTING)

Times are wall-clock seconds of the whole command. "Walk 1" and "walk 2" are the
earlier walks.

| # | Source | Command | Exit | Time | Note |
| --- | --- | --- | --- | --- | --- |
| 1 | README.md:171 | `rustc --version` | 0 | 0 s | 1.98.0-nightly |
| 2 | README.md:186 | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 0.3 s | |
| 3 | README.md:187 | `cmake --build build` | 0 | 35.9 s | cold; walk 2: 31.3 s |
| 4 | README.md:192 | `cmake -S . -B build/a64 --toolchain aarch64.cmake -DYAKOGL_OSAL=baremetal` (the `...` left out; `build/a64` instead of `build-a64` to keep the tree clean) | 1 | 0.1 s | "Could not find toolchain file: aarch64.cmake" (S1) |
| 5 | README.md:194 | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf- DRVSRC=driver/src` | 2 | 0.4 s | no `aarch64-none-elf-gcc` on this host (S3) |
| 6 | README.md:195-196 | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy DRVSRC=driver/src` | 2 | 22.9 s | `src/osal/baremetal.c:25: 'string.h' file not found` (S4) |
| 7 | README.md:197-199 | Zephyr build from `west.yml` | not run | | needs a Zephyr workspace; the gate's Zephyr rows build the images |
| 8 | README.md:288 | `tools/api_coverage.py` | 0 | 0.1 s | source fallback, no archive (S5). 304 exported, 304 resolvable (S6), 0 declared but not exported |
| 9 | README.md:310 | `cargo test --no-default-features --features host-test,gpu-powervr` | 0 | 47.6 s | 2,094 passed, 1 ignored |
| 10 | README.md:311 | `... host-test,gpu-powervr,report-es2` | 0 | 56.9 s | 1,963 passed, 2 ignored |
| 11 | README.md:312 | `... host-test,gpu-vivante` | 0 | 35.2 s | 1,966 passed |
| 12 | README.md:313 | `... host-test,gpu-powervr,shaderc-online` | 0 | 251.5 s | 3,106 passed, 3 ignored (walk 1: 337 s) |
| 13 | README.md:314 | `... host-test,gpu-vivante,shaderc-online` | 0 | 245.0 s | 2,976 passed, 3 ignored (walk 1: 340 s) |
| 14 | README.md:315 | `cargo test -p yakogl-tools` | 0 | 5.7 s | 113 passed |
| 15 | README.md:307, CONTRIBUTING.md | `tools/check.sh --quick` | **1** | 565.3 s | 114 ok, 2 failed, 0 gaps (116 rows). Failed: `run v3d tex_check, cl_golden, job_check` (this host's `MESA_V3D_TREE` has no `build-v3d`) and the `check.sh --changed` self-test (3 of 75 checks: it counts 146 rows where it expects 149, because the v3d rows are missing for the same reason). Host environment, see N9 |
| 16 | the same | `env -u MESA_V3D_TREE tools/check.sh --quick` | 0 | 236.3 s | **119 ok, 0 failed, 0 gaps: clean**; 95 test binaries run; warm tree |
| 17 | CONTRIBUTING.md | `tools/check_scope_selftest.sh` alone | 1 | 4.3 s | the same 3 failed checks as row 15 |
| 18 | README.md:306 | `tools/check.sh --changed <base>` | stopped | 20 min | an untracked helper file made the scope "all" (6 paths, every row), so it was stopped. Not repeated, see N10 |
| 19 | README.md:357-358 | `git grep -n 'ignore = "BUG' -- src tests` | 0 | 0.1 s | 2 matches |
| 20 | docs/handoff/README.md | `tools/handoff-notes.sh --check` | 0 at the base | | "nothing waits". With the note of this pass it exits 1 until the fold removes it (by design) |
| 21 | README.md:363 | `HH_TIMEOUT=60 tools/hostharness/compare.sh ...` | not run | | walk 1: exit 1, 3,436 s, 17 GB; walk 2: exit 1, 2,529 s, 50 GB |
| 22 | CONTRIBUTING.md:68-80 | `tools/board.sh` | not run | | needs a board |
| 23 | CONTRIBUTING.md | `driver/tools/mkfwblob.sh` with `rgx.fw` | not run | | needs TI's firmware |
| 24 | CONTRIBUTING.md | `tools/check.sh` (full gate) | not run | | walk 1: exit 1, 2,937 s (49 min); walk 2: exit 1, 620 s (S8, now fixed in the source) |

The full-gate time is not re-measured; CONTRIBUTING.md keeps the two walk numbers.

## Earlier findings (P1-P10, S1-S13): fixed or open at `a63fcc5b`

Status is from reading the text at the base, not from a run, unless a count is given.

| Finding | What | Status | Where |
| --- | --- | --- | --- |
| P1 | README placeholder `/path/to/powervr-bxs-am62p/src` | **fixed** (no such text; README.md:211 says `driver/src`) | README |
| P2, S1 | `aarch64.cmake` is not shipped | **open** | README.md:188, 192 |
| S2 | `build-a64/` is not ignored; after step 4 `--changed` runs every row | **open** (`.gitignore` has `build/` only) | README.md:192 |
| P3, S3 | `baremetal.mk` needs an `aarch64-none-elf` GCC; not said | **open** | README.md:193-194 |
| S4 | `DRVSRC=...` placeholder; clang needs bare-metal C headers | **open** | README.md:195-196 |
| P4 | stale test counts (1492 + 6, 74) | **fixed** (no counts in the text any more) | README |
| S5 | `api_coverage.py` falls back to the source without the archive | **open** | README.md:288 |
| S6 | 298 names vs 304 measured | **open**, 304 re-measured (step 8) | README.md:291, 293 |
| P5, S10 | "about 45 minutes" | **fixed** here: "35 to 50 minutes, depending on the host" with the two measurements | CONTRIBUTING.md:30 |
| P6, S7 | AM62P Zephyr rows: `swi_tables.ld` not found | CONTRIBUTING **fixed** (needs a real `ZEPHYR_WS` path, no symbolic link). The cause in `tools/check.sh:2519` is **open** (outside this item) | CONTRIBUTING.md, tools/check.sh |
| P7, S9 | `--quick` described as "the PowerVR build only" | CONTRIBUTING **fixed** (about 120 rows, 14 and 4 minutes). README **open**; `tools/check.sh:17` has the same stale wording, **open** | README.md:307, tools/check.sh:17 |
| S8 | `tests/es3_tex_levels.rs` does not set `query_begin`/`query_end`/`query_result` | **fixed in the source** (`query_begin: None` now at line 128), confirmed: the six test rows and `--quick` build (steps 9-16) | tests |
| P8, S11 | the count of open `ignore = "BUG` tests | **open**, and the count changed again: **two** today (`src/pvr/backend_tests.rs:4601` and `tests/es2_report_guard.rs:672`) | README.md:357-359 |
| P9, S12 | cost of the host-harness run | **open** | README.md:363-373 |
| P10, S13 | a clean comparison is not quiet when jobs time out | **open** | README.md:367-368 |

## New findings

- **N1 (README, open).** Same set as above; proposed texts follow.
- **N2 (tools, outside this item).** `tools/check.sh:17` says `--quick` is "the PowerVR
  build only". It runs about 120 rows. Change to `the short gate (about 120 rows)`.
- **N3 (tools, outside this item).** `tools/check.sh:2519`: `ZEPHYR_BASE="$ZEPHYR_WS/zephyr"`
  with `cd "$ZEPHYR_WS"`. Walk 2's lead: use `$(cd "$ZEPHYR_WS" && pwd -P)` so a linked
  workspace works. Not confirmed by a build. `tools/check.sh:2719` also defaults
  `HAL_NXP_DIR` to a path under `$HOME/com/github/`, which fits one host only; a
  contributor sees a gap that names `HAL_NXP_DIR` (fine), but the default is a local layout.
- **N4 (owner).** `CONTRIBUTING.md` says "a pushed branch is never rebased; bring main in
  with a merge". The `conventions` text of `.ccfarm/project.toml` says branches are "rebased
  onto origin/main". One of them is wrong. CONTRIBUTING.md was not changed for this.
- **N5 (owner).** `SECURITY.md` has the contact placeholder and three `TODO(owner)` lines
  (contact, response times, supported versions). Checklist line 4 stays partial until
  they are filled.
- **N6 (done).** `docs/release/building.md` line references had drifted by up to 1,200
  lines (`tools/check.sh` grew). Ten were corrected; the file now says line numbers drift.
- **N7 (other item).** `DESIGN.md` lines 17-25 state per-board conformance numbers (AM62P
  54 failures on `32778b57`, and so on). They belong to es2-docs-final and move with the final
  trees; not touched here.
- **N9 (tools, measured).** `tools/check.sh --quick` exits 1 on a host whose
  `MESA_V3D_TREE` names a Mesa tree without `build-v3d`: the v3d row fails ("is not a
  Mesa tree"), and `tools/check_scope_selftest.sh` fails 3 of 75 checks because it
  counts 146 rows, not 149. Proposed for a `tools/` item: treat such a tree as a gap
  or ignore it in the v3d row, and make the self-test independent of the variable.
  Until then a reader on such a host runs `env -u MESA_V3D_TREE tools/check.sh --quick`.
- **N10 (CONTRIBUTING.md, fixed: step 1 of "How a change reaches main" now says an
  untracked file counts and widens the scope).** An untracked file outside the ignored paths makes
  `tools/check.sh --changed` select every row (here 6 paths, scope "all"; stopped after
  20 minutes). Same cause as S2. Proposed line for step 1 of "How a change reaches
  main": "Keep scratch files in `build/` (ignored); an untracked file elsewhere runs
  every row." Not added: the run was stopped, not completed.
- **N8 (other item).** `CONTRIBUTING.md:16` says a docs-only change runs two rows, and
  `--local` "13 rows". Neither count was re-measured; both are left as written.

## README findings, all fixed (see the list at the end of this section)

Line numbers are of README.md at the base. Each proposed text replaces the quoted one.

1. **Line 188** (S1). `# cross build: a CMake toolchain file; the Rust target is derived from`
   becomes `# cross build: a CMake toolchain file of your own (none is shipped); the Rust target is derived from`.
2. **Line 192** (S1, S2). `-B build-a64` becomes `-B build/a64` (inside the ignored `build/`).
   The `aarch64.cmake` argument stays, as "of your own" now says it is yours.
3. **Line 193** (P3, S3). `# bare metal (RUST_TARGET defaults to aarch64-unknown-none)` becomes
   `# bare metal (RUST_TARGET defaults to aarch64-unknown-none); needs an aarch64-none-elf GCC on PATH, or use the clang form below`.
4. **Lines 195-196** (S4). `DRVSRC=...` becomes `DRVSRC=driver/src`; add after line 196:
   `# clang needs bare-metal C headers (a newlib sysroot, given with --sysroot in CC)`.
5. **Line 288** (S5). After `tools/api_coverage.py` add `(after cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table; without that archive it parses the source instead)`.
6. **Lines 291 and 293** (S6). `298 names` becomes `304 names` (walk 2), or drop both counts: `the names it exports` and `the names it resolves`. Counts drift; dropping is better.
7. **Line 307** (P7, S9). `#    the PowerVR build only, for a tight edit loop` becomes
   `#    the short gate (about 120 rows; 4 to 14 minutes), for a tight edit loop`.
8. **Lines 357-358** (P8, S11). `Five are open today` becomes `Two are open today`, and
   `four ES 2.0 report pins in `tests/es2_report_guard.rs` and the PowerVR's split 4x frame in
   `src/pvr/backend_tests.rs`` becomes `one ES 2.0 report pin in `tests/es2_report_guard.rs` and
   the PowerVR's split 4x frame in `src/pvr/backend_tests.rs``. Better: drop the count and
   keep the `git grep` command.
9. **After line 373** (P9, S12). Add: `With HH_TIMEOUT=60 a run takes 40 to 60 minutes and 17 to 50 GB of disk. With the default timeout (1200 s), gl_test case 174 fills a disk. Pass your own checkout of the driver as the last argument; in this repository it is driver/.`
10. **Line 367** (P10, S13). After `does not explain` add: `(a job that times out on both sides is reported too; read the report before you call a difference a regression)`.

All ten are **fixed** in README.md (plan 149, item pub-readme-fixes):

1. fixed: the comment says the toolchain file is the reader's own.
2. fixed: `-B build/a64`.
3. fixed: the comment names the aarch64-none-elf GCC need and the clang form.
4. fixed: `DRVSRC=driver/src` and the sysroot comment are in place.
5. fixed: the cargo build line follows `tools/api_coverage.py`.
6. fixed: both counts dropped (the tool prints 304 today).
7. fixed: the `--quick` comment says the short gate, 4 to 14 minutes.
8. fixed: the count is dropped; the two open pins are named (the PowerVR split 4x
   frame and the Vivante texture-size pin) and the `git grep` stays. The ES 2.0
   report pins are no longer ignored.
9. fixed: the HH_TIMEOUT, disk and driver-checkout sentence follows the compare.sh paragraph.
10. fixed: the timed-out-on-both-sides remark follows `does not explain`.

## Checklist: what can change (checklist.md is held by pub-checklist)

| Line | Now | Can change to | Condition |
| --- | --- | --- | --- |
| 3 CONTRIBUTING.md | met (text) | **partial** (not met) | the README commands were run (steps 1-14): steps 4-6 fail as written (S1, S3, S4). `tools/check.sh --quick` is clean only with `MESA_V3D_TREE` unset (steps 15-16, N9). The gate times in CONTRIBUTING.md come from walks 1 and 2 |
| 5 DESIGN.md | met (text) | **met** | all named paths exist after this change (3 stale references fixed); proof: the table "Paths checked" |
| 6 walkthrough | partial | stays **partial** | README findings 1-10 are fixed. The quick gate reaches clean (step 16) only after unsetting `MESA_V3D_TREE`; a host that sets it ends red (step 15) |
| 4 SECURITY.md | partial | stays partial | no stale path; the contact is the owner's |
