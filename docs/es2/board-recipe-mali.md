<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 (Mali-G610): the dEQP-GLES2 recipe, library, runner and kernel messages

Plan #327 (ROADMAP.md stage 5, issues #7 and #20). The NanoPC-T6 (RK3588, Arm
Mali-G610, back end `gpu-mali`) runs yakogl's Mali back end on its own Armbian
through the panthor kernel driver. **Nothing in this recipe has run dEQP on the
board yet.** Only `gl_test` sweeps are recorded (`tools/cts/RESULTS.md`, "Mali").
Every "expected" line below is the form the tools print, taken from their
sources and from a run of their self-tests on a host, not a board result.

The pieces, in the order a session uses them: the library (section 1), the
build (2), the tree table (3), the runner (4) and its self-test (5), the
kernel message classifier (6), the tally (7) and the board rules (8).

| piece | state | what |
|---|---|---|
| `tools/mali/panthor-gles/` | on main | `libGLESv2.so.2` and `libEGL.so.1` over panthor (section 1) |
| `tools/cts/mali-deqp-build.sh` | on main | cross-builds `deqp-gles2` for the board's Armbian (section 2) |
| `tools/cts/mali-gles2-tree.txt` | on main | the tree table: the mustpass list in rows (section 3) |
| `tools/cts/mali-deqp-run.sh` | on main | the runner: one chunk per row, a slice per session (section 4) |
| `tools/cts/mali-deqp-selftest.sh` | on main | its self-test, no board (section 5) |
| `tools/cts/mali-kmsg.py`, `mali-kmsg-selftest.sh` | on main | the panthor message classifier (section 6) |
| `tools/cts/mali-deqp-tally.py`, `mali-deqp-tally-selftest.sh` | on main | the tally over slices (section 7) |
| the board rules | on main | section 8 |

## 1. The library: `tools/mali/panthor-gles/`

dEQP's `yakogl` target (`tools/cts/cts-yakogl-target.patch`) opens
`libEGL.so.1` and `libGLESv2.so.2` at run time and looks every entry point up
by name. The directory builds that pair for the Mali over panthor (the Rust
development OSAL of `tools/mali/panthor`, render node `/dev/dri/renderD128`,
or `YAKOGL_MALI_DEV`). Its own README lists the files; what a session needs:

```sh
tools/mali/panthor-gles/build.sh [--debug] [--es3] <outdir>
tools/mali/panthor-gles/check.sh <outdir>
```

`build.sh` writes both objects for `aarch64-unknown-linux-gnu` into
`<outdir>`. It needs cargo with that target, clang, and for the first build
curl or python3, `ar` and `tar` with xz; the linker is `ld.lld` or the
toolchain's `rust-lld`. `libGLESv2.so.2` holds all the code and exports the
`gl*` and the `egl*` names (one process-wide state); `libEGL.so.1` is a
forwarder (SONAME `libEGL.so.1`, `DT_NEEDED libGLESv2.so.2`, `RUNPATH
$ORIGIN`), so the pair must sit in one directory.

**The floors.** The board's ABI is Debian trixie's: GLIBC 2.41, GLIBCXX
3.4.33. `build.sh` links against a Debian trixie arm64 sysroot (four pinned
packages, versions and sha256 in the script) unpacked once into
`$YAKOGL_MALI_SYSROOT`, else `$CONNECTED_DEPS_DIR/debian-trixie-arm64-sysroot`.
`check.sh` (host, no board) proves the result: AArch64 shared objects with
their SONAMEs, the `RUNPATH`, the constructor, every one of the 186 names of
`src/deqp-entry-points.txt` (44 EGL, 142 ES 2.0) defined, and **no version
need above `GLIBC_2.41` or `GLIBCXX_3.4.33`**. It exits 0 when all pass; a
library that needs a newer glibc than the board's fails to load on the board,
so a build is not used before this check is green.

**The trap.** The board's Armbian ships glvnd and Mesa 25.0.7, which provide
a `libGLESv2.so.2` and a `libEGL.so.1` with the same two SONAMEs. A dEQP
started without the right `LD_LIBRARY_PATH` loads the board's pair and runs
Mesa instead of yakogl, and its results would be
Mesa's, silently. That is why the library prints, once per process at the
first `eglInitialize`, on stderr:

```text
yakogl-mali: loaded build=<git describe> lib=<path of this .so>
yakogl-mali-exit: draws=<n> refused=<n> first_refused=<name or -> gpu_faults=<n> job_timeouts=<n>
```

Only yakogl's EGL prints the first line. A run is taken as yakogl's only if
the process's stderr has `yakogl-mali: loaded` (and `lib=` is the staged
`<stage>/lib/libGLESv2.so.2`). The second line comes at process exit
(`atexit`); it is absent when the process died, which is itself a finding.
`first_refused` is the first refusal by name and may hold spaces (take
everything between `first_refused=` and the last ` gpu_faults=`). The runner
(section 4) enforces the first line; the second is kept per chunk.

