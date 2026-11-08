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

Since 2026-10-19 the hand-over file, its notes directory and its notes script
are retired (open work is in the GitHub issues); this record names them as "the
hand-over file", "the hand-over notes" and "the hand-over notes script".

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
| `DESIGN.md` | the layout block (lines 131-190): `include/` headers (GLES2, GLES3, KHR, EGL, `gles_*.h`), `Cargo.toml`, `src/lib.rs`, `src/util.rs`, `capi/` and `capi/assemble.cmake`, `src/gl/*.rs` (all 23 names), `src/backend.rs`/`.h`, `src/pvr/*`, `src/vivante/*`, `src/v3d/*`, `src/osal/*`, `src/shaderc/*` and `online/`, `tests/reference/{blobs,vivante_blobs}.rs`, the five C headers, `src/abi_layout*`, `tools/{yakogl-tools,precompile,vivante,cts,hostharness}`, `tools/cts/{RESULTS,STATUS}.md`, `tools/hostharness/{compare,run}.sh`, `driver/{src,docs,tools,rust}`, `driver/tools/pvrlab/gl_test.c`, the three command generators and `pvr_fw_boot.c` in `driver/src/`, `CMakeLists.txt`, `baremetal.mk`, `zephyr/`, `README.md`, the hand-over file, `docs/es2-cts-failures.md`, `docs/online-shaderc/{abi,gc7000-abi}.md`, `driver/docs/{vivante-kernel-rust,rpi4-v3d}.md` | all exist |
| `DESIGN.md` | `src/osal/gc_kick.h`, `src/osal/v3d_kick.h` (as read in the layout block) | **stale**: the files are `src/vivante/gc_kick.h` and `src/v3d/v3d_kick.h`. Fixed: full paths now. Also `render.rs` is `src/osal/render.rs`; made explicit |
| `DESIGN.md` | commits `55de113`, `a444fe9`, `bfab616` | **not in this repository** (they are commits of the old C repository). Reworded to say so. `5ad2fa0e` exists and holds the quoted sections ("Incremental plan", "The Rust crate") |
| `DESIGN.md` | the hand-over file headings "Goal, in order", "State of main", "Open work", "Known defects" | "Goal, in order", "State of main", "Open work, in order" are headings; "Known defects" is item 3 inside "Open work", not a heading. Left as is: the name is findable |
| `DESIGN.md` | `yakogl-tools` subcommands `glsl2usc`, `usc-blob`, `femtovg-shaders`, `gl-consts`, `abi-layout`, `record` | all in `tools/yakogl-tools/src/main.rs` |
| `DESIGN.md` | cargo features `gpu-powervr`, `gpu-vivante`, `gpu-v3d`, `osal-linux`, `osal-zephyr`, `osal-baremetal`, `host-test`, `shaderc-online`, `precompiled-shaders`, `mesa-reference-table` | all in `Cargo.toml` |
| `DESIGN.md` | `YAKOGL_SHADERC_PRECOMPILED_ONLY`, `YAKOGL_RUST_LINKAGE` | the first is a Zephyr option (`zephyr/CMakeLists.txt`), the second a CMake cache variable; both exist |
| `SECURITY.md` | `tools/`, `driver/tools/`, the hand-over file "Known defects" | exist. The text has no other path |
| `SECURITY.md` | `<security-contact@example.invalid>` and three `TODO(owner)` lines (contact, response times, supported versions) | placeholders by design: owner decisions, not path problems. They block checklist line 4 |
| `CONTRIBUTING.md` | `README.md`, the hand-over file ("How to work here" exists, line 641), `tools/check.sh` (executable), `.github/workflows/gate.yml`, the hand-over notes' README, `tools/cts/RESULTS.md`, `tools/cts/STATUS.md`, `tools/board.sh` (executable), `driver/tools/mkfwblob.sh` (executable), `docs/source-form.md`, `LICENSE`, `NOTICE`, `LICENSES/`, `third_party/README.md`, `SECURITY.md` | all exist |
| `CONTRIBUTING.md` | `driver/src/fw/pvr_fw_blob.h`, `records/` | not in git by design: `driver/.gitignore` ignores `src/fw/`, `.gitignore` ignores `records/*` |
| `CONTRIBUTING.md` | `tools/check.sh --changed`, `--quick`, `--local` | all three modes exist (`tools/check.sh:12-17`) |
| `docs/release/building.md` | all repository paths; the `file:line` references | all paths exist; 10 line references had moved and were corrected (see `building.md`, "How this file was checked") |

