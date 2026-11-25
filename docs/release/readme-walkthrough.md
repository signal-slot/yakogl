<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# README and CONTRIBUTING walkthrough

This file records walks of the README host steps and CONTRIBUTING's commands
(checklist lines 3 and 6) and keeps the findings of the earlier walks that are
still open. It holds current state only: the run logs of the 2026-10 walks
(`ea984992`, `ca4288a8`, `a63fcc5b`) and their path-check tables are retired;
what they found is in the table below and in the commits named there. Open work
is in the GitHub issues, not in a file.

The walks are below, from "Line 3" on. Each names its date, commit, host and
each command with its exit code and time. Line 3 ends with the `--quick`
summary line, or says there is none.

## Findings of the earlier walks, at the tip

| Finding | What | Status |
| --- | --- | --- |
| S1, S2 | README's cross build needed a toolchain file nobody ships, and `build-a64/` was not ignored | fixed: README says the toolchain file is the reader's own, and uses `build/a64` (inside the ignored `build/`) |
| S3, S4 | `baremetal.mk` needs an `aarch64-none-elf` GCC; the clang form needs bare-metal C headers; `DRVSRC=...` was a placeholder | fixed: README names both forms; `DRVSRC` defaults to `driver/src` (`cde4864d`, `ccc51125`) |
| S5, S6 | `tools/api_coverage.py` falls back to the source without the archive; the name counts drifted | fixed: README gives the cargo build line first and prints no counts |
| P7, S9 | `--quick` described as "the PowerVR build only" | fixed in README and CONTRIBUTING.md, which gives the `--quick` size and times. The `tools/check.sh` header still says "about 147 rows" (177 listed on 2026-11-10): **open**, a `tools/check.sh` text for that file's owner |
| P8, S11 | the count of open `ignore = "BUG` tests | fixed: README drops the count and keeps the `git grep` (none open on 2026-10-26) |
| P9, S12, P10, S13 | cost of the host-harness compare; a clean comparison is not quiet when jobs time out | fixed: README states `HH_TIMEOUT=60`, 40 to 60 minutes, 17 to 50 GB, and the timed-out-on-both-sides remark |
| S8 | `tests/es3_tex_levels.rs` lacked the query fields | fixed in the source |
| N9 | `MESA_V3D_TREE` without `build-v3d` failed `--quick` | fixed: the three v3d rows become one gap naming `MESA_V3D_TREE` (a gap, not a pass); CONTRIBUTING.md says to unset it or point it at a built tree |
| N10 | an untracked file outside the ignored paths widens `--changed` to every row | fixed: CONTRIBUTING.md says so |
| P6, S7 | AM62P Zephyr rows: `swi_tables.ld` not found through a symbolic link | fixed: `zephyr/linker_snippets.cmake` rewrites the snippet includes to absolute paths; `tools/check.sh` still resolves the workspace by its real path |
| N3 | `tools/check.sh` defaults `HAL_NXP_DIR` to a path under `$HOME` that fits one host only | **open**, `tools/check.sh` (`hal_nxp_dir=`): a host without that layout sees a gap that names `HAL_NXP_DIR`, which is the intended message |
| N4 | CONTRIBUTING said a pushed branch is never rebased | fixed: CONTRIBUTING.md now says to rebase onto `origin/main` and push with `--force-with-lease` |
| N5 | SECURITY.md contact placeholder and `TODO(owner)` lines | no longer present at the tip; checklist line 4 carries its own state |
| N7 | `DESIGN.md` stated per-board conformance numbers | no longer present at the tip; the numbers live in README "Status" and `tools/cts/RESULTS.md` |
| gate from a symlinked path | `cd -P` at the top of `tools/check.sh` is the fix, not applied | **open**: `docs/notes/gate-symlinked-path.md`; CONTRIBUTING.md says to run the gate from the canonical path |

## Not covered by any walk