The host test of the library: `cargo test --manifest-path
tools/mali/panthor-gles/Cargo.toml` (the lines' formats, the refusal names,
the wait classification, the two lines end to end with no device).

## 2. The deqp-gles2 build: `tools/cts/mali-deqp-build.sh`

```sh
tools/cts/mali-deqp-build.sh --out DIR [--gles-lib DIR] [--dry-run] [--allow-dirty]
```

It cross-builds `deqp-gles2` (VK-GL-CTS at its pinned `CTS_COMMIT` with
`tools/cts/cts-yakogl-target.patch`, the `yakogl` target: no GL or EGL library
is linked, both are `dlopen()`ed) on an x86_64 host for the board's Armbian
trixie (aarch64), and writes the build directory section 4 reads.

| option | meaning |
|---|---|
| `--out DIR` | the build directory; absent or empty beforehand, written as `DIR.part` and renamed at the end |
| `--gles-lib DIR` | where `libGLESv2.so.2` and `libEGL.so.1` are (`tools/mali/panthor-gles/build.sh` makes them; this script does not); copied to `DIR/lib/`. Without it `DIR/lib/` is absent and the runner refuses the directory |
| `--dry-run` | check the arguments and the working tree, print every step, run none: no toolchain, CTS clone or build tree is needed |
| `--allow-dirty` | build from a tree with changes to tracked files; `build.txt` then says `dirty=yes` and the result is not a record |

The working tree must be clean, so `build.txt`'s `commit=` is the commit the
code came from; the runner refuses a build whose commit is not on
`origin/main`. Environment: `CTS` (a VK-GL-CTS clone holding `CTS_COMMIT`
with its `external/fetch_sources.py` sources; never written to), `TOOLCHAIN`,
`BUILD` (default `build/cts/mali-aarch64`) and `JOBS` (default `nproc`).

The inputs come from `tools/cts/deps.toml` through `tools/cts/deps-env.sh`:
`DEQP_WORK` makes the CTS clone, and `MALI_DEQP_TOOLCHAIN` (`board =
"nanopc_t6"`) unpacks Debian trixie's own `g++-14-aarch64-linux-gnu` 14.2.0
with its cross glibc 2.41 from pinned `.deb`s, without root or docker (an
x86_64 host with glibc 2.38 or newer). When the default toolchain directory is
absent the script runs `tools/cts/deps-env.sh --build --board nanopc_t6
MALI_DEQP_TOOLCHAIN` itself.

The board's floors are GLIBC 2.41, GLIBCXX 3.4.33, CXXABI 1.3.15. After the
build the script reads `objdump -T` of `deqp-gles2` and fails (nothing is
written to `DIR`) when a version is newer than a floor, when the program is
not AArch64 or does not use `/lib/ld-linux-aarch64.so.1`, or when
`libGLESv2`, `libEGL` or `libGL` is a NEEDED entry. On success it prints
`GLIBC_x GLIBCXX_y CXXABI_z: within the floors`.

The dry run, on a host with the tip checked out and clean (paths are the
host's; each `==` line is followed by the `+ command` lines the step would
run):

```sh
tools/cts/mali-deqp-build.sh --out build/x-b --dry-run
```

```text
mali-deqp-build: --dry-run: the steps below are printed, none is run
mali-deqp-build: commit=<40 hex> dirty=no cts=93bca01861b0e3ef3c387027a9791e6d065f900c
mali-deqp-build: CTS=<deps>/VK_GL_CTS TOOLCHAIN=<deps>/debian-trixie-arm64-cross BUILD=<repo>/build/cts/mali-aarch64
== 1. the Debian trixie arm64 cross toolchain (MALI_DEQP_TOOLCHAIN, tools/cts/deps.toml)
== 2. VK-GL-CTS 93bca01861b0e3ef3c387027a9791e6d065f900c with cts-yakogl-target.patch in <BUILD>/cts
== 3. configure and build deqp-gles2 in <BUILD>/build
== 4. check <BUILD>/build/modules/gles2/deqp-gles2: AArch64, /lib/ld-linux-aarch64.so.1, no GL/EGL link, objdump -T within GLIBC 2.41, GLIBCXX 3.4.33, CXXABI 1.3.15
== 5. write <repo>/build/x-b (through <repo>/build/x-b.part)
```

The real build was not run for this recipe. Its time, the measured symbol
floors and the program's size are in `build.txt` of the directory a real run
makes (`build_seconds=`, `glibc_max=`, `glibcxx_max=`, `cxxabi_max=`,
`deqp_gles2_bytes=`, with `commit=`, `cts=`, `cts_patch=`, `toolchain=` and
`floors=`). Check that directory with the runner's dry run (section 4) before
a session.

## 3. The tree table: `tools/cts/mali-gles2-tree.txt`

