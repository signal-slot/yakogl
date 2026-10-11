<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The gate in CI: where the time goes and how it gets short

The rule (2026-10-11): a pull request is merged when the review is OK, the CI
is OK and there is no conflict. A review reads the commit. The CI runs the
gate. So the gate's wall time is the time every change waits. Today that is
36 to 76 minutes per run. The target is a few minutes for most changes and
under 15 minutes for a change to a shared layer.

## 1. What a run costs today

Measured on 2026-10-11 from the gate logs of main `ea984992` (run
38117870891, runner system76) and pull request #48 (run 38120375054, runner
nuc), 141 rows each, both green.

| | system76 (12 cores) | nuc (8 cores) |
|---|---|---|
| whole `tools/check.sh` | 4567 s (76 min) | 2150 s (36 min) |
| `test` rows (34) | 3135 s | 1473 s |
| the four `shaderc-online` test rows | 2526 s | 1159 s |
| `gen` rows (16: precompiled-table proofs, Mesa table) | 865 s | 376 s |
| "no Mesa reference in the artefact" | 174 s | 89 s |
| `clippy` rows (26) | 146 s | 69 s |
| Zephyr images (4) | 112 s | 75 s |
| rows that took 0 s (cargo's cache was warm) | 69 | |

system76 has more cores than nuc and takes twice as long: it also runs up to
five farm agents (three heavy) at the same time (`farm_status`: system76
`max_concurrent=5`, `heavy=3/3`, load 5 to 7 during the run; nuc
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
their bytes: the tests read `tools/shaders`, `tests/fixtures`,
`tests/reference` and headers at run time, and `src/test_rules.rs` reads
`src/` and `tests/`. So the key also hashes every tracked file except the
documents, the CI files and `check.sh` with its self-tests. For every
binary but the library's own unit tests it also leaves out `tests/*.rs`.

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
request the gate runs `--changed <merge-base with main>`; on main after a
merge and nightly it runs every row. A document or a one-GPU change is
already small this way. A shared-layer change is not, and the next two
mechanisms carry it.

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

**Non-cargo rows** (the precompiled-table proofs, the Mesa table
reproduction, the Zephyr images, the generated-file reproductions, the
artefact symbol counts) get an input key: the git tree hash of the paths the
row reads plus the ids of the tools it runs. The row's `step` function
looks the key up before running and records it after a green run on main.
`check.sh` already names a row's scope; the key is the same idea with exact
inputs instead of a class. A row without a declared input set has no key
and always runs; the gate lists those rows in its summary, so the list
shrinks on purpose, never by accident.

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
(78 s) instead of the sum (365 s). `cargo nextest` does the same per test
and would be the better tool once it is installed on the gate hosts
(`cargo install cargo-nextest --locked`; a host package, so the owner's
decision); the gate's own runner needs nothing installed.

**Across rows.** Rows of one scope class are independent processes on one
target directory. cargo serializes the builds on its directory lock, which
is short with a warm cache, and the test runs then overlap. `check.sh` runs
rows through a pool (`--jobs N`), keeping the log per row and the summary
line as today; a red row is reported the same way.

**Across machines.** When one machine is not enough, the workflow splits a
run into shards (one job per scope class: `pvr`, `vivante`, `v3d`,
`precompile`, `tools`, `images`), each a `check.sh --shard <class>` on its
own runner, and a last job joins the summaries. More runner capacity is a
matter of registering more runner instances: a machine can run several
(one per directory), and this host (64 cores) would be the strongest of
them. Whether the owner's workstation becomes a runner is the owner's
decision.

**Away from the farm.** The two runners are farm workers. A gate job on
system76 shares its cores with up to five agents; that alone doubled the
run. Either the farm's slots on runner machines go down while a gate runs
(ccfarm knows a node's load; a runner job could pause the worker's claims
through `ccfarm node pause` for its duration), or the gate gets machines
the farm does not use. Decision for the owner.

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

Estimates from the nuc numbers (8 cores, no farm load), with 3.1 to 3.4 in
place; to be replaced by measured runs as each step lands.

| change | rows | today (nuc) | after |
|---|---|---|---|
| a document, a record | 2 | 36 min (every row ran) | under 1 min |
| one GPU backend (`src/vivante/*`) | 38 | 36 min | 3 to 5 min |
| a shared layer (`src/gl/*`, the online compiler) | 134 | 36 min | 10 to 15 min on one 8-core runner; 5 to 8 min sharded over two |
| a test file | its binary | 36 min | 1 to 2 min |

The full run on main after a merge stays the proof of the whole; it is also
the run that writes the caches, so the next pull request starts from it.

## 5. Steps, in order

1. `gate.yml`: a pull request runs `tools/check.sh --changed <merge-base>`;
   main and nightly run every row. (No decision needed.)
2. `check.sh`: test binaries run concurrently inside a row, and a binary
   whose hash passed on main is skipped (3.2, 3.3). The gate's summary
   counts skipped-by-cache rows separately from the `--changed` skips.
3. `Cargo.toml`: `[profile.test] opt-level = 3`, with a measurement of the
   four slow binaries before and after in the commit message.
4. `check.sh`: rows run through a pool (`--jobs`), and the non-cargo rows
   get input keys.
5. `gate.yml`: shards, one job per scope class, when a second runner
   instance exists. Needs the owner: runner instances (which machines, how
   many), and the farm's slots on them.
6. Split the one-test corpus binaries, one at a time, each with its before
   and after time.

Every step is measured on the same commit on nuc before and after (the row
times in the gate log) and the numbers go into this file's section 1.
