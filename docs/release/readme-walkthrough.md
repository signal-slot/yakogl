<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# README and CONTRIBUTING walkthrough

This file records walks of the README host steps and CONTRIBUTING's commands
(checklist lines 3 and 6) and keeps the findings of the earlier walks that are
still open. It holds current state only: the run logs of the 2026-10 walks
(`ea984992`, `ca4288a8`, `a63fcc5b`) and their path-check tables are retired;
what they found is in the table below and in the commits named there. Open work
is in the GitHub issues, not in a file.

The walks are below, from "Line 6: cross builds" on. Each names its date, base
commit, each command with exit code and time, and the `--quick` summary line.

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

Checklist lines 3 and 6 take their State from the walks below. The state words
are in `docs/release/checklist.md`; the walks decide, not this file.

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
| 3d. clang form with a distribution libc, `GPU=powervr` and `GPU=vivante` | `make -f baremetal.mk CC='clang --target=aarch64-none-elf -isystem <picolibc include dir>' AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy`, plus a link of a minimal test program against `libyakogl.a`; `-isystem` is the only change to the README's `CC` string (no `--sysroot`), and no Arm GNU toolchain is used | 0 (the archive build and the test link, for `GPU=powervr` and for `GPU=vivante`) | `8d1488cd` (item picolibc-link; measured there) | clang 21.1.8, `ld.lld` 21.1.8; `lld-21` and `liblld-21` are not installed by default on this host and were fetched the same no-root way | Ubuntu picolibc: package `picolibc-aarch64-linux-gnu` 1.8.10-3 from the Ubuntu archive pool (universe/p/picolibc), unpacked without root with `apt-get download` and `dpkg-deb -x` | archive sizes not recorded; no libc symbol that `libyakogl.a` needs is missing from picolibc, so the tree needs no change; the test link needs `stdout`, `stderr` and `_exit` from the integrator; the handoff reported an lld linker-script quirk, its details were not recorded |
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
- The same clang form also works with a distribution C library, without the Arm GNU
  toolchain: Ubuntu's `picolibc-aarch64-linux-gnu`, `-isystem <picolibc include dir>`
  in `CC` in place of `--sysroot` (the only change to the README's `CC` string); the
  link step needs `ld.lld` (`lld-21` and `liblld-21` are not installed by default), and
  `stdout`, `stderr` and `_exit` for the test link come from the integrator (row 3d).
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
   not a tree defect. A distribution route works too, without the Arm GNU
   toolchain: Ubuntu's picolibc (package picolibc-aarch64-linux-gnu 1.8.10-3,
   unpacked no-root), -isystem <picolibc include dir> in CC in place of
   --sysroot, ld.lld for the link: exit 0 for GPU=powervr and GPU=vivante at
   8d1488cd (row 3d).

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

No change is needed for the `DRVSRC` wording. Three host-quirk insertions follow, each
anchored to the README text above (grep the anchor; apply from the bottom up, or
re-grep, because (a) adds 3 lines and (c) adds 1). They rest on the rows and the
"Host quirks seen" notes of the section "Line 6: cross builds".

**(a) Step 2, SDK 1.0 prefix** (rows 2, 2'; first quirk note). Anchor: the line
`make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-`. Add above it:

```
# with the Zephyr SDK 1.0 the prefix is
# $ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-
# (the pre-1.0 $ZEPHYR_SDK_INSTALL_DIR/aarch64-zephyr-elf/bin/ path does not exist there)
```

**(b) Step 3, clang form** (rows 3, 3a, 3d; the sysroot, picolibc and llvm-name quirk
notes). Anchor: the three comment lines that start `# clang needs bare-metal C
headers:`. Replace them (this replaces the older optional sentence of this section) with:

```
# clang needs bare-metal C headers: a newlib sysroot, e.g. the aarch64-none-elf
# directory of the Arm GNU toolchain (without one the build stops at 'string.h').
# It also needs llvm-ar, llvm-nm and llvm-objcopy under those names on PATH;
# Ubuntu installs them unversioned only in /usr/lib/llvm-21/bin and puts just the
# -21 names in /usr/bin, so add that directory to PATH or pass AR=llvm-ar-21 etc.
# Without the Arm GNU toolchain Ubuntu's picolibc-aarch64-linux-gnu works too:
# CC='clang --target=aarch64-none-elf -isystem <picolibc include dir>' in place of
# --sysroot; linking a program against it also needs ld.lld (package lld-21 or lld)
```

**(c) Step 4, stale ZEPHYR_BASE** (the ZEPHYR_BASE quirk note; recorded for form (a), the
`west.yml` workspace, row 4b; no row records it for the `ZEPHYR_EXTRA_MODULES` form 4a,
so that form is not named). Anchor: the line
`# Zephyr: a workspace from west.yml (driver/samples/zephyr/README.md), or add this`.
Add above it:

```
# for the west.yml workspace unset a ZEPHYR_BASE left over from another workspace
```