The 17,165-case `tools/cts/gles2-mustpass.txt` in 75 rows, one chunk (one
dEQP process, at most 400 cases) each, in the form of `check-gles3-tree.py`'s
reader: `# image T6S<session>R<row>: <count>`, then one comma-separated
`dEQP-GLES2....*` filter line. Labels restart in each session (`T6S2R01`); the
runner's `--rows A-B` counts rows over the whole file, 1-based.

| session | rows | cases | what |
|---|---|---|---|
| S1 | 1-24 | 5,141 | everything but `functional.shaders` and `functional.fragment_ops` |
| S2 | 25-38 | 4,872 | `fragment_ops` and `shaders.operator` |
| S3 | 39-67 | 4,477 | shaders: random, matrix, swizzles, loops, indexing |
| S4 | 68-75 | 2,675 | the other shaders groups |

The groups that hang or fault on other boards (loops, indexing, random,
texture, `fbo.render`, `draw`) stand alone in their rows, so a crash or GPU
hang loses one row. The table's header holds the same sessions as
`# session S<n>: rows A-B, N cases` lines.

Check it against the mustpass list (no board, a second or two):

```sh
python3 tools/cts/check-gles3-tree.py --mustpass tools/cts/gles2-mustpass.txt \
    --max 400 tools/cts/mali-gles2-tree.txt
```

It prints one `<label> <count>` line per row (`T6S1R01 224` ... `T6S4R08
119`) and ends with

```text
check-gles3-tree: tools/cts/mali-gles2-tree.txt: 75 images, 17165 matched once, 0 not matched, 0 matched more than once, 0 empty images
```

It exits non-zero when a mustpass name is in no row or in two rows, a row
selects nothing, or a row has more than `--max` cases. If
`gles2-mustpass.txt` changes, the table and the counts above change with it.
The runner and its self-test use the three-row
`tools/cts/testdata/mali-deqp-run/tree.txt` of the same form.

## 4. The runner: `tools/cts/mali-deqp-run.sh`

```sh
BOARD_HOST=root@<t6> tools/cts/mali-deqp-run.sh --build DIR --out SLICEDIR \
    --rows A-B [--tree FILE] [--session S1..S4|smoke] [--dry-run]
```

| option | meaning |
|---|---|
| `--build DIR` | a build directory with `deqp-gles2`, `data/` (holding `gles2/`), `build.txt` with a `commit=<hex>` line, and `lib/` with `libGLESv2.so.2` and `libEGL.so.1` |
| `--out SLICEDIR` | where the slice is written; must be absent or empty (a slice is never overwritten) |
| `--rows A-B` | the tree rows to run, 1-based, inclusive; outside the table is refused |
| `--tree FILE` | the tree table (default `tools/cts/mali-gles2-tree.txt`) |
| `--session S` | `S1`, `S2`, `S3`, `S4`, `smoke` or `adhoc` (default), recorded in `meta.txt` |
| `--dry-run` | print the plan and exit 0: no ssh, nothing staged, no slice |

`BOARD_HOST` is `<user>@<host>` of the lease (`target:nanopc-t6`), never a
file; `BOARD_SSH_OPTS` is passed to ssh and scp. Spell `BOARD_HOST` the way
`tools/mali/panthor/run-on-board.sh` and the kexec recipe do: the board lock is
one file per `BOARD_HOST` string. Environment knobs: `CHUNK_LIMIT` (seconds a
process may run, default `600 + 3 s a case`), `MAX_ATTEMPTS` (processes a
chunk, 100), `LOCK_WAIT` (0), `HOLD_MAX` (21600 s), `RAW_DIR` (raw logs),
`KMSG_SETTLE_S` (1), `STAGE_MARGIN_KB` (1 GiB), `DEQP_SURFACE` (256),
`REDACT_EXTRA`, and for odd hosts `MALI_DEQP_MAIN_REPO`, `MALI_DEQP_MAIN_REF`
(default this checkout and `origin/main`) and `MALI_DEQP_MUSTPASS`.

### The dry run

```sh
tools/cts/mali-deqp-run.sh --build tools/cts/testdata/mali-deqp-run/fixture-build \
    --out build/x-slice --rows 1-3 --tree tools/cts/testdata/mali-deqp-run/tree.txt \
    --session smoke --dry-run
```

Exit 0, nothing written. Output of a real run of that command on the tip
(`<repo>` stands for the checkout's absolute path. The fixture's commit
`009f62a5…` is one of origin/main's; the run quoted was made in a clone that
has no `origin/main` ref, hence `on origin/main: no (a real run refuses it)`
on the second line, which a real run would, exit 6. In a checkout with
`origin/main` the line ends `on origin/main: yes`; nothing else differs):

