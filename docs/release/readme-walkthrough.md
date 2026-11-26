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

Three walks of 2026-11-25 are recorded, oldest first. Walk 1 covers the
CONTRIBUTING commands before the gate; walks 2 and 3 are the `--quick` rows.
None of the three produced a green `--quick` line at the tip, and none is
invented here.

**Walk 1 — fresh clone of `d6d93303` (2026-11-25 07:10 UTC; stopped on disk).**
The walking host could not reach GitHub, so the commit is the newest
`origin/main` ref the workspace held, not a live fetch; a fresh clone, no
build directory. Host: x86-64 Linux, 4 cores, rustc 1.99.1 (the walk's report
did not record the kernel). `tools/setup-gate-host.sh --check` exits 0 with
the layout exported and the PowerVR firmware header copied in: every item is
ok, including the aarch64 compiler, `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`,
`GLMARK2_SRC` and the firmware header; this host would have had no gap row.
Every CONTRIBUTING command before `--quick` exited 0, apart from the first
`tools/cts/deps-env.sh --check`, which exits 1 until the next command builds
the missing entry (`--check`, `--build`, the PowerVR header copy,
`tools/check.sh --changed origin/main` with no change; exit codes recorded,
per-command wall times not). `tools/check.sh --quick` was stopped after 18 of
its 487 rows: those rows used about 16 GB of build output on a host with about
3 GB free (later 20 GB). A re-walk needs about 50 GB free, or a pruned gate
cache (it was over 160 GB), with `CARGO_INCREMENTAL=0`.
`tools/setup-gate-host.sh --check` exits 0 even with only 18 GB free: it does
not check disk before a walk. That run was red before the stop, on the clippy
row `host-test,gpu-v3d,shaderc-online,no-shader-table` ("`format!` in
`format!` args" at `tests/es3_tf_v3d.rs:448`, in from `78741eae`); fixed by
`8625ca04`. `tools/check.sh --local origin/main` and
`tools/check_scope_selftest.sh` were not run.

**Walk 2 — fresh clone of `4326fbd9` (2026-11-25 12:40 UTC; ran to its
summary line, red).** The completed `--quick` walk of these tips: a fresh
clone, 4 cores, a cold tree, 68 GB pruned from the shared gate cache before
the run. The host exports `LC_ALL=ja_JP.UTF-8`, a locale the host has not
installed. Kernel and rustc are not recorded. Exit 1; verbatim:

`check.sh: 474 ok, 16 failed, 0 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 clippy rows left to the nightly, 2 not run (passed on main) in 5980s; test binaries: 1917 run, 0 skipped (passed on main); runner: cargo-nextest 0.9.148 (cd1d6d546 2026-10-07) (the rows that read libtest's output run the binaries concurrently)`

The plan lists 492 rows, and `0 gap(s)` means the host carried every input the
gap rows read (`ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`, an aarch64 compiler;
`MESA_V3D_TREE` unset). The 16 failed rows, each with its cause from the
walk's report:

- six clippy rows, naming `tests/v3d71_texfmt.rs`, `src/v3d/texstate71.rs`,
  `tests/mali_link_varying_offsets.rs`, `src/mali/frame_uniarray_tests.rs`
  and `tests/mali_rebind_stencil_gl.rs`: fixed since by `36c51e9a` and
  `6a54408d`;
- nine `tools/` rows, failing because the host exports a locale it has not
  installed; the walk re-ran those nine scripts under `LC_ALL=C` and all nine
  pass. `tools/check.sh` exports no locale, and `tools/setup-gate-host.sh
  --check` does not report one: those fixes belong to plans #344/#387 and to
  this plan's `fix-setup-locale` item;
- one code defect: `tools/cts/imx95-gles2-accept-selftest.sh` fails even
  under `LC_ALL=C` — its fixture records say `verdin_imx95` while the tally
  tool accepts only `verdin-imx95` — with a redaction hit reported in
  `tools/cts/log/note.txt`; fixed since by `83fc2e44` (plan #407's patch).

Under `LC_ALL=C` at the tip, `tools-rows-lcc` measured every non-cargo row:
the nine locale rows pass, but the `imx95-gles2-accept` self-test and
`driver/tools/kexec-zephyr/t6-verdict-test.sh` are red (the latter's fix is a
running item of this plan, not in the tree at the walked commits), and the
long rows took `am62p-gles3-acceptance-selftest.sh` 1506 s and
`cts-selftest.sh` 937 s.

**Walk 3 — checkout of `6a54408d` (2026-11-25 20:17 UTC, the integration tip
at walk start, also on `origin/main`; killed, no summary line).** The re-walk
after the clippy fixes, in an existing checkout of that commit, not a fresh
`git clone` (a fresh clone is part of the line 3 criterion, so this walk does
not meet it). Host: Linux 7.0.0-38-generic x86-64, rustc 1.99.1
(`58cf8f9c3 2026-10-13`). `tools/setup-gate-host.sh --check` exits 1 and
lists three missing items: an aarch64 C compiler, `GLMARK2_SRC` and
`driver/src/fw/pvr_fw_blob.h`; a completed `--quick` on this host would end
with gap rows, and a gap is not a pass. `tools/check.sh --quick` ran with
`LC_ALL=C` and `CARGO_INCREMENTAL=0`; verbatim from the walk's report:
"`tools/check.sh --quick` has no summary line: the sandbox killed it at the
2-hour limit after 108 `ok` rows and no failures or gaps. The rows that never
ran are untested." 108 rows ok, 0 failed, 0 gap(s) at the kill point; every
row after it is unmeasured, including the `imx95-gles2-accept` self-test row
(whose fix `83fc2e44` landed one hour after the walked commit).

**Result: not green.** No `--quick` run of the 2026-11-25 tip reached
"N ok, 0 failed, 0 gap(s)":

- walk 2 ran to its line and is red, "474 ok, 16 failed, 0 gap(s)"; its
  failing rows are fixed in the tree except that the nine locale rows need
  `LC_ALL=C` (the gate exports no locale) and `t6-verdict-test.sh` is red
  under `LC_ALL=C` until that running item lands;
- walk 3 re-walked the fixed tip under `LC_ALL=C` and never reached a summary
  line (killed at the harness's 2-hour background limit after 108 ok rows),
  on a host that could not give a 0-gap run;
- the last green `--quick` line remains the 2026-11-12 one, from a fresh clone
  of `570cd8c6`: "181 ok, 0 failed, 0 gap(s)" in 3121 s. It is not a reading
  of the current tip.

To close line 3: a completed `--quick` from a fresh clone, on a host where
`tools/setup-gate-host.sh --check` exits 0 and the disk is sufficient, under
`LC_ALL=C`, with a wall budget above the 5980 s walk 2 took (the run must be
detached, since a 2-hour command window cannot hold the plan).

CONTRIBUTING's statements, checked at the tip:

- Plan sizes (`YAKOGL_CHECK_PLAN=1`): `--quick` 487 rows and the full plan 547
  at `d6d93303`; 492 and 552 at `4326fbd9`. CONTRIBUTING.md states 494 and 554
  at its `512f3297` reading. The sizes move with main and no file keeps them.
- A docs-only `tools/check.sh --no-clippy --changed <parent>` plans 4 rows:
  holds at `d6d93303` and at `4326fbd9`.
- `MESA_V3D_TREE` set to a tree without `build-v3d` turns the three v3d rows
  into one gap (483 ok, 1 gap in the plan listing): holds.
- The flag order is `--no-clippy --changed <base>`; with `--no-clippy` last,
  `tools/check.sh` ignores it and keeps clippy.

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
That part is the Line 3 walk above: red at `4326fbd9` ("474 ok, 16 failed,
0 gap(s)"), and its newest re-walk at `6a54408d` reached no summary line.