Also executable: `tools/check.sh`, `tools/board.sh`, `tools/api_coverage.py`,
the hand-over notes script, `tools/hostharness/{compare,run}.sh`, `driver/tools/mkfwblob.sh`.

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
| 20 | the hand-over notes' README | the hand-over notes script, `--none-waiting` | 0 at the base | | "nothing waits". With the note of this pass it exits 1 until the fold removes it (by design) |
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
| P7, S9 | `--quick` described as "the PowerVR build only" | CONTRIBUTING **fixed** (169 of 209 rows counted at `bfd75c57`, minutes warm and up to an hour cold; `1e54f2d1`). README **open**; `tools/check.sh:17` has the same stale wording, **open** | README.md:307, tools/check.sh:17 |
| S8 | `tests/es3_tex_levels.rs` does not set `query_begin`/`query_end`/`query_result` | **fixed in the source** (`query_begin: None` now at line 128), confirmed: the six test rows and `--quick` build (steps 9-16) | tests |
| P8, S11 | the count of open `ignore = "BUG` tests | **open**, and the count changed again: **two** today (`src/pvr/backend_tests.rs:4601` and `tests/es2_report_guard.rs:672`) | README.md:357-359 |
| P9, S12 | cost of the host-harness run | **open** | README.md:363-373 |
| P10, S13 | a clean comparison is not quiet when jobs time out | **open** | README.md:367-368 |

## New findings

- **N1 (README, open).** Same set as above; proposed texts follow.
- **N2 (tools, outside this item).** `tools/check.sh:17` says `--quick` is "the PowerVR
  build only". It runs 169 of 209 rows (counted at `bfd75c57`). Change to `the short gate (169 of 209 rows)`.
- **N3 (tools, outside this item).** `tools/check.sh:2519`: `ZEPHYR_BASE="$ZEPHYR_WS/zephyr"`
  with `cd "$ZEPHYR_WS"`. Walk 2's lead: use `$(cd "$ZEPHYR_WS" && pwd -P)` so a linked
  workspace works. Not confirmed by a build. `tools/check.sh:2719` also defaults
  `HAL_NXP_DIR` to a path under `$HOME/com/github/`, which fits one host only; a
  contributor sees a gap that names `HAL_NXP_DIR` (fine), but the default is a local layout.
- **N4 (owner).** `CONTRIBUTING.md` says "a pushed branch is never rebased; bring main in
  with a merge". The `conventions` text of ccfarm's settings for the project says branches are "rebased
  onto origin/main". One of them is wrong. CONTRIBUTING.md was not changed for this.
- **N5 (owner).** `SECURITY.md` has the contact placeholder and three `TODO(owner)` lines
  (contact, response times, supported versions). Checklist line 4 stays partial until
  they are filled.
- **N6 (done).** `docs/release/building.md` line references had drifted by up to 1,200
  lines (`tools/check.sh` grew). Ten were corrected; the file now says line numbers drift.
- **N7 (other item).** `DESIGN.md` lines 17-25 state per-board conformance numbers (AM62P
  54 failures on `32778b57`, and so on). They belong to es2-docs-final and move with the final
  trees; not touched here.