```text
mali-deqp-run: dry run: no ssh connection, nothing staged
mali-deqp-run: build <repo>/tools/cts/testdata/mali-deqp-run/fixture-build, commit 009f62a5f8e66357fc22d7d1e708943513c94467, on origin/main: no (a real run refuses it)
mali-deqp-run: library libGLESv2.so.2:bb21db53f5c7,libEGL.so.1:4a48c4778abc
mali-deqp-run: session smoke, rows 1-3 of tools/cts/testdata/mali-deqp-run/tree.txt: 3 row(s), 17 case(s)
mali-deqp-run:   row 1 T1: 6 case(s), limit 618 s a process
mali-deqp-run:   row 2 T2: 6 case(s), limit 618 s a process
mali-deqp-run:   row 3 T3: 5 case(s), limit 615 s a process
mali-deqp-run: stage $BOARD_HOST:/root/yakogl-mali/009f62a5/ (deqp-gles2, data/, lib/), removed at the end
mali-deqp-run: slice build/x-slice, raw logs <repo>/build/mali-deqp-run/x-slice
mali-deqp-run: per chunk <label>, process <k> (over ssh to $BOARD_HOST):
  nohup dmesg -w > /root/yakogl-mali/009f62a5/dmesg-<label>.log 2>&1 < /dev/null &
  echo 'yakogl-run-start mali-deqp <label> <stamp>' >> /dev/kmsg
  cd /root/yakogl-mali/009f62a5 && setsid -w timeout -k 10 <limit> env LD_LIBRARY_PATH=/root/yakogl-mali/009f62a5/lib /root/yakogl-mali/009f62a5/deqp-gles2 --deqp-caselist-file=/root/yakogl-mali/009f62a5/<label>.r<k>.txt --deqp-log-filename=/root/yakogl-mali/009f62a5/<label>.r<k>.qpa --deqp-archive-dir=/root/yakogl-mali/009f62a5/data --deqp-surface-type=pbuffer --deqp-surface-width=256 --deqp-surface-height=256 --deqp-log-images=disable --deqp-log-shader-sources=enable > /root/yakogl-mali/009f62a5/<label>.r<k>.out 2> /root/yakogl-mali/009f62a5/<label>.r<k>.err
  echo 'yakogl-run-end mali-deqp <label> <stamp>' >> /dev/kmsg; kill <dmesg pid>
  cat /proc/sys/kernel/random/boot_id
```

Read it: the commit and whether it is on origin/main; the two library
hashes; the row labels with their case counts and the per-process time limit;
the stage path; the board commands of one process. The case counts come from
the same reader the tree check uses, applied to the real mustpass list, so a
filter that matches no case or a wrong count shows here, without a board.

### What a real run does

1. **Refusals before the board is touched**: a missing or incomplete build
   directory, a `build.txt` without `commit=<hex>`, a non-empty `--out`, and a
   build commit that is not an ancestor of `origin/main` (exit 6): every
   result is built from one origin/main commit. Then `BOARD_HOST` must be set.
2. **The board lock**: sourced from `driver/tools/kexec-zephyr/board_lock.sh`,
   `LOCK_WAIT 0` (refuse, never queue), `HOLD_MAX` 21600 s. The lock is
   what keeps the runner out of another plan's session (#309, #311); the
   lease on `target:nanopc-t6` is what hands the board over.
3. **Board checks**: `fuser`, `dmesg`, `setsid` and `timeout` exist; nobody
   holds `/dev/dri/renderD128` (`fuser`; exit 3 if someone does, exit 4 if
   `fuser` itself fails); `boot_id` and `uname -r` are read into `meta.txt`.
4. **Staging** under `/root/yakogl-mali/<sha8>/` only (`/tmp` is tmpfs on the
   board): after a `df -Pk` check for the build's size plus `STAGE_MARGIN_KB`,
   `deqp-gles2`, `data/` and `lib/` are copied there. The stage is removed
   at the end, also on an early exit.
5. **The chunk loop**, one chunk per tree row A..B: a `dmesg -w` capture is
   started, `yakogl-run-start mali-deqp <label> <stamp>` is written to
   `/dev/kmsg`, then one `deqp-gles2` process runs over the row's cases
   (`--deqp-caselist-file`, pbuffer surface of `DEQP_SURFACE` pixels, images
   not logged, shader sources logged) with `LD_LIBRARY_PATH=<stage>/lib`
   under `setsid` and `timeout` (`CHUNK_LIMIT`), and the end marker closes
   the capture.
