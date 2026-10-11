<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (merge `77c860056`, ccfarm `01a12a9b-34ae-72d8-b1f7-d34954f8dccf`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# README and CONTRIBUTING walkthrough on a fresh clone

> Status added when landed on main (2026-10-12): the findings are of commit `ea984992` (2026-10-11). Read against main's README.md and CONTRIBUTING.md before acting: P1 (the `/path/to/powervr-bxs-am62p/src` placeholder) is gone from README.md; P2 (`aarch64.cmake`, README.md line 191) and P5 ("about 45 minutes", CONTRIBUTING.md line 31) are still in the text. The other findings were not re-checked.

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
| 18 | README.md:348 | `tools/hostharness/compare.sh <out> rust:. rust:. driver` (the driver checkout is the in-tree `driver/`; `<out>` and `CARGO_TARGET_DIR` under the workspace's ignored `build/`) | 1 | 2519 s (42 min) with `HH_TIMEOUT=60` | P9, P10. Literal run (default 1200 s timeout) stopped by me after 11 min with 21 GB of logs and no end in sight: gl_test case 174 (`fwccbloss`) writes about 1.9 GB per mode, per 10 min, in each of 12 modes, on the fake GPU; 4 runs would need well over the 73 GB free. Rerun with `HH_TIMEOUT=60` (documented in tools/hostharness/run.sh:26): 2,832 runs per profile, dev 14 unexpected, release 16 unexpected, all in gltest.174 and gltest.25, which hit the timeout in both builds and are cut at different lines. Every other job agrees between A and B (identical library, so no real regression is expected). The `gl_test` cases that fail on the fake GPU (case 1 `uniform`, case 10 `prims`, and others) fail the same way on both sides and are not counted |
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
  GB per run and fills a disk. Add after line 358: `Plan for 40 minutes and about 20 GB
  of free disk with HH_TIMEOUT=60; the default timeout (1200 s) lets gl_test case 174 write
  more than 20 GB per library and profile. The driver checkout is driver/ in this
  repository.`
- **P10, README.md:353-355.** `exits non-zero when a log differs in a way allow.txt does not
  explain` is true, but a clean comparison needs the timeout out of the picture: with
  `HH_TIMEOUT=60` the same library against itself exits 1, because the two jobs that hit the
  timeout (gltest.174, gltest.25) are cut at different lines. Either list those two jobs in
  tools/hostharness/allow.txt for a timed-out run, or have tools/hostharness/compare.sh
  (the `report.py compare` call at line 44) skip a pair in which both sides timed out.
  Say so in the README line quoted above.
- **Not a README problem, for the owner.** The Zephyr, firmware and board
  steps (7, 20, 21) need setup that a host-only contributor cannot do; the two docs say so
  already.
