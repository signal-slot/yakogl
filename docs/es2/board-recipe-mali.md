<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 (Mali-G610): the dEQP-GLES2 recipe, library, runner and kernel messages

Plan #327 (ROADMAP.md stage 5, issues #7 and #20). The NanoPC-T6 (RK3588, Arm
Mali-G610, back end `gpu-mali`) runs yakogl's Mali back end on its own Armbian
through the panthor kernel driver. **Nothing in this recipe has run dEQP on the
board yet.** Only `gl_test` sweeps are recorded (`tools/cts/RESULTS.md`, "Mali").
Every "expected" line below is the form the tools print, taken from their
sources and from a run of their self-tests on a host, not a board result.

This is the first part: the pieces already on main (the library, the runner,
the kernel message classifier, the board rules). Three sections are marked
**to be completed**; their tools are not merged yet and their options are
deliberately not described here.

| piece | state | what |
|---|---|---|
| `tools/mali/panthor-gles/` | on main | `libGLESv2.so.2` and `libEGL.so.1` over panthor (section 1) |
| `tools/cts/mali-deqp-build.sh` | to be completed | cross-builds `deqp-gles2` for the board's Armbian (section 2) |
| `tools/cts/mali-gles2-tree.txt` | to be completed | the tree table: the mustpass list in rows (section 3) |
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

## 2. The deqp-gles2 build: to be completed

`tools/cts/mali-deqp-build.sh` (item `mali-deqp-build`, not merged) makes the
build directory the runner reads. What the runner requires of that directory
is in section 4; the build's options, its time and its expected output are
added when the script is on main.

## 3. The tree table: to be completed

`tools/cts/mali-gles2-tree.txt` (item `mali-gles2-tree`, not merged) splits
the 17,165-case `tools/cts/gles2-mustpass.txt` into rows, one chunk each, in
the form of `check-gles3-tree.py`'s reader (`# image <LABEL>: <count>` then a
`dEQP-GLES2....*` filter line; labels `[A-Za-z0-9]+`). The real rows, their
counts and the four sessions S1..S4 are added when it is on main. The runner
and its self-test use a three-row table of the same form,
`tools/cts/testdata/mali-deqp-run/tree.txt`.

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
`KMSG_SETTLE_S` (1), `STAGE_MARGIN_KB` (1 GiB), `DEQP_SURFACE` (64),
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
  cd /root/yakogl-mali/009f62a5 && setsid -w timeout -k 10 <limit> env LD_LIBRARY_PATH=/root/yakogl-mali/009f62a5/lib /root/yakogl-mali/009f62a5/deqp-gles2 --deqp-caselist-file=/root/yakogl-mali/009f62a5/<label>.r<k>.txt --deqp-log-filename=/root/yakogl-mali/009f62a5/<label>.r<k>.qpa --deqp-archive-dir=/root/yakogl-mali/009f62a5/data --deqp-surface-type=pbuffer --deqp-surface-width=64 --deqp-surface-height=64 --deqp-log-images=disable --deqp-log-shader-sources=enable > /root/yakogl-mali/009f62a5/<label>.r<k>.out 2> /root/yakogl-mali/009f62a5/<label>.r<k>.err
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