6. **Restarts**: a process that stops before the row's end (a crash, the
   timeout) is followed by a fresh one over the cases no process began
   (dEQP's `--deqp-caselist-file`). A process that begins none of the cases
   it was given loses the first of them (Lost `chunk-crash` for a crash,
   `chunk-hang` for the timeout) and the rest go on. A process that exits 0
   or 1 yet began none of its cases leaves them Lost `not-run`. At most
   `MAX_ATTEMPTS` processes per chunk; what is left then is Lost `not-run`.
7. **After each chunk**: does the board answer ssh (`ssh_after`), is
   `boot_id` the same (`boot_same`), how many kernel lines came between the
   markers and how many of them are panthor lines about a job timeout or a
   reset (`resets`; the finer classes are section 6). Every process's stderr
   must have `yakogl-mali: loaded`, else the chunk is Lost `wrong-lib` and the
   run stops (exit 9). The last `yakogl-mali-exit:` line of the chunk is kept.
8. **The end**: the second `boot_id` goes to `meta.txt`, the slice is
   written, the lock released.

### Lost reasons

| detail in `results.tsv` | meaning |
|---|---|
| `chunk-crash` | the case a process died in (a signal, exit other than 0/1/124/137), or the first case of a process that began none |
| `chunk-hang` | the same for the host-side `timeout` (rc 124/137) |
| `board-lost` | ssh stopped answering, or the board came back with another `boot_id`: the rest of the run |
| `not-run` | never begun: dEQP ended normally without them, `MAX_ATTEMPTS` was reached, the rows after a stop, or a process survived SIGKILL |
| `wrong-lib` | the process ran the board's own libGLESv2, not yakogl's: every case of that chunk |

A chunk's `status` in `chunks.tsv` is `done`, `crash`, `hang`, `fault` (the
chunk finished but panthor lines of a timeout or reset came between its
markers), or `lost`.

### Exit codes

| code | meaning |
|---|---|
| 0 | every chunk ran, whatever dEQP said (read the slice) |
| 1 | usage, or a refusal before the board (missing build, tree or mustpass, non-empty `--out`) |
| 3 | `/dev/dri/renderD128` is held by a process |
| 4 | the board or its tools: missing `fuser`/`dmesg`/`setsid`/`timeout`, ssh failed, no space, staging failed |
| 6 | the build's commit is not on origin/main |
| 7 | the `boot_id` changed (the board rebooted) |
| 8 | ssh to the board lost |
| 9 | wrong library (no `yakogl-mali: loaded`) |
| 10 | a `deqp-gles2` survives SIGKILL after a hang (a wedged ioctl) |

On 7, 8 and 10 the rest of the plan is written as Lost, the slice is still
written with what ran, and **the board needs a person: nothing is retried**.

### The slice

Written to `--out`, redacted (no host names, addresses, user names, MACs or
serials; `REDACT_WORDS` is built from `BOARD_HOST`, this host's names and
user, and `YAKOGL_REDACTION_PRIVATE`/`_FILE`):

| file | content |
|---|---|
| `meta.txt` | `key=value`, in this order: `commit` (full), `session`, `rows`, `tree`, `boot_id_before`, `boot_id_after`, `kernel`, `lib`, `started`, `ended`. `boot_id_after` is `unreachable` or `unknown` when the board was lost |
| `chunks.tsv` | header `row label cases status seconds kmsg resets ssh_after boot_same exit_line`; one row per tree row of the plan; `-` where unknown (a chunk never run, a lost capture); `exit_line` is the last `yakogl-mali-exit:` line, whitespace folded |
| `results.tsv` | header `case status detail`; one row per case of the plan: dEQP's status (`Pass`, `Fail`, `NotSupported`, ...) and its Result text, or `Lost` with the reason above |
| `kmsg/<label>.txt` | the kernel lines between the chunk's markers (or `# kernel capture unavailable: ...`) |

The raw logs (every process's `.qpa`, stdout, stderr, and the whole dmesg
capture) go to `RAW_DIR`, default `build/mali-deqp-run/<slice name>/` (ignored
by git). They are **not redacted and never committed**. This is the format
the tally (section 7) reads.

### Before the slice is committed

Write its `README.md`, then check the slice for addresses, host names and
serials, from the repository root:

```sh
python3 -I tools/cts/mali-slice-readme.py --write tools/cts/log/<date>-t6-gles2-S<n>-<sha8>
python3 -I tools/cts/mali-slice-redact.py tools/cts/log/<date>-t6-gles2-S<n>-<sha8>
```

Exit 0 prints nothing and the slice may be committed. Exit 1 prints
`REDACT <file>:<line>: <kind>` for each hit and never the matched text:
redact those lines by hand and rerun until it exits 0. Exit 2: a path cannot
be read. Every file under the slice is scanned, `README.md` included. An
IPv4 address is a hit even in a documentation range (192.0.2.x); only
0.0.0.0, 127.x and version-like quads are not. Board addresses, host names,
UART paths and serials never go in a file or a commit (section 8).

## 5. The self-test: `tools/cts/mali-deqp-selftest.sh`

```sh
tools/cts/mali-deqp-selftest.sh
```

