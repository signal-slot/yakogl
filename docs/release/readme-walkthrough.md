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

**Fresh clone of `8f1a5f71` (2026-11-26), red on one row.** `env LC_ALL=C tools/check.sh --quick` in a fresh clone, started detached so the run outlives the 2 h command limit (a single background wait is killed at about 2 h; re-arm waiters on the walked process's real pid, not on `$!`), ended with its summary line "508 ok, 1 failed, 0 gap(s)" after 10760 s (exit 1). No row was a gap: the run used the layout of `tools/cts/deps-env.sh` (CONTRIBUTING's form), whose entries were in the host's dependency cache (`<deps>`); `MP2_QEMU` was built for the walk (about 5 min). The host is a prepared gate host (its walk report names the dependency cache, not the toolchain versions; Linux 7.0.0-38-generic x86-64, rustc 1.99.1 — the versions the step-3 walk of "Line 6" below records on the same date, under the same host requirements). The run counts 509 rows (508 ok + 1 failed), so green reads "509 ok, 0 failed, 0 gap(s)". The failed row and its owner:

- `host-test,gpu-v3d,shaderc-online,no-shader-table`: the `shaderc71_mali_testdata` test asserts 1800 pairs while `tools/mali/testdata` now has 1814 — a stale count in the test, not a code defect; plan #387 raises `PAIRS` (`COMPILED_42` and `COMPILED_71` may follow). `tests/shaderc71_mali_testdata.rs` still asserts 1800 at this reading. The two rows red in the walks before their fixes, `tools/cts/imx95-gles2-accept-selftest.sh` and the kexec-zephyr `t6-verdict-test.sh` self-test, pass at `8f1a5f71`.

Before the gate: at the same-day walk of `4f34ffb8`, every CONTRIBUTING command before the gate exited 0, apart from the first `tools/cts/deps-env.sh --check`, which exits 1 until the next command builds the missing entry. Nine `tools/` rows fail on a host that exports a locale it has not installed and pass under `LC_ALL=C`; the gate exports no locale, so export `LC_ALL=C` and keep `TMPDIR` outside every git checkout.

The last green `--quick` is "181 ok, 0 failed, 0 gap(s)" at `570cd8c6` (2026-11-12, 3121 s). It is not a reading of the tip.

To close line 3: a fresh clone of a commit at or after the `PAIRS` bump, `env LC_ALL=C tools/check.sh --quick` started detached on this host with a wall budget above 10760 s, ending "509 ok, 0 failed, 0 gap(s)" (the row count moves with main).

CONTRIBUTING's statements, checked at `4f34ffb8`:

- Plan sizes (`YAKOGL_CHECK_PLAN=1`): `--quick` 495 rows and the full plan 555. CONTRIBUTING.md at this commit states 480 and 540 (its `ab919063` reading); the sizes move with main and no file keeps them.
- A docs-only `tools/check.sh --no-clippy --changed <parent>` plans 4 rows: held at `d6d93303` and `4326fbd9`.
- `MESA_V3D_TREE` set to a tree without `build-v3d` turns the three v3d rows into one gap: holds.
- The flag order is `--no-clippy --changed <base>`; with `--no-clippy` last, `tools/check.sh` ignores it and keeps clippy.

## Line 6

README's host steps in order, then CONTRIBUTING, on a fresh clone.

**At `4f34ffb8` (2026-11-26): step 4 (`west`) ran, exit 0.** In a fresh workspace all five README step-4 commands and the first `west build` exited 0 (`west init -m <repo> --mr main <workspace>` from the root manifest), and all 33 Zephyr image rows of the gate exit 0 with no gap. README's host steps and the API-coverage step are recorded below at `50e70464` (2026-11-26) — the cmake Linux build, the host tools test and the API-coverage step — and step 3's bare-metal forms in the `40b44800` re-walk below it; no README host-step change since is known to alter them. The last part of the criterion, a passing `tools/check.sh --quick` with no gap, is line 3's reading: "508 ok, 1 failed, 0 gap(s)" at `8f1a5f71`, red (section "Line 3"). Line 6 is partial until line 3 is met.

The `50e70464` walk (2026-11-26, the cmake Linux build, the host tools test and
the API-coverage step; step 2's cross build was not re-run — no toolchain file
is shipped and README says it is the reader's own — and step 3 is the
`40b44800` re-walk below):

- Date: 2026-11-26. Main commit walked: `50e70464` (2026-11-26 05:42 UTC), the
  integration tip when the walk ran. Fresh `git clone`, no build directory at
  the start. All commands ran with `LC_ALL=C` (the host's `ja_JP.UTF-8` locale
  is not installed).
- Host: Linux 7.0.0-38-generic x86-64, rustc 1.99.1 (58cf8f9c3 2026-10-13).

| Step | Command as written | Exit | Wall time |
| --- | --- | --- | --- |
| 1 configure | `cmake -S . -B build -G Ninja -DYAKOGL_OSAL=linux` | 0 | 2 s |
| 1 build | `cmake --build build` | 0 | 85 s |
| host tools test | `cargo test -p yakogl-tools` | 0 | 66 s |
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
That part is line 3's newest reading: red at `8f1a5f71` ("508 ok, 1 failed,
0 gap(s)"). The older readings at `4f34ffb8`, `4326fbd9` and `6a54408d` are
superseded.
