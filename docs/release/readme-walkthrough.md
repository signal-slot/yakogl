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

**Not met.** The newest `--quick` walk is still the red one at `a06902bd`. `e52f2e3f` re-indents the list that made it red; its re-walk was not run (the host had 20 GB free against the 50 GB needed, and no `MP2_QEMU`).

**Fresh clone of `a06902bd` (2026-11-26), red on six clippy rows.** `env LC_ALL=C tools/check.sh --quick` in a fresh clone of the then-current integration tip (the walked commit `a06902bd` is itself the commit that fixed the two `nanopc_t6` clippy sites that made the walk before it red), started detached so the run outlives the 2 h command limit (`LANG=C` and `CARGO_INCREMENTAL=0` set too), ran on 2026-11-26 and ended with the summary line:

> check.sh: 553 ok, 6 failed, 0 gap(s) (a gap was not run: it is not a pass), 0 skipped (every row), 0 clippy rows left to the nightly, 2 not run (passed on main) in 7666s; test binaries: 2010 run, 0 skipped (passed on main); runner: cargo-nextest 0.9.148 (cd1d6d546 2026-10-07) (the rows that read libtest's output run the binaries concurrently)

after 7666 s (check.sh's own figure; 7667 s wall clock), exit 1. No row was a gap: the run exported the layout of `tools/cts/deps-env.sh` (CONTRIBUTING's form), copied `driver/src/fw/pvr_fw_blob.h` from `PVR_FW_BLOB` without committing it (a fresh clone has no `driver/src/fw/`: `mkdir -p driver/src/fw` before the copy), and built `MP2_QEMU` for the walk (exit 0, 5 m 14 s), which closed the uboot-mp2 qemu row. The host is the same prepared gate host as the walk before it (Linux 7.0.0-38-generic x86-64, rustc 1.99.1, clippy 0.1.99 following the host's nightly); its stale cargo cache was pruned before the walk (172 GB free), and the run reused the warm target cache, so its time is not a cold-build reading. The run prints 559 rows (553 ok, 6 failed), so green reads "559 ok, 0 failed, 0 gap(s)". The failed rows and their owner:

- Six clippy rows, all with the same two warnings: `doc list item without indentation` at `tests/depth_resolve_pvr_edges.rs:22:5` and `:23:5`. The file came in with `8748a5ca`; its `//!` bullet list at lines 20–25 has continuation lines that are not indented past the list marker. One small commit re-indenting that list closes all six rows; until it lands the tree is not green on `--quick`.

The walk before this one, at `d13c9558` ("523 ok, 2 failed, 0 gap(s)" in 7501 s), was red only on the two `yakogl-drivers` clippy rows (`driver/rust/src/board/nanopc_t6_go.rs:700` `redundant_comparisons`, `driver/rust/src/board/nanopc_t6_frame_tests.rs:42` `assertions_on_constants`); `a06902bd` fixed both, and they are green in this walk. The `shaderc71_mali_testdata` pair count fixed by `8c4fae25` and the self-test rows red in the earlier walks (`tools/cts/imx95-gles2-accept-selftest.sh`, the kexec-zephyr `t6-verdict-test.sh` self-test, and `tools/cts/imx8mp-glmark2-wrongscene-selftest.sh`, fixed by `e758afdb`) pass at `a06902bd`: the walk's only failed rows are the six above.

Before the gate: at the same-day walk of `4f34ffb8`, every CONTRIBUTING command before the gate exited 0, apart from the first `tools/cts/deps-env.sh --check`, which exits 1 until the next command builds the missing entry. Nine `tools/` rows fail on a host that exports a locale it has not installed and pass under `LC_ALL=C`; the gate exports no locale, so export `LC_ALL=C` and keep `TMPDIR` outside every git checkout.

The last green `--quick` is "181 ok, 0 failed, 0 gap(s)" at `570cd8c6` (2026-11-12, 3121 s). It is not a reading of the tip.

To close line 3: a fresh-clone walk of `e52f2e3f` (which re-indents the `//!` bullet list at `tests/depth_resolve_pvr_edges.rs`) or later, `env LC_ALL=C tools/check.sh --quick` started detached with a wall budget above 7667 s, ending "N ok, 0 failed, 0 gap(s)" with N equal to the plan listing (`YAKOGL_CHECK_PLAN=1 tools/check.sh --quick`) at the walked commit (562 rows at `e52f2e3f`). For the next walk: `MP2_QEMU` is built in the host's dependency cache (`$CONNECTED_DEPS_DIR`), and the stale `ci-target/debug` cache is pruned (172 GB free), so it is a warm rebuild.

CONTRIBUTING's statements, checked at `4f34ffb8`:

- Plan sizes (`YAKOGL_CHECK_PLAN=1`): `--quick` 495 rows and the full plan 555. CONTRIBUTING.md at this commit states 480 and 540 (its `ab919063` reading); the sizes move with main and no file keeps them.
- A docs-only `tools/check.sh --no-clippy --changed <parent>` plans 4 rows: held at `d6d93303` and `4326fbd9`.
- `MESA_V3D_TREE` set to a tree without `build-v3d` turns the three v3d rows into one gap: holds.
- The flag order is `--no-clippy --changed <base>`; with `--no-clippy` last, `tools/check.sh` ignores it and keeps clippy.

## Line 6

README's host steps in order, then CONTRIBUTING, on a fresh clone.

**At `4f34ffb8` (2026-11-26): step 4 (`west`) ran, exit 0.** In a fresh workspace all five README step-4 commands and the first `west build` exited 0 (`west init -m <repo> --mr main <workspace>` from the root manifest), and all 33 Zephyr image rows of the gate exit 0 with no gap. README's host steps and the API-coverage step are recorded below at `9f286b5e` (2026-11-26) — the cmake Linux build, the host tools test and the API-coverage step — and step 3's bare-metal forms in the `40b44800` re-walk below it; no README host-step change since is known to alter them. The last part of the criterion, a passing `tools/check.sh --quick` with no gap, is line 3's reading: "553 ok, 6 failed, 0 gap(s)" at `a06902bd`, red (section "Line 3"). Line 6 is not met: it is partial until line 3 is met.

The `9f286b5e` walk (2026-11-26, the cmake Linux build, the host tools test and
the API-coverage step; step 2's cross build was not re-run — no toolchain file
is shipped and README says it is the reader's own — and step 3 is the
`40b44800` re-walk below):

- Date: 2026-11-26. Main commit walked: `9f286b5e`, the
  integration tip when the walk ran. Fresh `git clone`, no build directory at
  the start. All commands ran with `LC_ALL=C` (the host's `ja_JP.UTF-8` locale
  is not installed).
- Host: Linux 7.0.0-38-generic x86-64, rustc 1.99.1 (58cf8f9c3 2026-10-13).

| Step | Command as written | Exit | Wall time |
| --- | --- | --- | --- |
| 1 configure | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | <1 s |
| 1 build | `cmake --build build` | 0 | 66 s |
| host tools test | `cargo test -p yakogl-tools` | 0 | 57 s |
| API coverage | `python3 -I tools/api_coverage.py` | 0 | 1 s |

Outputs: `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` ended `-- Build
files have been written to: <workspace>/build`; `cmake --build build` `--
assembled <workspace>/build/libyakogl.a (73 runtime symbols made local)`;
`cargo test -p yakogl-tools` "128 passed; 0 failed"; `api_coverage.py` `SUMMARY
declared=186 exported=304 procaddress=304`, missing 0, and `PINNED
missing_gl2=0 missing_egl=0`. The `cmake --build` and `cargo test` times
include a cold build in the fresh clone, on the host's shared cargo cache. The
archive `api_coverage.py` read (`exported_source=nm`) was a `libyakogl_capi.a`
in the host's cargo target cache (`<home>/.cache/yakogl/ci-target/debug/`),
not built in the clone; with no such cache the script parses the source
instead — the fallback README names — which was not timed here.

Step 3 alone, re-walked 2026-11-26 at `40b44800` (host Linux 7.0.0-38-generic
x86-64, rustc 1.99.1, Ubuntu clang 21.1.8 with the LLVM tools installed only
versioned). The Arm GNU Toolchain was fetched with its sha256 checked
(`ebaf2d47...3d2be9`, equal to Arm's `.sha256asc`): `aarch64-none-elf-gcc
(Arm GNU Toolchain 14.3.Rel1 (Build arm-14.174)) 14.3.1 20250623`, kept at
`<deps>/arm-gnu/arm-gnu-toolchain-14.3.rel1-x86_64-aarch64-none-elf` for
later walks. Each row ran in a fresh clone with `build/bm` removed
beforehand and the default `DRVSRC`; `<arm-gnu>` is that toolchain
directory.

| Step | Command as written | Exit | Wall time |
| --- | --- | --- | --- |
| 3 GCC form, Arm GNU on `PATH` | `make -f baremetal.mk CROSS_COMPILE=aarch64-none-elf-` | 0 | 55 s |
| 3 clang form, `--sysroot`, versioned tools | `make -f baremetal.mk CC='clang --target=aarch64-none-elf --sysroot=<arm-gnu>/aarch64-none-elf' AR=llvm-ar-21 NM=llvm-nm-21 OBJCOPY=llvm-objcopy-21` | 0 | 47 s |
| 3 clang form, unversioned tool names | the same with `AR=llvm-ar NM=llvm-nm OBJCOPY=llvm-objcopy` | 2 | 3 s |

The third row fails with `failed to find tool "llvm-ar"` on this host;
README already says the tools may need their versioned names, so it is the
documented host condition, not a README fault. Step 3 is thereby verified
in both forms at a current commit; the "not verified" line and the "To
close the line" note of "Line 6: cross builds (2026-11-05)" in
`docs/release/checklist.md` are closed by this table; that older record
text in the checklist itself is not rewritten here.

The criterion of line 6 ends in a passing `tools/check.sh --quick` with no gap.
That part is line 3's newest reading: red at `a06902bd` ("553 ok, 6 failed,
0 gap(s)"). The older readings at `8f1a5f71`, `4f34ffb8`, `4326fbd9`,
`6a54408d` and `d13c9558` are superseded.