Host only, about 20 s (the hang waits for a 2 s timeout); prints a line per
check and ends `mali-deqp-selftest: 26 checks passed` (run on the tip:
26 passed, 0 failed). A fake `ssh` and `scp` on `PATH` run the board's side in
a directory of this host, and a fake `deqp-gles2` writes its `.qpa` from
`tools/cts/testdata/mali-deqp-run/` (its README says what each fixture is).
The cases: a clean run (16 Pass, 1 Fail, slice format, redaction, the stage
deleted, the lock released); a crash mid-row with the restart; a hang hitting
the timeout; a panthor job-timeout line counted in `resets` and the chunk
`fault`; a missing `loaded` line (`wrong-lib`, exit 9); lost ssh (exit 8, one
probe, no retry); a commit not on the main ref refused before any ssh
(exit 6); `--dry-run` making no ssh or scp call. `MALI_DEQP_SELFTEST_KEEP=1`
keeps the temporary directory.

**The gate row.** On the tip no row of `tools/check.sh`, `cts-selftest.sh` or
the CI runs `mali-deqp-selftest.sh` or `mali-kmsg-selftest.sh`: the wiring is
item `mali-deqp-recipe` of plan #327 (it also adds the READMEs). Until it is
in, a session runs both by hand; a recipe line does not claim a gate row that
is not there.

## 6. Kernel messages: `tools/cts/mali-kmsg.py`

```sh
tools/cts/mali-kmsg.py [--json] [FILE|-]
bash tools/cts/mali-kmsg-selftest.sh
```

Reads a kernel capture (dmesg with or without timestamps, or `dmesg -r`) and
prints one line, for example on a chunk's `kmsg/<label>.txt` of the slice:

```text
kmsg: resets=N timeouts=N mmu_faults=N fw_faults=N gpu_faults=N other_panthor=N first=<line|->
```

`--json` prints the same keys as a JSON object; `-` or no file reads stdin;
exit 2 on a usage or read error. The self-test prints
`mali-kmsg-selftest: 8 ok, 0 failed` (the file is not executable in the tree:
run it with `bash`).

How a chunk's lines are classified (the facts are in
`docs/notes/mali-panthor-kmsg.md`, a reading of Linux v6.18.55 panthor, the
Armbian kernel's version; read its sections 1 to 6 before judging a chunk):

- Only lines naming `panthor` or the device `fb000000.gpu` count; the `<N>`
  prefix and the timestamp are dropped. Info lines (probe, clock rate,
  firmware sha, interface, `using only 1st core group`) are benign and not
  counted. Each other line falls in exactly one class, first match winning:
  - `resets`: the cause lines of a reset (`Soft reset timeout`, `FW ping
    timeout, scheduling a reset`, `FW ... reset failed`, `Fast reset failed`,
    `Failed to boot MCU after reset`). A successful reset logs nothing and
    there is no reset counter in this panthor, so `resets` counts cause lines,
    not resets (note section 1, items 1 and 2).
  - `timeouts`: `job timeout: pid=`, the progress-timer lines, `CSG ... update
    request timedout`, `CSG suspend failed`, `Flush caches timeout`, power
    transition and readiness timeouts. A job timeout kills the group, not the
    GPU (note section 1, item 4).
  - `mmu_faults`: `Unhandled Page fault in AS<n>`, `AS_ACTIVE bit stuck`,
    `failed to unmap range`.
  - `gpu_faults`: `GPU Fault 0x... (` and the protected-mode variant.
  - `fw_faults`: MCU boot or stop failures and firmware image errors.
  - `other_panthor`: every panthor line no pattern matches, e.g. the
    `CS_FATAL` and `CS_FAULT` reports and their exception-type lines.
- `first=` is the first counted line, cut to 160 characters.
- A multi-line message (one `drm_err` with embedded newlines) is counted once,
  on the line that names the device.
- The patterns cover the format strings read from the v6.18 tag; the tail of
  `panthor_sched.c` past `panthor_group_create()` was not read, so a message
  there lands in `other_panthor`. A nonzero `other_panthor` is a reason to
  read the chunk's `kmsg/` file, not to drop it.

The runner's own `resets` column (section 4) is the coarser count: panthor
lines matching `job timeout|reset`, and `fault` lines move the chunk to status
`fault`. The classifier splits them; run it per `kmsg/<label>.txt` for the
tally. A failed reset (`Failed to boot MCU after reset`) makes the render node
unusable until the module is reloaded, which the runner never does: later
chunks then fail to draw and the session stops there for a person (note
section 1, item 5).

## 7. The tally

`tools/cts/mali-deqp-tally.py` (stdlib, run with `python3 -I`) reads the
slices of section 4:

- `--check-cover MUSTPASS SLICE...` exits 0 only when every mustpass case is
  in exactly one slice with a status (`Lost` with one of `chunk-crash`,
  `chunk-hang`, `board-lost`, `not-run`), every `chunks.tsv` row has its
  `kmsg`, `resets`, `ssh_after` and `boot_same` fields, and every slice's
  `commit` is an ancestor of `origin/main` (`--no-git` skips that, for
  fixtures; `--main-ref` names another ref).
