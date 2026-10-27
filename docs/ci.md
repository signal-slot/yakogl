<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The gate in CI: where the time goes and how it gets short

The rule (2026-10-11): a pull request is merged when the review is OK, the CI
is OK and there is no conflict. A review reads the commit. The CI runs the
gate. So the gate's wall time is the time every change waits. Today that is
36 to 76 minutes per run. The target is a few minutes for most changes and
under 15 minutes for a change to a shared layer. (Where it stands on
2026-10-12, after steps 1 to 4 and 6 and the third runner: a document
change 4 s, a change to one tool or GPU 10 to 25 minutes, a change to a
shared layer 20 to 35 minutes while the test binaries it rebuilds run;
main's full run 32 minutes on runner C, writing the caches.)

## 1. What a run costs today

Measured on 2026-10-11 from the gate logs of main `ea984992` (run
38117870891, runner B) and pull request #48 (run 38120375054, runner A),
141 rows each, both green. The self-hosted runners are named A, B and C in
this file.

| | runner B (12 cores) | runner A (8 cores) |
|---|---|---|
| whole `tools/check.sh` | 4567 s (76 min) | 2150 s (36 min) |
| `test` rows (34) | 3135 s | 1473 s |
| the four `shaderc-online` test rows | 2526 s | 1159 s |
| `gen` rows (16: precompiled-table proofs, Mesa table) | 865 s | 376 s |
| "no Mesa reference in the artefact" | 174 s | 89 s |
| `clippy` rows (26) | 146 s | 69 s |
| Zephyr images (4) | 112 s | 75 s |
| rows that took 0 s (cargo's cache was warm) | 69 | |

Runner B has more cores than runner A and takes twice as long: it also runs up to
five farm agents (three heavy) at the same time (`farm_status`: runner B
`max_concurrent=5`, `heavy=3/3`, load 5 to 7 during the run; runner A
`max_concurrent=1`).

One row measured on this host (64 cores, load 20 to 35), `cargo test
--features host-test,gpu-vivante,shaderc-online`:

| | |
|---|---|
| build, warm cache | 39 s |
| test run, 53 binaries, 2857 tests | 405 s wall, 365 s summed |
| the four slowest binaries | es3_ubo 78 s, stack_survey 78 s, gc_const_map 49 s, online_corpus 48 s |
| three of them at `opt-level = 3` | es3_ubo 38 s, gc_const_map 32 s, online_corpus 35 s (1.4x to 2x) |

27 of the 51 integration test files hold exactly one `#[test]`, so the test
harness has nothing to run in parallel inside them, and `cargo test` runs
the binaries one after another. A row's time is the sum of its binaries'
times, on one core each. The build is not the cost; the test execution is.

After step 2 (a row runs its binaries concurrently, `cargo_test` in
`tools/check.sh`), the same row on this host, warm cache, load 25 to 60,
2026-10-11:

| | before | after step 2 |
|---|---|---|
| `test host-test,gpu-vivante,shaderc-online` (52 binaries) | 405 s (test run) | 63 s (build check and run) |

The binaries' own run-time inputs are part of the cache key, not only
their bytes. Since step 4 that part of the key is the files the tests
open while they run (`TEST_TREE_PATHS` in `tools/check.sh`), measured: the
580 binaries of every test row run under strace opened, inside the
checkout, only `tools/shaders`, `tests/fixtures`, `tests/reference`, a few
files under `tools/cts`, `tools/testdata` and `driver/`, the C sources
`tests/v3d_glmark2_mem.rs` compiles, and the generated glmark2 corpus;
the library's unit tests also read `src/` and `tests/`
(`src/test_rules.rs`). Before step 4 it was every tracked file except the
documents and the CI files.

After step 3 (`[profile.test] opt-level = 3`), the three slow binaries,
the same binaries back to back (two runs per arm), load 2 to 8:

| | opt-level 1 | opt-level 3 |
|---|---|---|
| es3_ubo | 50.4 s, 49.4 s | 30.7 s, 30.7 s |
| gc_const_map | 46.5 s, 45.9 s | 29.5 s, 29.5 s |
| online_corpus | 46.1 s, 46.2 s | 29.5 s, 29.5 s |

The whole `tools/check.sh` after steps 2 and 3, on this host, from an
uncommitted tree on top of `df934dfd` (a change to `check.sh` and
`Cargo.toml`, so every row, 142), load 2 at the start:

| | first run (records the cache) | second run (reads it) |
|---|---|---|
| whole run | 2377 s | 677 s |
| test binaries | 492 run | 0 run, 492 skipped |
| the 34 `test` rows | 1077 s | 25 s |
| slowest rows | Mesa table 202 s, `test` PowerVR tier 184 s, `test` PowerVR online 178 s | Mesa table 222 s, GC7000 table proof 174 s, PowerVR table proof 54 s |

A PowerVR online row now takes as long as its slowest binary,
`tests/usc_limits.rs` (130 to 135 s, one test,
`deqp_test_programs_compute_what_the_ir_means_in_each_if_lowering`).
The second run's remaining time is the non-cargo rows (step 4). Two
rows were red in both runs for a reason outside this change: the AM62P
gl_test images, whose generated linker script includes Zephyr's
`swi_tables.ld` by a relative path that does not resolve from a build
directory under `/tmp`.

With cargo-nextest 0.9.148 (built into a scratch directory here; this
host is not a runner) on `376e3a64`, which is main `5ad2fa0e` with these
changes merged, every row, 142:

| | first run (records) | second run (reads) |
|---|---|---|
| summary | 140 ok, 2 failed | 140 ok, 2 failed |
| whole run | 3100 s | 615 s |
| test binaries | 492 run | 0 run, 492 skipped |
| the 34 `test` rows | 1065 s | 20 s |
| slowest rows | "no Mesa reference in the artefact" 796 s, Mesa table 197 s, `test` PowerVR online 161 s | Mesa table 196 s, GC7000 table proof 154 s, PowerVR table proof 53 s |

The first run followed a change to `src/`, so every build was redone; an
earlier pair on `7020a32c` without a source change took 1325 s (test rows
545 s) and 755 s. The 796 s artefact row is not explained yet (99 to 154 s
in the other runs; a rebuild of the release archives is the guess, not
measured). The two red rows are the AM62P images (above). nextest found
one test that passed only when another test of the same process ran
first (`gl::shader::tests::a_refused_shader_online_logs_one_summary_line_and_no_source_chunks`,
fixed in `376e3a64`).

After step 4 (rows through a pool, input keys for the slow rows outside
cargo, the narrower test key), on this host restricted to 8 cores
(`taskset -c 0-7`, runner A's count), `tools/check.sh --jobs N --changed
origin/work/ci-fast` on `fc1bb2a6` (a change to `check.sh`, so every row,
143), 2026-10-12, every run green. This host has kache, a compiler cache,
in front of rustc and cc (`~/.cargo/config.toml`), so a build here is
faster than on a runner; cargo-nextest is not installed here, so the test
rows ran their binaries themselves.

| run | cache | whole run | test binaries | keyed rows |
|---|---|---|---|---|
| `--jobs 1` (target already built) | empty | 1174 s | 492 run | 11 run |
| `--jobs 2` | empty | 877 s | 492 run | 11 run |
| `--jobs 4` | empty | 593 s, 547 s (two runs) | 492 run | 11 run |
| `--jobs 4` | recorded by the run before, same commit | 87 s | 492 skipped | 11 not run |

The rows with an input key, before (`--jobs 1`, empty cache: the run
that recorded, and a second one) and after (read from the record): the
Mesa table 189 s and 195 s, the GC7000 table proof 156 s and 153 s, the
PowerVR one 57 s and 56 s, their ES 2.0 builds 37 s/38 s and 4 s/4 s,
"no Mesa reference in the artefact" 106 s and 8 s (the second run found
every archive built, with kache), the no-table and no-knob rows 0 to 2 s,
and three Python checks (16 s, 14 s, 3 s): each 0 s after, the log
printing `skip  <row> (passed on main fc1bb2a69)`. The first
recorded run did not let the next one skip the test binaries: their key
hashed the glmark2 corpus as it was before the run, which the run's own
glmark2 row then regenerated from a newer tree (the same bytes on every
regeneration since, checked); from the second recording on they are
skipped (the last row of the table).

"No Mesa reference in the artefact" builds 26 release archives, one core
each (`lto = true`, `codegen-units = 1`), and cargo builds one at a time in
one target directory: 731 s on runner A in the CI run of `5503e1c4`
(38148174637), 796 s once here. Since step 4 it builds them in 4 target
directories of its own at once. The row alone, cold (fresh target
directories, no compiler cache, 8 cores), the row's functions from
`fc1bb2a6` and from this change: 782 s before, 224 s after, the same
counts (controls 4800 and 1224 symbols, 24 products with 0) and the same
2.4 GB of target directories.

Step 6 (the slow one-test binaries), on this host, 2026-10-12, the same
binary built before and after each commit, warm build, load 5 to 35
from other agents (the load moves; each pair ran back to back):

| binary (feature set) | before | after | commit |
|---|---|---|---|
| usc_limits (PowerVR online), cargo test | 177.8, 150.6, 174.4 s | 101.0, 101.1, 102.5 s | `3351f98f` |
| usc_limits (PowerVR online), nextest | 89.6, 88.2 s | 48.7, 48.6, 48.8 s | `3351f98f` |
| stack_survey (GC7000 online), cargo test | 62.5, 57.6 s | 44.2, 36.0 s | `8ccc05ff` |
| stack_survey (PowerVR online), cargo test | 20.3, 17.2 s | 8.1, 8.1 s | `8ccc05ff` |
| es3_ubo (GC7000 online), cargo test | 33.5, 32.0 s | 31.9, 32.0 s | `c95a284c` |
| es3_ubo (GC7000 online), nextest | 20.3, 20.4 s | 5.4, 5.5, 5.4 s | `c95a284c` |

`usc_limits` was not short of threads: its survey decoded the same
groups again on every pass of a loop (`3351f98f` caches the decode).
Every test of `usc_limits` and `es3_ubo` holds one process-wide lock (the
compiler's scratch arena, the GL context), so under cargo test a split
runs no faster; under nextest each test is its own process.
`online_corpus` and `gc_const_map` (GC7000) are not split: each runs in
about 0.1 s on the 324 sources of `tools/shaders` (measured 2026-10-24).
They took about 33 s and 31 s while the Qt Quick 3D sources were in the
corpus (until #1509), most of it the GC7000 selection's dead-code pass on
their largest material shaders.

`tools/check.sh --changed` (the scope map) does not help a shared-layer
change: `src/gl/draw.rs`, `src/shaderc/online/*` and `tools/check.sh`
select all 134 rows ("scopes: all"); `src/vivante/*` selects 38; a document
selects 2.

## 2. The four causes

1. **Serial test execution.** One core runs a 78 s corpus test while the
   other cores idle. The four `shaderc-online` rows alone are 55% of a run.
2. **The same work repeated.** Every run proves the precompiled tables
   deterministic (four rows, made twice and verified row by row), reproduces
   the Mesa table with `pco_shc`, builds the product archives for a symbol
   count, and runs every test binary, whether or not any input of that work
   changed since the last green run.
3. **One machine, one job, rows in sequence.** A run is one job on one
   runner. The two runners are the farm's worker machines, and the farm's
   agents compete for the same cores.
4. **Tests at `opt-level = 1`.** The dev profile is right for an edit loop
   and halves the speed of a compiler-heavy test.

## 3. The design

Three mechanisms, in the order they apply to a change. Each is exact: it
skips nothing a change can reach.

### 3.1 Run only what the change reaches (exists)

`tools/check.sh --changed <base>` selects rows by the scope map. On a pull
request the gate runs `--changed <merge-base with main>`; nightly on main's
tip it runs every row (not after each merge, and not again on a commit
that already passed one: the owner, 2026-10-25). A document or a one-GPU
change is already small this way. A shared-layer change is not, and the
next two mechanisms carry it.

### 3.2 Never redo work whose inputs did not change (new)

**Test binaries.** cargo already rebuilds only what changed. The gate
extends that to running: a test binary is run only when its bytes are new.
`cargo test --no-run --message-format=json` names the executables; the
gate hashes each one (sha256 of the file) together with the row's
environment key (the toolchain, the dependency layout's tool ids from
`tools/cts/deps-env.sh --check`); a hash that passed on a green run of
main is a pass, recorded as `$CARGO_TARGET_DIR/../test-ok/<hash>`; any
other hash runs. This is exact dependency tracking for free: a change to
`src/gl/draw.rs` relinks every yakogl test binary, so every one runs; a
change to `tests/es3_ubo.rs` changes one binary, so one runs; a change to
`tools/` or `docs/` changes none. The record is written only by a run on
main (never by a pull request's run), so a pull request can only reuse
passes of what is already on main.

**Non-cargo rows** with a slow run get an input key (`rowkey` in
`check.sh`, step 4): the files the row reads (git blob ids, and the
content of what is changed in the working tree), the bytes of the programs
it runs and of what it builds and reads (with every shared library they
load: glslangValidator is a front end to libglslang.so), the toolchain,
the environment, the row's command and the text of its shell functions.
Each input set was measured with strace, not only read from the code. The
keyed rows: the four table proofs, the Mesa table, the three release
archive rows, and three Python checks. The archive rows also check cargo's
dep-info against their key on every build, so an input added outside the
key fails the row instead of being missed. The record is
`$TEST_OK_DIR/rows/<key>` (next to the test binaries'), written only on
main; a hit prints `skip  <row> (passed on main <commit>)` and the output
the row printed on main, and the summary counts it as "not run (passed on
main)". A row whose inputs cannot all be named has no key and its comment
says why: the Zephyr images (the west workspace and the SDK are outside
the checkout), common-tally (it runs board.sh, which runs cargo and git
over the checkout), the board.sh self-test (not traced). The other rows
take about a second.

Both caches live on the runner (`~/.cache/yakogl/`), next to the cargo
target directory that already persists between runs. A cache entry is a
claim about identical bytes, so there is no staleness; a wrong entry can
only come from a wrong key, and the key is printed next to each skipped
row (`skip   test ... (passed on main a1b2c3d4, hash 9f...)`), so a skip is
visible and attributable.

### 3.3 Run the rest in parallel (new)

**Inside a row.** The test binaries of one `cargo test` row run
concurrently: the gate runs the executables itself (from the same
`--message-format=json` list), N at a time (N = cores), each binary's own
harness threads as today. A row's wall time becomes its longest binary
(78 s) instead of the sum (365 s). On a host with `cargo-nextest`
(0.9.148 on runner A and runner B, in `~/.cargo/bin`), a row runs its
binaries through `cargo nextest run` instead, which runs every test as
its own process, so a binary's tests spread over the cores too. A host
without it (a developer's machine) uses the gate's own runner. The hash
cache comes first in both cases, and the runner is part of the key. The
rows that read libtest's output (the knob tests by name, the V3D count,
the PowerVR pool) always use the gate's own runner. Measured once each
on this host, no cache: the GC7000 online row 44 s (own runner) and
52 s (nextest); the PowerVR online row 136 s and 97 s, because nextest
splits `tests/usc_limits.rs` over processes.

**Across rows.** `check.sh --jobs N` (`YAKOGL_CHECK_JOBS`, step 4) runs up
to N rows at once, each in its own process with its output in files, and
prints them in the script's order, so the log reads the same with any N
and a red row's output stays under its FAIL line. cargo serializes builds
on its directory lock; what the pool must order is a path two rows leave
different, so a row that shares one names it (`uses w:NAME` / `r:NAME`):
an uplifted binary or archive rebuilt per feature set, a generated table
another row reads, a scratch directory, and the checkout itself for the
rows that edit it. The list is in `check.sh`; one entry was found by the
strace survey (yakogl-precompile's tests write fixed paths under
`build/test-tmp`). The CI runs 4 jobs (`gate.yml`): on 8 cores, 4 jobs
took half the time of 1 (section 1); more was not measured.

**Compiler slots.** Every compiler the rows start comes out of one GNU make
jobserver that `check.sh` makes at its start (a FIFO in a directory of its
own under TMPDIR; cargo and the cc crate are its clients), sized by memory: one
slot per 3072 MB of `MemAvailable` (`YAKOGL_CHECK_MEM_PER_COMPILER_MB`),
never more than the cores. A pool row holds a slot for its whole run, so
the rows at once can never run more compilers than the slots. Without it
each cargo starts one compiler per core: on 2026-10-13 runner C (16 threads,
16 GB, no swap) ran about ten clippy-drivers of 1 to 1.6 GB under four
rows, ran out of memory and hung for two hours. One clippy-driver on the
yakogl crate peaks at 2.9 GB (2026-10-14). The run prints its slots on its
first line.

The FIFO is not under the checkout. On 2026-10-18 (run 39062794102) seven
V3D test rows were red with rustc's "failed to connect to jobserver" for a
`build/check/tests-<pid>` path of a pid that was not the run's. Observed: a
`check.sh` of a cancelled run still alive, its `tests-<pid>` directory gone,
and the warning stored by cargo in the shared target directory (replayed to
every build of a fresh unit). Inferred, not observed: the next job's
`git clean -ffdx` removed that orphan's FIFO. The fix: the FIFO lives in its
own directory under TMPDIR (locked for the run's lifetime, swept by the next
run only when the lock is free), and the outermost run removes the stored
warnings before its rows; a build that still cannot open its jobserver
stays a red row.

**Across machines.** When one machine is not enough, the workflow splits a
run into shards (one job per scope class: `pvr`, `vivante`, `v3d`,
`precompile`, `tools`, `images`), each a `check.sh --shard <class>` on its
own runner, and a last job joins the summaries. More runner capacity is a
matter of registering more runner instances: a machine can run several
(one per directory).

**Beside the farm.** ccfarm runs its agents at nice 10 / CPUWeight 20
by default, and the runner units will be set below that. So a gate run
on a busy machine is slower by design, and the gate makes up for it by
doing less and running in parallel.

**The runners (2026-10-14).** `gate.yml`'s `runs-on` names the label
`yakogl-gate`, which runners A, B and C all carry, so a run takes
whichever is free. From 2026-10-12 to 10-14 the gate ran on runner C alone
(label `gate-primary`), to keep it away from farm agents; on 2026-10-13
runner C hung out of memory (section "Compiler slots") and every pull
request queued behind it for hours. A slower runner is still a runner
(user, 2026-10-14): the compiler slots keep a run inside the host's
memory, and the farm keeps priority through CPU weight (above). GitHub
has no priority among runners, so the faster one is not preferred.

Runner C is a mini PC (Ryzen 9 5900HX, 8 cores / 16 threads, 16 GB,
Gentoo, no swap). Measured there on 2026-10-12, every row: 2009 s cold on
its first run (five rows red for the host, since fixed: the table proofs
needed the library's `.rmeta`, #222, and a glmark2 tree copied with two
directories missing) and 1900 s green on main `bf6c98ca6` (run
38248051277, 144 ok, the cache-writing run); runner A's run of the same main
took 2261 s. A document pull request's gate took 4 s there (#225).

### 3.4 Make the slow tests faster

- `[profile.test] opt-level = 3` (inherits dev otherwise): about 2x on the
  compiler-heavy tests, measured above. Debug assertions and overflow
  checks stay on. The dev profile for the edit loop is unchanged.
- The one-test corpus binaries (`es3_ubo`, `stack_survey`, `gc_const_map`,
  `online_corpus`, ...) split their corpus into several `#[test]`s (by
  dEQP group, by shader directory), so the harness runs them on several
  cores. Mechanism 3.3 already removes most of their cost; this removes the
  rest and helps the edit loop too.
- The precompiled-table proofs run their four configurations in parallel
  (3.3) and skip when the list and the compiler did not change (3.2).

## 4. What a pull request waits for after this

Estimates from the runner A numbers (8 cores, no farm load), with 3.1 to 3.4 in
place; to be replaced by measured runs as each step lands.

| change | rows | today (runner A) | after |
|---|---|---|---|
| a document, a record | 2 | 36 min (every row ran) | under 1 min |
| one GPU backend (`src/vivante/*`) | 38 | 36 min | 3 to 5 min |
| a shared layer (`src/gl/*`, the online compiler) | 134 | 36 min | 10 to 15 min on one 8-core runner; 5 to 8 min sharded over two |
| a test file | its binary | 36 min | 1 to 2 min |

The nightly full run on main stays the proof of the whole; it is also the
run that writes the caches, so the next day's pull requests start from it.

## 5. Steps, in order

1. `gate.yml`: a pull request runs `tools/check.sh --changed <merge-base>`;
   the nightly run on main runs every row. (No decision needed.)
2. `check.sh`: test binaries run concurrently inside a row, and a binary
   whose hash passed on main is skipped (3.2, 3.3). The gate's summary
   counts skipped-by-cache rows separately from the `--changed` skips.
3. `Cargo.toml`: `[profile.test] opt-level = 3`, with a measurement of the
   four slow binaries before and after in the commit message.
4. Done: `check.sh` runs rows through a pool (`--jobs`, `gate.yml` sets
   4), the slow rows outside cargo have input keys, the test binaries'
   tree key holds only what the tests read, and the artefact row builds
   its archives in four directories at once. Measured in section 1: a full
   run on 8 cores 1174 s with one job, 547 to 593 s with four, 87 s when
   the same commit's record exists.
5. `gate.yml`: shards, one job per scope class, when a second runner
   instance exists. Needs the owner: runner instances (which machines, how
   many), and the farm's slots on them.
6. Split the one-test corpus binaries, one at a time, each with its before
   and after time.

Every step is measured on the same commit on runner A before and after (the row
times in the gate log) and the numbers go into this file's section 1.