- **N9 (tools, fixed by `tools/check.sh: report a MESA_V3D_TREE without build-v3d as a gap, not a failure`; the finding as measured follows).** `tools/check.sh --quick` exited 1 on a host whose
  `MESA_V3D_TREE` names a Mesa tree without `build-v3d`: the v3d row fails ("is not a
  Mesa tree"), and `tools/check_scope_selftest.sh` fails 3 of 75 checks because it
  counts 146 rows, not 149. Proposed for a `tools/` item: treat such a tree as a gap
  or ignore it in the v3d row, and make the self-test independent of the variable.
  Now the three v3d rows are one gap naming `MESA_V3D_TREE` (not a pass); the self-test
  unsets the variable for itself and empties it for its plans, so it passes either way.
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
   `#    the short gate (169 of 209 rows, counted at bfd75c57; minutes warm, up to an hour cold), for a tight edit loop`.
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

## Line 6: cross builds (README steps 1-4)

This section is the measured record of the README cross-build block (README.md,
"host library" to "Zephyr") for checklist line 6. For line 6 it supersedes rows 4-7
of "Steps run" above (`aarch64.cmake` not found, no GCC on the host, the clang
header error, Zephyr not run), which were measured at `a63fcc5b48c5` with the
README as it was then; those rows stay as the earlier record.

Every command was run as `sh -c '...; echo EXIT=$?'` with the output in a file (never
through a pipe, which would hide the exit code). Builds went into `build/` of the
workspace, one `BUILD=` directory per row, each from an empty directory. Rows 1-3c and
V1-V3 were measured at **`f652d6dc`**, the integration tip of 2026-11-07, which holds the
`DRVSRC` fix (`9c736975`, `baremetal.mk:73` reads `DRVSRC  ?= $(HERE)/driver/src`). Main
(`80c486d2`) differs from it only in README.md, checklist.md and two docs notes, none in
a build input. Host: Linux 7.0.0 x86-64, rustc 1.101.0-nightly (`5d8fec066`, 2026-10-16)
with the `aarch64-unknown-none` target, cmake 4.2.3, ninja 1.13.2, GNU Make 4.4.1.

Acceptance checks on `f652d6dc`, under `sh` with GNU grep 3.12: c1 (`baremetal.mk` has
the line `DRVSRC ?= $(HERE)/driver/src`) exit **0**; c2 (no `../src` left in
`baremetal.mk`, `grep -q` finds none) exit **1** (nothing found, as required).

Toolchain of the gcc-prefix rows: the **Zephyr SDK 1.0.1** (`$ZEPHYR_SDK_INSTALL_DIR`):
`aarch64-zephyr-elf-gcc (Zephyr SDK 1.0.1) 14.3.0`, GNU ar and objcopy 2.43.1. The prefix
with SDK 1.0 is
`$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-` (the pre-1.0
path `$ZEPHYR_SDK_INSTALL_DIR/aarch64-zephyr-elf/bin/` fails). The Arm GNU Toolchain
`aarch64-none-elf-` prefix is row 2a (the same toolchain download as the clang sysroot, below).

Toolchain of the clang rows: Ubuntu clang 21.1.8 (`6ubuntu1`) and Ubuntu LLVM 21.1.8
`llvm-ar`, `llvm-nm`, `llvm-objcopy`, all with `--target=aarch64-none-elf`, run as the
README's **unversioned** names: `/usr/lib/llvm-21/bin` (which has `clang`, `llvm-ar`,
`llvm-nm`, `llvm-objcopy` unversioned) was put first on `PATH`. Sysroot: the
`aarch64-none-elf` directory of the Arm GNU Toolchain 14.3.rel1
(`arm-gnu-toolchain-14.3.rel1-x86_64-aarch64-none-elf`, newlib; tarball downloaded from
developer.arm.com into `$CONNECTED_DEPS_DIR/arm-gnu-dl`, sha256
`ebaf2d47f2e7f7b645864c5c8cf839e526daed83a2e675a3525d03f5ba3d2be9`, equal to the
published value). `<sysroot>` in the table is that directory.