The Zephyr build from `west.yml` outside the gate's image rows, the host-harness
compare (about an hour and 17 to 50 GB), board steps (`tools/board.sh`), serial
consoles, credentials, and the PowerVR firmware (`driver/tools/mkfwblob.sh`
needs TI's firmware). They need hardware or files a fresh host does not have.

## Checklist cells

Checklist lines 3 and 6 take their State from "Line 3" and "Line 6" below. The state words
are in `docs/release/checklist.md`; the walks decide, not this file.

## Line 3

CONTRIBUTING.md's build, test and gate commands, walked on a gate host.

- Date: 2026-11-25. Main commit walked: `d6d93303` (2026-11-25 07:10 UTC), a
  fresh clone, no build directory. The walking host could not reach GitHub, so
  the commit is the newest `origin/main` ref the workspace held, not a live
  fetch.
- Host: x86-64 Linux, 4 cores, rustc 1.99.1 (the walk's report did not record
  the kernel). `tools/setup-gate-host.sh --check` exits 0 with the layout
  exported and the PowerVR firmware header copied in: every item is ok,
  including the aarch64 compiler, `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`,
  `GLMARK2_SRC` and the firmware header. This host would have had no gap row.
  Its disk was the problem (below).

| Command | Exit | Wall time |
| --- | --- | --- |
| `tools/cts/deps-env.sh --check` (layout missing `MP2_QEMU`) | 1 | not recorded |
| `tools/cts/deps-env.sh --build ...` (as CONTRIBUTING.md writes it) | 0 | not recorded |
| the PowerVR header copy | 0 | not recorded |
| `tools/setup-gate-host.sh --check` | 0 | not recorded |
| `tools/check.sh --changed origin/main` (no change) | 0 | not recorded |
| `tools/check.sh --quick` | stopped, red, **no summary line** | stopped after 18 of 487 rows |
| `tools/check.sh --local origin/main` | not run | |
| `tools/check_scope_selftest.sh` | not run | |

Every CONTRIBUTING command before `--quick` exited 0, apart from the first
`deps-env.sh --check`, which exits 1 until the next command builds the missing
entry. The report that carried this walk gave exit codes but no per-command
times, so none is written here.

**Result: not green.** There is no `tools/check.sh --quick` summary line at this
commit, and none is invented here. Two reasons:

- Disk: the first 18 of the 487 `--quick` rows used about 16 GB of build output
  on a host with about 3 GB free (later 20 GB), so the run was stopped. A
  re-walk needs about 50 GB free, or a pruned gate cache (it was over 160 GB),
  with `CARGO_INCREMENTAL=0`. `tools/setup-gate-host.sh --check` exits 0 even
  with only 18 GB free: it does not check disk before a walk.
- A red clippy row before the stop: `host-test,gpu-v3d,shaderc-online,no-shader-table`
  fails with "`format!` in `format!` args" at `tests/es3_tf_v3d.rs:448`, which
  came in with `78741eae` (2026-11-24). A re-walk is red on that row until a
  `tests:` commit fixes it.

CONTRIBUTING's statements, checked at the tip:

- Plan sizes (`YAKOGL_CHECK_PLAN=1`): `--quick` 487 rows and the full plan 547
  at `d6d93303`. They move with main and no file keeps them.
- A docs-only `tools/check.sh --no-clippy --changed <parent>` plans 4 rows:
  holds.
- `MESA_V3D_TREE` set to a tree without `build-v3d` turns the three v3d rows
  into one gap (483 ok, 1 gap in the plan listing): holds.
- The flag order is `--no-clippy --changed <base>`; with `--no-clippy` last,
  `tools/check.sh` ignores it and keeps clippy.

The last complete `--quick` summary line is the 2026-11-12 one, from a fresh
clone of `570cd8c6`: "181 ok, 0 failed, 0 gap(s)" in 3121 s. It is not a
reading of the current tip.

## Line 6

README's host steps in order, then CONTRIBUTING, on a fresh clone.

- Date: 2026-11-24. Main commit walked: `cb36d3c7` (2026-11-24 15:04 UTC), the
  newest `origin/main` ref the workspace held (GitHub was not reachable). Fresh
  `git clone`, detached at that commit, no build directory at the start; each
  `make` row used its own `BUILD=` directory.
- Host: Linux 7.0.0-43-generic x86-64, rustc 1.99.1 (stable,
  `aarch64-unknown-none` installed), cargo 1.99.1, cmake 4.2.3, ninja 1.13.2,
  GNU Make 4.4.1, Ubuntu clang 21.1.8 with LLVM 21.1.8 (`llvm-ar-21`,
  `llvm-nm-21`, `llvm-objcopy-21`), Zephyr SDK 1.0.1 (`aarch64-zephyr-elf-gcc`
  14.3.0), Arm GNU Toolchain 14.3.rel1 (`aarch64-none-elf`, newlib). Every
  passing build log had 0 warnings.

| Step | Command as written | Exit | Wall time |
| --- | --- | --- | --- |
| 1 configure | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | <1 s |
| 1 build | `cmake --build build` | 0 | 35 s |
| 2 configure | `cmake -S . -B build/a64 --toolchain <reader's toolchain file> -DYAKOGL_OSAL=baremetal` | 0 | <1 s |
| 2 build | `cmake --build build/a64` | 0 | 35 s |
| 3 GCC form, `aarch64-none-elf` GCC on PATH | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-` | 0 | 23 s |
| 3 GCC form, host without that GCC | the same command | 2 | <1 s |
| 3 GCC form, Zephyr SDK 1.0 prefix | `make -f baremetal.mk CROSS_COMPILE=<sdk>/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-` | 0 | 16 s |
| 3 clang form, no sysroot | `make -f baremetal.mk CC='clang --target=aarch64-none-elf' AR=llvm-ar-21 NM=llvm-nm-21 OBJCOPY=llvm-objcopy-21` | 2 | 15 s |
| 3 clang form, `--sysroot` (Arm toolchain) | the same with `CC='clang --target=aarch64-none-elf --sysroot=<arm-gnu>/aarch64-none-elf'` | 0 | 22 s |
| 3 clang form, `--sysroot` (Zephyr SDK newlib) | the same with `--sysroot=<sdk>/gnu/aarch64-zephyr-elf/aarch64-zephyr-elf` | 0 | 18 s |
| 4 Zephyr (`west`) | not run: `west` is absent on this host | | |
| host tools test | `cargo test -p yakogl-tools` | 0 | 27 s |
| API coverage prep | `cargo build -p yakogl-capi --features gpu-powervr,mesa-reference-table` | 0 | 9 s |
| API coverage | `python3 -I tools/api_coverage.py` | 0 | 1 s |
| API coverage, no archive | the same with an empty cargo target directory | 0 | <1 s |

All `baremetal.mk` rows used the default `DRVSRC` (`DRVSRC ?= $(HERE)/driver/src`
in `baremetal.mk`); none passed it. The two exit-2 rows are the documented host
requirements: the GCC form needs an `aarch64-none-elf` GCC (`failed to find tool
"aarch64-none-elf-gcc"`), and the clang form needs bare-metal C headers
(`'string.h' file not found` at `src/osal/baremetal.c:25`) that `--sysroot`
supplies. Outputs: `cargo test -p yakogl-tools` "128 passed; 0 failed";
`api_coverage.py` `SUMMARY declared=186 exported=304 procaddress=304`, missing
0, from the archive (`nm`) and, with no archive, from the source: the same 304.

The criterion of line 6 ends in a passing `tools/check.sh --quick` with no gap.
That part is the Line 3 walk above, which has no summary line at this commit.
