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

**Fresh clone of `4f34ffb8` (2026-11-26), red.** `tools/check.sh --quick` under `LC_ALL=C` ran to its summary line twice: the first read "487 ok, 8 failed, 0 gap(s)", the second (final) "488 ok, 7 failed, 0 gap(s)". The plan lists 495 rows, so green reads "495 ok, 0 failed, 0 gap(s)"; no row was a gap (the host carried `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`, an aarch64 compiler; `MESA_V3D_TREE` unset). A cache of 68 GB was pruned before the walk; the run needs about 50 GB free with `CARGO_INCREMENTAL=0`.

Red rows and causes:

- The `--quick` clippy rows that use the feature sets of the 14 clippy rows that are red at `4f34ffb8` (clippy 0.1.99): `src/v3d/texstate71.rs:15` duplicated `allow(dead_code)`, `tests/v3d71_texfmt.rs:259` `type_complexity`, `tests/v3d71_gltest_rpi5_conf.rs:162` constant assertion, `tests/v3d_pi4_aborts2_refusals.rs:447` nested `format!` (fixed by `36c51e9a`), and in the Mali row `tests/mali_link_varying_offsets.rs:129-130`, `tests/mali_rebind_stencil_gl.rs:375`, `src/mali/frame_uniarray_tests.rs:388` (fixed by `6a54408d`). Re-running the 14 rows at `217f0a19` gives exit 0 and 0 warnings.
- `tools/cts/imx95-gles2-accept-selftest.sh`: its fixture records say `verdin_imx95` while the tally tool accepts only `verdin-imx95`. A fix is not in the tree (still red on main `90d422fe`); it is a two-file change.
- Not recorded per row: which of the seven failed rows are the quick-reached clippy rows and which the self-test; the walk's per-row list is not part of what this file was given. A re-walk at a commit with both fixes reads the rest.

Before the gate, every CONTRIBUTING command exited 0, apart from the first `tools/cts/deps-env.sh --check`, which exits 1 until the next command builds the missing entry. Nine `tools/` rows fail on a host that exports a locale it has not installed and pass under `LC_ALL=C`; the gate exports no locale, so export `LC_ALL=C` and keep `TMPDIR` outside every git checkout.

The last green `--quick` is "181 ok, 0 failed, 0 gap(s)" at `570cd8c6` (2026-11-12, 3121 s). It is not a reading of the tip.

To close line 3: a fresh clone of a commit at or after `6a54408d` that also carries the self-test fix, `--quick` under `LC_ALL=C` on a host where `tools/setup-gate-host.sh --check` exits 0, ending "495 ok, 0 failed, 0 gap(s)".

CONTRIBUTING's statements, checked at `4f34ffb8`:

- Plan sizes (`YAKOGL_CHECK_PLAN=1`): `--quick` 495 rows and the full plan 555. CONTRIBUTING.md at this commit states 480 and 540 (its `ab919063` reading); the sizes move with main and no file keeps them.
- A docs-only `tools/check.sh --no-clippy --changed <parent>` plans 4 rows: held at `d6d93303` and `4326fbd9`.
- `MESA_V3D_TREE` set to a tree without `build-v3d` turns the three v3d rows into one gap: holds.
- The flag order is `--no-clippy --changed <base>`; with `--no-clippy` last, `tools/check.sh` ignores it and keeps clippy.

## Line 6

README's host steps in order, then CONTRIBUTING, on a fresh clone.

**At `4f34ffb8` (2026-11-26): step 4 (`west`) ran, exit 0.** In a fresh workspace all five README step-4 commands and the first `west build` exited 0 (`west init -m <repo> --mr main <workspace>` from the root manifest), and all 33 Zephyr image rows of the gate exit 0 with no gap. README's host steps 1 to 3 and the API-coverage step are the `cb36d3c7` walk below; no README host-step change since is known to alter them. The last part of the criterion, a passing `tools/check.sh --quick` with no gap, is line 3's reading: "488 ok, 7 failed, 0 gap(s)" at `4f34ffb8`, red (section "Line 3"). Line 6 is partial until line 3 is met.

The `cb36d3c7` walk (2026-11-24, host steps 1 to 3 and the API-coverage step; step 4 was not run there because `west` was absent on that host):

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
That part is line 3's newest reading above: red at `4f34ffb8` ("488 ok, 7 failed,
0 gap(s)"). The older readings at `4326fbd9` and `6a54408d` are superseded.