- `--tally [--markdown] SLICE...` prints the counts by status and the non-Pass
  counts by cause: the detail with numbers and case names stripped, or
  `refused:<name>` when the detail or the chunk's exit line names a back-end
  refusal. `--markdown` prints the table that goes into
  `docs/es2-cts-failures.md` between `<!-- mali-deqp-tally begin -->` and
  `<!-- mali-deqp-tally end -->`.
- `--check-doc DOC SLICE...` exits 0 only when that block equals the
  `--markdown` output.

Commands and expected output, run on a host with no board, on the synthetic
slices of `tools/cts/testdata/mali-deqp/good/` (two slices, eight cases, a toy
mustpass list; `--no-git` because their commit is not on main):

```sh
python3 -I tools/cts/mali-deqp-tally.py --self-test     # ends: 22 checks passed
python3 -I tools/cts/mali-deqp-tally.py --check-cover tools/cts/testdata/mali-deqp/mustpass.txt \
    --no-git tools/cts/testdata/mali-deqp/good/*         # no output, exit 0
python3 -I tools/cts/mali-deqp-tally.py --tally --no-git tools/cts/testdata/mali-deqp/good/*
```

```text
slices: 2
Pass                   3
Fail                   1
NotSupported           1
InternalError          1
Crash                  1
Lost                   1
TOTAL                  8
cause Crash               1  segfault
cause Fail                1  Image comparison failed: max difference = (#, #, #)
cause InternalError       1  refused:blend_equation_separate
cause Lost                1  chunk-crash
cause NotSupported        1  not supported
```

`--check-doc` prints nothing and exits 0 on `doc-current.md` of that
directory; on `doc-stale.md` it prints `RULE doc-stale: <doc>: the
mali-deqp-tally block differs from --markdown of the slices` and exits 1.
Every failed rule prints `RULE <name>: ...`: `--check-cover` with the real
mustpass list over these slices prints `RULE cover-unknown: <slice>
results.tsv:<n>: <case> is not in the mustpass list` and `RULE cover-missing:
<case> is in no slice` (then `... and N more`).

After the four sessions, from the repository root and without `--no-git`
(`S1` to `S4` stand for the four slice directories
`tools/cts/log/<date>-t6-gles2-S<n>-<sha8>`):

```sh
python3 -I tools/cts/mali-deqp-tally.py --check-cover tools/cts/gles2-mustpass.txt S1 S2 S3 S4
python3 -I tools/cts/mali-deqp-tally.py --tally S1 S2 S3 S4
python3 -I tools/cts/mali-deqp-tally.py --tally --markdown S1 S2 S3 S4   # the block for the doc
python3 -I tools/cts/mali-deqp-tally.py --check-doc docs/es2-cts-failures.md S1 S2 S3 S4
```

The first must print nothing. `docs/es2-cts-failures.md` has no tally block
yet; the first real tally adds it.

`tools/cts/mali-deqp-tally-selftest.sh` runs `--self-test` on the synthetic
slices of `tools/cts/testdata/mali-deqp/` and pins its count of passed checks;
`tools/cts/cts-selftest.sh` runs it as check 40, so the gate's
`cts-selftest.sh` row covers it.

## 8. The board rules

- **Addresses.** `BOARD_HOST` comes from the lease of `target:nanopc-t6`
  (console from `target:nanopc-t6-console`). No file in the tree, no commit
  and no slice holds a host, address, port, UART path, user name or serial.
- **Coordination.** The board is shared with plan #309 (Mali sweep refusals)
  and plan #311 (the Zephyr Z3 host side). Take it only through the lease, and
  never run on a board another plan holds. The runner's board lock and its
  render-node check are the second guard, not a replacement for the lease.
- **Never** load or unload kernel modules (no `modprobe`, `rmmod`, `insmod`),
  touch Tailscale or sshd, touch the protected microSD partitions or the eMMC,
  touch `/boot` or U-Boot. The runner only runs `dmesg`, `fuser`, `df`,
  `uname`, `pkill`/`pgrep` of its own `deqp-gles2`, `mkdir`/`rm` under its
  stage, writes to `/dev/kmsg`, and `deqp-gles2`.
- **Stage only under `/root/yakogl-mali/<sha8>/`**, never `/tmp` (tmpfs).
- **One origin/main commit per chunk set.** Build from an origin/main commit;
  the runner refuses any other (exit 6).
- **A crash, hang or GPU fault must leave the board usable over ssh.** The
  chunk's `ssh_after`, `boot_same` and `kmsg/` file are the evidence; read them
  before the next chunk or session.
- **A board that does not come back needs a person.** Exit 7, 8 or 10 ends
  the run; nothing is retried, nothing is power-cycled from the runner. Report
  it, with the slice, and stop.
- **Redaction.** Raw logs stay in `RAW_DIR`, outside git. Only the redacted
  slice may be quoted in a commit, a record or an issue.
- Board results replace their row in `tools/cts/RESULTS.md` (current results
  only) and the summary in `tools/cts/STATUS.md`; no per-session records.
  Open work and defects are GitHub issues, never a file in the tree.

