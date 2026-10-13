<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (merge `77c860056`, ccfarm `01a12a9b-34ae-72d8-b1f7-d34954f8dccf`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# README and CONTRIBUTING walkthrough on a fresh clone

> Status added when landed on main (2026-10-12): the findings are of commit `ea984992` (2026-10-11). Read against main's README.md and CONTRIBUTING.md before acting: P1 (the `/path/to/powervr-bxs-am62p/src` placeholder) is gone from README.md; P2 (`aarch64.cmake`, README.md line 191) and P5 ("about 45 minutes", CONTRIBUTING.md line 31) are still in the text. The other findings were not re-checked.

> Added 2026-10-13: a second walk, of commit `ca4288a8` (2026-10-13), is in "Second walk" at the end of this file. Its line references are of current main. Where the two walks differ, use the second one.

A new contributor follows the host-side steps of `README.md` and
`CONTRIBUTING.md`, in order, without installing any system package.

- Date: 2026-10-11.
- Clone: `git clone` of the repository's `origin` (it could be fetched), into a new
  directory under the system temp directory. `CARGO_TARGET_DIR` was `.target` inside
  the clone. The clone was at commit `ea984992`. The README and CONTRIBUTING lines
  quoted below are the same in that commit and in the integration tip `e3efeaa`, except
  `README.md` lines 40-42 (status table) and `CONTRIBUTING.md` lines 6-7, which origin
  already fixed.
- Host: Linux 7.2.8 (x86-64, Gentoo), rustc 1.98.0-nightly (2026-07-01),
  cargo 1.98.0-nightly, cmake 4.3.5, ninja 1.13.2. The Rust target
  `aarch64-unknown-none-softfloat` was already installed. The host already had the
  Zephyr, NXP HAL, `pco_shc` and firmware-header setup that `CONTRIBUTING.md` lists, so
  the full gate showed 0 gaps. A contributor without it will get named gaps.
- Times are wall-clock seconds. The `cargo test` rows ran on a target directory that a
  first `cargo test` run had already warmed, so the cold time is longer (the
  `shaderc-online` rows were cold: about 6 minutes each).
- Not done, by rule: board steps, serial consoles, credentials, the PowerVR firmware.

## Steps

| # | Source | Command | Exit | Time | Problem |
| --- | --- | --- | --- | --- | --- |
| 1 | README.md:175 | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux -DYAKOGL_PVR_DRIVER_DIR=/path/to/powervr-bxs-am62p/src` (literally) | 1 | 0 s | P1: the path does not exist; CMake says "point it at driver/src" |
| 2 | README.md:175 | the same with `-DYAKOGL_PVR_DRIVER_DIR=<clone>/driver/src` | 0 | 0 s | none |
| 3 | README.md:177 | `cmake --build build` | 0 | 31 s | none |
| 4 | README.md:182 | `cmake -S . -B build-a64 --toolchain aarch64.cmake ...` | not run | - | P2: no `aarch64.cmake` in the clone; also needs a cross compiler |
| 5 | README.md:184 | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf- DRVSRC=<clone>/driver/src` | 2 | 1 s | P3: `aarch64-none-elf-gcc` is not installed; the README does not list it |
| 6 | README.md:185 | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' ...` | not run | - | skipped: needs a cross C toolchain (the install of system packages is not allowed) |
| 7 | README.md:187 | Zephyr build from `west.yml` | not run | - | skipped: needs a Zephyr workspace and a board image. The Zephyr rows of step 14 did build images |
| 8 | README.md:299 | `cargo test --no-default-features --features host-test,gpu-powervr` | 0 | 25 s | P4: 1,986 + 6 tests passed, 1 ignored; README says 1492 + 6, 2 ignored |
| 9 | README.md:300 | `... --features host-test,gpu-powervr,report-es2` | 0 | 52 s | none (1,949 passed, 1 ignored) |
| 10 | README.md:301 | `... --features host-test,gpu-vivante` | 0 | 26 s | none (1,864 passed) |
| 11 | README.md:302 | `... --features host-test,gpu-powervr,shaderc-online` | 0 | 337 s | none (2,778 passed, 2 ignored); slow on a cold tree |
| 12 | README.md:303 | `... --features host-test,gpu-vivante,shaderc-online` | 0 | 340 s | none (2,724 passed, 2 ignored) |
| 13 | README.md:304 | `cargo test -p yakogl-tools` | 0 | 1 s | P4: 102 tests passed; README says 74 |
| 14 | README.md:296, CONTRIBUTING.md:11 | `tools/check.sh` (full gate) | 1 | 2937 s | P5, P6: 139 ok, 2 failed, 0 gaps. Both failures are the AM62P Zephyr `-Werror` rows (swi_tables.ld not found). CONTRIBUTING says about 45 minutes; it took 49 |
| 15 | README.md:297, CONTRIBUTING.md:24 | `tools/check.sh --quick` | 0 | 823 s | P7: 110 ok, 0 failed. It runs far more than "the PowerVR row and the AM62P archive only" |
| 16 | README.md:278 | `tools/api_coverage.py` | 0 | 1 s | none (it ran inside step 14: 0 core entry points missing) |
| 17 | README.md:344 | `grep -rn 'ignore = "BUG' src` | 0 (1 match) | 0 s | P8: README says there are none left; `src/pvr/backend_tests.rs:4601` has one |
| 18 | README.md:348 (README.md:363 on main at `ba25def`) | `HH_TIMEOUT=60 tools/hostharness/compare.sh build/hh-out rust:. rust:. driver` (the driver checkout is the in-tree `driver/`; `<out>` under the workspace's ignored `build/`; `HH_TIMEOUT` set through `bash -c 'export HH_TIMEOUT=60; ...'`) | 1 | 3436 s (57 min) | P9, P10. Ran to the end and wrote the report: 2,952 runs per profile (dev and release, A and B), 17 GB of logs, free disk fell by about 10 GB. A against B: dev 35 unexpected, release 25 unexpected. All 60 are timeouts, none a real difference: `gltest.174` both sides timed out (23), `gltest.25` both sides timed out (20) and cut at different lines, `scen.progs-qt3d` A timed out and B finished (15; the 60 s limit under 12 parallel jobs), and one each of `gltest.26` and `gltest.25` with A timed out and B ok. Both sides are the same library (`rust:.` against itself), so no regression is expected and none was seen outside timeouts. The `gl_test` cases that fail on the fake GPU fail the same way on both sides. Without `HH_TIMEOUT` (the default 1200 s) the run is not practical: an earlier literal run was stopped after 11 minutes with 21 GB of logs, and a later one after 25 minutes at 140 GB in the first of four runs, because `gltest.174` (`fwccbloss`) writes about 1.9 GB per mode per 10 minutes. Not done: a comparison against `HEAD~1` as a git worktree, because `git worktree add` was refused in this session |
| 19 | CONTRIBUTING.md:29-39 | branch, rebase and commit rules; regenerate generated files | n/a | - | no command; the gate's `gen` rows (step 14) passed |
| 20 | CONTRIBUTING.md:50-62 | `tools/board.sh` and the board rules | not run | - | skipped: needs board |
| 21 | CONTRIBUTING.md:64-69 | `driver/tools/mkfwblob.sh` with `rgx.fw` | not run | - | skipped: needs TI's firmware (licensed, not in the repository) |
| 22 | CONTRIBUTING.md:71-85 | licensing and security rules | n/a | - | no command |

## Fixes proposed

Each fix names the line and the exact change. Neither file was edited here.

- **P1, README.md:175-176 and README.md:184, 186.** The placeholder
  `/path/to/powervr-bxs-am62p/src` fails at configure time on a fresh clone. The driver
  is in the repository. Replace `-DYAKOGL_PVR_DRIVER_DIR=/path/to/powervr-bxs-am62p/src`
  with `-DYAKOGL_PVR_DRIVER_DIR=driver/src`, and `DRVSRC=/path/to/powervr-bxs-am62p/src`
  (line 184) and `DRVSRC=...` (line 186) with `DRVSRC=$PWD/driver/src`. Also check the
  default `../src` in the Options table (README.md:201); a fresh clone has no such
  directory, so the default should read `driver/src`.
- **P2, README.md:181-182.** `aarch64.cmake` is not in the repository. Change the comment
  to `# cross build: a CMake toolchain file of your own (none is shipped); the Rust target`
  and keep the rest.
- **P3, README.md:183-184.** Add after line 183: `# needs an aarch64-none-elf GCC
  (aarch64-none-elf-gcc, -ar) on PATH; without one, use the clang form below`.
- **P4, README.md:299 and README.md:304.** The counts are out of date and will drift
  again. Change `# 1492 + 6 tests, 2 ignored` to `# about 2,000 tests`, and
  `# 74 tests, the host tools` to `# the host tools`. Add to line 302: `# about 6
  minutes on a cold tree`.
- **P5, CONTRIBUTING.md:16-17.** Change `about 45 minutes on a warm tree` to
  `about 50 minutes on a warm tree`.
- **P6, CONTRIBUTING.md:18-23.** The gate failed on the two AM62P Zephyr `-Werror` rows
  with zero gaps: the generated `snippets-rodata.ld` holds a relative path to the Zephyr
  workspace that no longer resolves from a clone in the temp directory. I did not find
  the cause, so this is a lead: it may depend on where the clone sits relative to
  `ZEPHYR_WS`. Until the owner decides, add after line 23: `Keep the clone and ZEPHYR_WS
  under one parent directory: the AM62P Zephyr rows failed on a clone in another tree
  (see docs/release/readme-walkthrough.md).` The i.MX8MP and Raspberry Pi 4 Zephyr rows
  passed in the same run.
- **P7, README.md:297 and CONTRIBUTING.md:24-25.** `--quick` ran 110 rows in 823 s: the
  fmt check, the PowerVR, GC7000 and V3D configurations, the build checks, the drivers and
  the generated-file checks. Change `#    the PowerVR build only, for a tight edit loop`
  to `#    the short gate (about 14 minutes), not the landing gate`, and change
  `(the PowerVR row and the AM62P archive only)` to `(about 14 minutes; fewer rows than
  the full run)`.
- **P8, README.md:343-346.** Change `there are none left (`grep -rn 'ignore = "BUG' src`),
  because the fix round either fixed the bug or ...` to `there is one left
  (`grep -rn 'ignore = "BUG' src`: pvr-msaa-split, see docs/HANDOFF.md), ...`, or fix that
  defect and keep the text.
- **P9, README.md:348-358 (and tools/hostharness/README.md:43-66).** The command works
  literally with `driver` (this repository's `driver/`) as the driver checkout, but the text
  says nothing of its cost: with the default 1200 s timeout, case 174 alone writes tens of
  GB per run and fills a disk. Add after line 358: `Plan for an hour and about 20 GB
  of free disk with HH_TIMEOUT=60 (3436 s and 17 GB on a 12-thread host); the default timeout (1200 s) lets gl_test case 174 write
  more than 20 GB per library and profile. The driver checkout is driver/ in this
  repository.`
- **P10, README.md:353-355.** `exits non-zero when a log differs in a way allow.txt does not
  explain` is true, but a clean comparison needs the timeout out of the picture: with
  `HH_TIMEOUT=60` the same library against itself exits 1 (35 unexpected in dev, 25 in
  release on the latest run), because the jobs that hit the timeout (gltest.174, gltest.25,
  and scen.progs-qt3d on one side only) are cut at different lines. Either list those two jobs in
  tools/hostharness/allow.txt for a timed-out run, or have tools/hostharness/compare.sh
  (the `report.py compare` call at line 44) skip a pair in which both sides timed out.
  Say so in the README line quoted above.
- **Not a README problem, for the owner.** The Zephyr, firmware and board
  steps (7, 20, 21) need setup that a host-only contributor cannot do; the two docs say so
  already.

## Second walk (commit `ca4288a8`)

The same walk, two days later, on a new clone. The text above is kept as the record of
the first walk. Problems found in this walk are named S1 to S13, so they do not mix with
P1 to P10 above.

- Date: 2026-10-13. Every row below was run and measured on this date.
- Clone: `git clone` of `origin` (it could be fetched) into a new directory under the
  system temp directory. The clone was at `ca4288a8`, the tip of `origin/main` at that
  time. Cargo's target directory was `.target` inside the clone (`CARGO_TARGET_DIR`).
  `README.md` and `CONTRIBUTING.md` are the same on `ca4288a8` and on `b7d7a79`, the
  base of this change (`git diff` is empty), so every line reference below holds on both.
- Host: Linux 7.2.8 (x86-64, Gentoo), rustc 1.98.0-nightly (2026-07-01),
  cargo 1.98.0-nightly, cmake 4.3.5, ninja 1.13.2, GCC 16.2.1, Python 3.14.8,
  clang and LLVM 23. Cargo and `cc` ran through a build-cache shim on `PATH`. The Rust
  targets `aarch64-unknown-none` and `aarch64-unknown-none-softfloat` were installed.
  No `aarch64-none-elf` GCC. The Zephyr workspace, NXP HAL, `pco_shc` and firmware
  header that CONTRIBUTING.md:32-37 lists were set up, so no row was a gap.
- Order: the README order, then the CONTRIBUTING steps that README does not cover. All
  rows ran one after the other from one script; nothing else ran on the clone.
- Times: wall-clock time of the whole command. The first gate run (row 9) built
  everything cold; later rows reuse its build. Exit: the command's own exit status.
- Not done, by rule: board steps, serial consoles, credentials, the PowerVR firmware.

### Steps

| # | Source | Command | Exit | Time | Problem |
| --- | --- | --- | --- | --- | --- |
| 1 | README.md:171 | `rustc --version` (Rust 1.88 or newer) | 0 | 0 s | none (1.98.0-nightly) |
| 2 | README.md:186 | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 0.3 s | none |
| 3 | README.md:187 | `cmake --build build` | 0 | 31.3 s | none (cold) |
| 4 | README.md:192 | `cmake -S . -B build-a64 --toolchain aarch64.cmake -DYAKOGL_OSAL=baremetal` (the `...` left out) | 1 | 0.0 s | S1: "Could not find toolchain file: aarch64.cmake". S2: the failed step leaves a `build-a64/` directory that git does not ignore |
| 5 | README.md:194 | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf- DRVSRC=driver/src` | 2 | 0.3 s | S3: `aarch64-none-elf-gcc` is not installed, and the README does not say it is needed (cc-rs fails, then make stops) |
| 6 | README.md:195-196 | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy DRVSRC=...` (literally) | 2 | 20.1 s | S4: `src/osal/baremetal.c:25:10: fatal error: 'string.h' file not found`: clang has no bare-metal C headers here, and the README does not say a sysroot is needed. `DRVSRC=...` is also a placeholder |
| 7 | README.md:197-199 | Zephyr build from `west.yml` | not run | - | skipped: needs a Zephyr workspace build of its own. The gate's Zephyr rows (rows 9 and 11) build the images |
| 8 | README.md:288 | `tools/api_coverage.py` | 0 | 0.1 s | S5: run on its own after rows 2-3, it finds no `.target/debug/libyakogl_capi.a` and falls back to parsing the source (its output says to run `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table` first). S6: it reports 304 exported and 304 resolvable names; README.md:291 and 293 say 298 |
| 9 | README.md:306, CONTRIBUTING.md:14-15 | `tools/check.sh --changed origin/main` | 1 | 2,146 s (36 min) | S2, S7, S8. It saw one changed path, `build-a64/` from row 4, and so ran every row: "126 ok, 23 failed, 0 gap(s)". CONTRIBUTING.md:15 says a clean tree runs two rows. 21 failures are S8, 2 are S7 |
| 10 | README.md:307, CONTRIBUTING.md:38 | `tools/check.sh --quick` | 1 | 258 s (4.3 min, warm) | S8, S9: "114 ok, 5 failed, 0 gap(s)", 119 rows, not "the PowerVR row and the AM62P archive only". The 5 failures are S8 |
| 11 | README.md:308, CONTRIBUTING.md:26-37 | `tools/check.sh` (full gate) | 1 | 620 s (10.3 min, warm) | S7, S8, S10: "126 ok, 23 failed, 0 gap(s)", the same rows as row 9. The time is low because the failing rows stop at the build |
| 12 | README.md:310 | `cargo test --no-default-features --features host-test,gpu-powervr` | 101 | 0.8 s | S8: `error[E0063]: missing fields query_begin, query_end and query_result in initializer of YakoglBackend` at `tests/es3_tex_levels.rs:93`; no test ran |
| 13 | README.md:311 | `... --features host-test,gpu-powervr,report-es2` | 101 | 0.4 s | S8: the same error |
| 14 | README.md:312 | `... --features host-test,gpu-vivante` | 101 | 0.3 s | S8: the same error |
| 15 | README.md:313 | `... --features host-test,gpu-powervr,shaderc-online` | 101 | 0.4 s | S8: the same error |
| 16 | README.md:314 | `... --features host-test,gpu-vivante,shaderc-online` | 101 | 0.3 s | S8: the same error |
| 17 | README.md:315 | `cargo test -p yakogl-tools` | 0 | 0.2 s | none (113 passed; built by row 9) |
| 18 | README.md:357-358 | `git grep -n 'ignore = "BUG' -- src tests` | 0 | 0.1 s | S11: 3 matches (one in `src/pvr/backend_tests.rs`, two in `tests/es2_report_guard.rs`); README says five, four of them in `tests/es2_report_guard.rs` |
| 19 | README.md:363 | `HH_TIMEOUT=60 tools/hostharness/compare.sh <out> rust:. rust:. driver` | 1 | 2,529 s (42 min) | S12, S13: the clone as both libraries, `driver/` as the driver checkout, `<out>` in the temp directory. It wrote 50 GB and ended "UNEXPECTED DIFFERENCES": 2,952 runs per profile; 9 unexpected in dev and 11 in release. All 20 are `gltest.174` with both sides timed out and cut at different lines; nothing else differed. The default timeout (1,200 s) was not run: both earlier walks stopped it when it filled the disk (P9 above) |
| 20 | CONTRIBUTING.md:43-55 | branch, commit and generated-file rules | n/a | - | no command; the gate's generated-file rows passed in rows 9 and 11 |
| 21 | CONTRIBUTING.md:66-78 | `tools/board.sh` and the board rules | not run | - | skipped: needs board |
| 22 | CONTRIBUTING.md:80-85 | `driver/tools/mkfwblob.sh` with `rgx.fw` | not run | - | skipped: needs TI's firmware (licensed, not in the repository) |
| 23 | CONTRIBUTING.md:87-101 | licensing and security rules | n/a | - | no command |

### Fixes proposed (second walk)

`README.md` and `CONTRIBUTING.md` were not edited here.

- **S1, README.md:188 and 192.** `aarch64.cmake` is not shipped. Change line 188
  `# cross build: a CMake toolchain file; the Rust target is derived from` to
  `# cross build: a CMake toolchain file of your own (none is shipped); the Rust target is derived from`.
- **S2, README.md:192.** `build-a64/` is outside the ignored `build/`, so after this
  step `tools/check.sh --changed origin/main` sees a changed path and runs every row
  (row 9: 36 minutes, not seconds). Change `-B build-a64` to `-B build/a64`.
- **S3, README.md:193-194.** Change `# bare metal (RUST_TARGET defaults to aarch64-unknown-none)`
  to `# bare metal (RUST_TARGET defaults to aarch64-unknown-none); needs an
  aarch64-none-elf GCC on PATH, or use the clang form below`.
- **S4, README.md:195-196.** Change `DRVSRC=...` to `DRVSRC=driver/src`, as on line 194,
  and add after line 196: `# clang needs bare-metal C headers (a newlib sysroot, given
  with --sysroot in CC)`.
- **S5, README.md:288.** After `tools/api_coverage.py` add `(after cargo build -p
  yakogl-capi --features gpu-powervr,mesa-reference-table; without that archive it
  parses the source instead)`.
- **S6, README.md:291 and 293.** Change both `(298 names` to `(304 names`, or drop the
  counts, since they drift: `the names it exports` and `the names it resolves`.
- **S7, CONTRIBUTING.md:32-37.** The two AM62P Zephyr `-Werror` rows failed with
  `fatal error: ... swi_tables.ld: No such file or directory`; the i.MX8MP and
  Raspberry Pi 4 rows passed. This is the failure the first walk saw (P6 above). Its
  likely cause, read from the generated `snippets-rodata.ld` and Zephyr's code in this
  walk, but not yet confirmed by a build with the real path: Zephyr writes the `#include` of a linker snippet relative
  to `${ZEPHYR_BASE}/include` (`zephyr_linker_sources()`, Zephyr's
  `cmake/modules/extensions.cmake:1456`). `tools/check.sh:2519` sets
  `ZEPHYR_BASE="$ZEPHYR_WS/zephyr"`. On this host `ZEPHYR_WS` is a symbolic link, while
  CMake gives the snippet's real path. The relative path is right on paper, but the
  compiler follows the link before it reads `..`, so the `..` steps end at the file
  system root and the file is looked for there. Only AM62P uses `swi_tables.ld`, so only its rows fail. It does
  not depend on where the clone is, so P6's proposal above is wrong. Add after
  CONTRIBUTING.md:37: `ZEPHYR_WS must be a real path, not a path through a symbolic
  link: the AM62P Zephyr rows fail on a linked one (swi_tables.ld not found).` The
  better fix is in `tools/check.sh:2519`: use `ZEPHYR_BASE="$(cd "$ZEPHYR_WS" && pwd
  -P)/zephyr"` (and the same for `cd`), so the rows work with a linked workspace too.
- **S8, not a README problem: `tests/es3_tex_levels.rs:93`.** `YakoglBackend` now has
  `query_begin`, `query_end` and `query_result` (`src/backend.rs:1261-1269`). The
  `backend()` initializer in that test does not set them, so no `cargo test` row of
  the README builds. This is the same on `b7d7a79`. Add `query_begin: None,
  query_end: None, query_result: None,` to that initializer, as
  `tests/es2_fragdata.rs:256` does. Nothing in README.md or CONTRIBUTING.md has to
  change for this.
- **S9, README.md:307 and CONTRIBUTING.md:38.** `--quick` ran 119 rows. Change
  `the PowerVR build only, for a tight edit loop` to `the short gate (about 120 rows),
  for a tight edit loop`, and `(the PowerVR row and the AM62P archive only)` to
  `(about 120 rows; fewer than the full run)`. No time is proposed: this walk only ran
  it warm (4.3 minutes), the first walk took 14 minutes.
- **S10, CONTRIBUTING.md:31.** Change `about 45 minutes on a warm tree` to
  `about 35 minutes from a cold tree and 10 to 50 minutes on a warm one, depending on
  the host`. This walk: 36 minutes cold (row 9, every row), 10 minutes warm (row 11,
  but with the S8 rows failing early); the first walk: 49 minutes.
- **S11, README.md:357-359.** Change `Five are open today` to `Three are open today`,
  and `four ES 2.0 report pins` to `two ES 2.0 report pins`. Or drop the counts and keep
  the `git grep` command.
- **S12, README.md:363-373.** Add after line 373: `With HH_TIMEOUT=60 a run takes about
  45 minutes and 50 GB of disk. With the default timeout (1200 s), gl_test case 174
  fills a disk. Pass your own checkout of the driver as the last argument; in this
  repository it is driver/.`
- **S13, README.md:366-367.** A tree compared with itself is not quiet: here all 20
  unexpected differences were `gltest.174` timing out on both sides, cut at different
  lines (the first walk saw more such timeout pairs, P10 above). Add after `does not
  explain`: `(a job that times out on both sides is reported too; read the report
  before you call a difference a regression)`. Or, as P10 says, have the compare skip a
  pair in which both sides timed out.