| Row | Command | Exit | Commit | Compiler and tools | Sysroot / SDK | Output |
| --- | --- | --- | --- | --- | --- | --- |
| 1. CMake toolchain build | `cmake -S . -B build/a64 --toolchain <toolchain.cmake> -DYAKOGL_OSAL=baremetal`, then `cmake --build build/a64`. The toolchain file is the reader's own: `CMAKE_SYSTEM_NAME Generic`, `CMAKE_SYSTEM_PROCESSOR aarch64`, `CMAKE_C_COMPILER`/`CMAKE_AR`/`CMAKE_NM`/`CMAKE_OBJCOPY` set to the SDK prefix below, `CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY` | configure 0, build 0 | `f652d6dc`. Earlier: `6ad0f777`: 0, 41,945,730 bytes | aarch64-zephyr-elf-gcc 14.3.0, cmake 4.2.3, ninja 1.13.2, rustc 1.101.0-nightly; Rust target derived: `aarch64-unknown-none` | Zephyr SDK 1.0.1 | `build/a64/libyakogl.a` 42,130,018 bytes (default `online` shader back end) |
| 2. `baremetal.mk`, gcc prefix, with `DRVSRC=driver/src` | `make -f baremetal.mk CROSS_COMPILE=$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf- DRVSRC=driver/src` | 0 | `f652d6dc`. Earlier: `6ad0f777`: 0, 27,493,510 bytes | aarch64-zephyr-elf-gcc 14.3.0, rustc 1.101.0-nightly, cmake 4.2.3 | Zephyr SDK 1.0.1 (freestanding; no libc needed to build) | `libyakogl.a` 27,643,406 bytes ("133 runtime symbols made local" in the earlier runs) |
| 2'. the same without `DRVSRC` | `make -f baremetal.mk CROSS_COMPILE=$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-` | **0** | `f652d6dc` (the `DRVSRC` default is committed there, `9c736975`). Earlier: `964f50f0` without the fix: 2, `pvr_render.h: No such file or directory` at `src/abi_layout_check.c:17` | aarch64-zephyr-elf-gcc 14.3.0 | Zephyr SDK 1.0.1 | `libyakogl.a` 27,643,790 bytes (384 bytes more than row 2: the path strings differ; the member list was identical with and without `DRVSRC=driver/src` at the earlier commits) |
| 2a. rows 2 and 2' with the Arm GNU prefix | row 2 and row 2' with `CROSS_COMPILE=<Arm GNU Toolchain 14.3.rel1>/bin/aarch64-none-elf-` | 0 with `DRVSRC`, 0 without | `f652d6dc`. Earlier: `98251549`: 0, 0 | aarch64-none-elf-gcc (Arm GNU Toolchain 14.3.Rel1, Build arm-14.174) 14.3.1 20250623 | the toolchain's newlib | `libyakogl.a` 27,661,518 bytes with `DRVSRC`, 27,661,966 without |
| 3. clang form, README literal (no `--sysroot`) | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy` | **2** | `f652d6dc`. Earlier: `964f50f0`, `14e93e64`: 2 | clang 21.1.8, llvm-ar, llvm-nm, llvm-objcopy 21.1.8, unversioned names via `PATH` | none | `'string.h' file not found` at `src/osal/baremetal.c:25` (`baremetal.mk:184`): a host requirement, a newlib sysroot must be in `CC`; no tree defect |
| 3a. clang form with a sysroot, GPU=powervr | `make -f baremetal.mk CC='clang --target=aarch64-none-elf --sysroot=<sysroot>' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy DRVSRC=driver/src` | 0 | `f652d6dc`. Earlier: `964f50f0`: 0, 26,936,654 bytes | as row 3 | Arm GNU Toolchain 14.3.rel1 newlib, `<sysroot>` as above | `libyakogl.a` 26,943,854 bytes |
| 3b. the same, `GPU=vivante` | row 3a plus `GPU=vivante` | 0 | `f652d6dc`. Earlier: `964f50f0`: 0, 22,781,802 bytes | as row 3 | as 3a | `libyakogl.a` 22,822,930 bytes |
| 3c. the same, powervr, no `DRVSRC` | row 3a without `DRVSRC=driver/src` | **0** | `f652d6dc`. Earlier: `964f50f0` without the fix: 2, `pvr_render.h` not found at `src/abi_layout_check.c:17` | as row 3 | as 3a | `libyakogl.a` 26,944,542 bytes |
| 4a. Zephyr, `ZEPHYR_EXTRA_MODULES` | `west build -b rpi_4b examples/triangle` with yakogl and `driver/` in `ZEPHYR_EXTRA_MODULES`, `CONFIG_YAKOGL=y` | 0 | `b60224da` (zephyr-step4; **not** re-run at `f652d6dc`, see below) | rustc 1.101.0-nightly, cmake 4.2.3, ninja 1.13.2, west 1.5.0 | Zephyr 4.4.99 (`af17c0c8aa2a`), Zephyr SDK 1.0.1 | `zephyr.elf` 20,159,608 bytes; `libyakogl.a` 1,942,720 bytes |
| 4b. Zephyr, form (a): a fresh west workspace from yakogl's `west.yml` | a fresh west workspace from `west.yml`, `west update` (3 m 21.8 s), then the sample build | 0 | `d055a0b` (not re-run) | as 4a | Zephyr `af17c0c8` (equal to the `west.yml` pin), Zephyr SDK 1.0.1 | `zephyr.elf` 20,174,808 bytes; `libyakogl.a` 1,944,368 bytes |
| V1. `GPU=vivante` | row 2 plus `GPU=vivante` | 0 | `f652d6dc`. Earlier: `6ad0f777`: 0, 22,951,578 bytes | as row 2 | Zephyr SDK 1.0.1 | `libyakogl.a` 22,992,970 bytes |
| V2. `REPORT_ES2=0` | `make -f baremetal.mk CROSS_COMPILE=<SDK prefix> REPORT_ES2=0` (no `DRVSRC`) | 0 | `f652d6dc`. Earlier: `606343ff`: 0, 28,291,846 bytes | as row 2 | Zephyr SDK 1.0.1 | `libyakogl.a` 28,442,126 bytes |
| V3. `RUST_LINKAGE=external` | row 2' plus `RUST_LINKAGE=external` | 0 | `f652d6dc`. Earlier: `6ad0f777`, `964f50f0`: 0, 659,340 bytes | as row 2 | Zephyr SDK 1.0.1 | `libyakogl_c.a` 659,724 bytes (C parts only) |

Sources of the rows: rows 1, 2, 2', 2a, 3, 3a-3c and V1-V3 were all re-run for this record at
`f652d6dc`, each into an empty `BUILD=` directory, so the exit codes are those of the
first, complete `make`/`cmake` run. The "Earlier" numbers are the previous measurements
at `6ad0f777`, `964f50f0`, `606343ff` and `14e93e64`; sizes differ from the current ones
because the sources moved. Rows 4a and 4b come from earlier items (
zephyr-step4/zephyr-4a-rerun, zephyr-west-init/west-venv-e2e); their commands and
outputs are not stored in the repository, so those cells are as the items reported them.
Of steps 2 and 4 only step 2 with the SDK prefix was re-run at the tip (rows 2, 2', 2a): the
re-run of step 4 needs `west`, a venv with `requirements-base.txt` and a Zephyr
workspace, none of which this host had, so row 4 keeps its older commits (the Zephyr
build does not read `baremetal.mk`; the `DRVSRC` change cannot reach it).

Host quirks seen:

- With Zephyr SDK 1.0 the step-2 prefix is
  `$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-`; the pre-1.0
  path fails.
- The clang form needs a newlib sysroot in `CC` (`--sysroot=<Arm GNU
  Toolchain>/aarch64-none-elf`); without it step 3 stops at `string.h` (row 3).
- Ubuntu installs the unversioned `llvm-ar`, `llvm-nm`, `llvm-objcopy` only under
  `/usr/lib/llvm-21/bin`, not in `/usr/bin` (there only the `-21` names); that directory
  has to be on `PATH` for README's literal names, or the `-21` names passed.
- A stale `ZEPHYR_BASE` in the environment breaks step 4 form (a); unset it first. The
  sample also needs `RUSTUP_TOOLCHAIN=nightly`.
- On hosts that refuse `$?` in a command line, put `echo EXIT=$?` inside `sh -c '...'`.
  Some hosts also refuse a direct call of the toolchain binaries; a call through `make`
  or `sh -c` works.
- README's step 1 needs a toolchain file of the reader's own; the one used here is eight
  lines (listed in row 1).

## Line 6: texts to paste (#1606, checklist State, README)

Three texts for places this record cannot edit (checklist.md and README.md are held by
other plans). All hashes and versions are from the section "Line 6: cross builds" above.

### #1606 (comment for GitHub issue #1606)

```
Checklist line 6 (README walkthrough): both open points are closed.