## 9. Rerunning Lost cases

### When

A `Lost` case (section 4, Lost reasons) is a case the board did not answer.
`--check-cover` (section 7) accepts the four reasons `chunk-crash`,
`chunk-hang`, `board-lost` and `not-run`, but a record that is to stand for a
whole tree must not leave cases in them: criterion c3's cover check wants a
status for every mustpass case, and a Lost case is no result. So after the four
sessions, a rerun is needed when any `results.tsv` holds `Lost`. A `Lost
wrong-lib` chunk is not rerun: the runner stopped (exit 9) because the library
was wrong; fix that first. After exit 7, 8 or 10 the board needs a person
(section 8) before any rerun.

### The table: `tools/cts/mali-rerun-tree.py`

```sh
python3 -I tools/cts/mali-rerun-tree.py [--mustpass FILE] [--max N] \
    [--reason R]... SLICE... > build/rerun-tree.txt
```

`SLICE...` are the committed slice directories
(`tools/cts/log/<date>-t6-gles2-<session>/`). The table holds every Lost case
of them once, in mustpass order, in the form of `mali-gles2-tree.txt`:
`# image T6RR<nnn>: <cases>` and a line of case names. Options:

| option | meaning |
|---|---|
| `--reason R` | repeatable; keep only the Lost cases with that reason (`chunk-crash`, `chunk-hang`, `board-lost`, `not-run`); another value exits 2 |
| `--max N` | at most `N` cases in a row of `board-lost`/`not-run` cases (default 400, the tree check's `--max`); `N` must be a positive number |
| `--mustpass FILE` | the mustpass list the cases must be in (default `tools/cts/gles2-mustpass.txt`) |

A `chunk-crash` or `chunk-hang` case is a row of its own, so it cannot take
neighbours down again; `board-lost` and `not-run` cases fill rows. Labels are
`T6RR001`, `T6RR002`, ... and are never a label of the first-run tree
(`T6S<session>R<row>`), so a rerun chunk shares no kmsg file or case list with
a first-run chunk. A Lost case that is not in the mustpass list exits 1; an
unreadable slice, a bad option or no slice exits 2. Example on the synthetic
slices of section 7:

```sh
python3 -I tools/cts/mali-rerun-tree.py --mustpass tools/cts/testdata/mali-deqp/mustpass.txt \
    tools/cts/testdata/mali-deqp/good/*
```

```text
# The Lost cases of 2 slice(s): 1 case(s) in 1 row(s), mustpass order.
# One case per row for chunk-crash and chunk-hang; at most 400 per row otherwise.
# image T6RR001: 1
dEQP-GLES2.functional.c.eight
```

Read the table before the run: the row count is the `--rows` range, and the
case counts are what the dry run below must show.

### The run

The runner takes the table with `--tree` and runs all its rows under the
`adhoc` session (the default, or `smoke`): it refuses a session other than
`S1`..`S4`, `smoke` and `adhoc`, and `S1`..`S4` name the first-run tree.

```sh
tools/cts/mali-deqp-run.sh --build DIR --out SLICEDIR --rows 1-N \
    --tree build/rerun-tree.txt --session adhoc --dry-run
BOARD_HOST=root@<t6> tools/cts/mali-deqp-run.sh --build DIR --out SLICEDIR \
    --rows 1-N --tree build/rerun-tree.txt --session adhoc
```

`N` is the table's row count. The dry run prints the same plan as in section
4 (the rows are `T6RR<nnn>`), touches no board and writes nothing; run it
first. Everything of sections 4 and 8 applies unchanged: the build comes from
one origin/main commit (the runner refuses another, exit 6), `BOARD_HOST` from
the lease, a crash, hang or fault leaves the board usable over ssh, and exit
7, 8 or 10 stops the run for a person.

### The slice and its commit

`--out SLICEDIR` is written in the format of section 4, with `session=adhoc`
in `meta.txt` and the rerun table's path as `tree`. Commit it under
`tools/cts/log/` named `<YYYY-MM-DD>-t6-gles2-adhoc` (the pattern of
`tools/cts/log/README.md`, with the runner's session name), redacted by the
runner as any slice, with the raw logs left in `RAW_DIR`. The first
slices are not edited. Write its `README.md` with `mali-slice-readme.py
--write` and run `python3 -I tools/cts/mali-slice-redact.py <slice dir>` on
it before the commit, as section 4 says: exit 1 means redact the named lines
by hand and rerun until 0; board addresses, host names, UART paths and
serials never go in a file or a commit.

### The board

One lease on `target:nanopc-t6`, as section 8: a rerun session takes the
board only through it, and never a board another plan (#309, #311) holds. The
runner's board lock and render-node check are the second guard.

### The tally

On main, `tools/cts/mali-deqp-tally.py` does not merge rerun slices yet: `--check-cover` over a first slice and its rerun reports `RULE cover-duplicate` for every rerun case.