1. DRVSRC default: baremetal.mk:73 now reads `DRVSRC ?= $(HERE)/driver/src`
   (commit 9c736975, the cherry-pick of f82cfcce). `make -f baremetal.mk` no
   longer needs `DRVSRC=driver/src`: step 2 exits 0 with and without it, and the
   clang form too.
2. README step 3, clang form: exit 0 with a newlib sysroot (Arm GNU Toolchain
   14.3.rel1, aarch64-none-elf directory, via --sysroot in CC) for GPU=powervr
   and GPU=vivante, with the unversioned llvm-ar, llvm-nm, llvm-objcopy of
   Ubuntu LLVM 21.1.8 (clang 21.1.8). The README literal without a sysroot exits 2
   ('string.h' file not found at src/osal/baremetal.c:25): a host requirement,
   not a tree defect.

Also: step 1 (CMake toolchain build) exit 0, step 2 with the Zephyr SDK 1.0.1
prefix (gcc 14.3.0) exit 0, step 4 (Zephyr) exit 0 (earlier commits b60224da,
d055a0b; the Zephyr build does not read baremetal.mk).

Evidence: docs/release/readme-walkthrough.md, section "Line 6: cross builds
(README steps 1-4)". Rows 1-3c and V1-V3 were measured at f652d6dc.
```

### checklist State (replacement for the State cell of row 6)

```
**done** (2026-11-08; the cross builds at `f652d6dc`, line 3 at `14e93e64`, GCC form at `22265e87`, base `40eb8d00`). The criterion is that a new reader following README then CONTRIBUTING reaches a passing `tools/check.sh --quick` with no gap: a fresh clone does (line 3, 170 ok, 0 failed, 0 gap(s) at `14e93e64`), and every cross build README lists builds as README writes it (`docs/release/readme-walkthrough.md`, "Line 6: cross builds": CMake step 1 exit 0; `baremetal.mk` step 2 exit 0 with and without `DRVSRC`, since `baremetal.mk:73` defaults `DRVSRC` to `$(HERE)/driver/src`, commit `9c736975`; clang form of step 3 exit 0 with a newlib sysroot (Arm GNU Toolchain 14.3.rel1, clang 21.1.8) and exit 2 without one, a host requirement; Zephyr step 4 exit 0). `CONTRIBUTING.md` gives the `--quick` size and the `MESA_V3D_TREE` gap.
```

### README (cross-build wording)

`grep -n DRVSRC README.md` finds only line 244, the option table row
`YAKOGL_PVR_DRIVER_DIR` (`DRVSRC`) with default `driver/src`. `git log --oneline -S
'DRVSRC=driver/src' -- README.md` lists `d254d172` ("docs: README bare-metal commands use
baremetal.mk's in-tree DRVSRC default"), which removed the `DRVSRC=driver/src` argument.
The current lines are:

```
make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-
...
make -f baremetal.mk \
     CC='clang --target=aarch64-none-elf --sysroot=<arm-gnu>/aarch64-none-elf' \
     AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy
```

No change is needed for the `DRVSRC` wording. Optional sentence for step 3, replacing the
comment above the clang form (the README already says sysroot and versioned names; this
adds that the unversioned names must be on `PATH`):

```
# clang needs bare-metal C headers: a newlib sysroot for aarch64-none-elf in CC (for
# example --sysroot=<arm-gnu-toolchain>/aarch64-none-elf) and unversioned llvm-ar,
# llvm-nm, llvm-objcopy on PATH (distributions ship them versioned, llvm-ar-21, ...:
# pass those names as AR, NM and OBJCOPY)
```
