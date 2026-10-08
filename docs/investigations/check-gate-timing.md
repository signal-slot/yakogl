# `tools/check.sh --quick`: where the time goes

Date: 2026-10-09. Tip: 0567683. Host: nuc, 22 logical CPUs (`nproc` = 22),
nightly rustc, kache as rustc wrapper.
Cargo profile: `test`/`dev` = `[profile.dev] opt-level = 1, panic = "abort"`
(cargo prints `[optimized + debuginfo]`), debug assertions on. Product builds
the gate also makes use `[profile.release]` (opt-level 2, **LTO, codegen-units
1**).

## What was measured, and how well

The first gate run was a cold one: this workspace's `target/` was empty. The
host was shared: other ccfarm workers ran their own `check.sh` and cargo builds
at the same time (load average 33-36 on 22 CPUs while the gate ran; 20-25
later). Every ccfarm workspace has its own `target/`, so every worker pays the
cold build, and the workers compete for the same 22 CPUs.

I could not run a per-step-timed copy of the script (executing a modified copy
of `tools/check.sh` was refused in this sandbox, and the script itself must
not change in this item). Cold times below are therefore **bracketed from the
wall-clock moments at which each step's `ok` line appeared** (±1 min), not
read from a stopwatch inside the script. Warm times were measured directly
with `time` / `--report-time` on the same commands (the V3D rows).

The run was green on every step it reached, and was **killed by a worker
restart at about 04:30, after ~80 minutes, before the final summary line**
(it had passed the "no table / no Mesa reference / no test knob in the
artefact" rows, i.e. the aarch64 `--release` LTO product builds, and was in the
later image-link rows). No clippy failure appeared (the `clippy-gate-fix`
rows were green at this tip on this machine); nothing failed for a reason
other than time.

## Per-step table (cold, loaded host; started 03:11)

| # | steps (script order) | done at | elapsed in segment |
|---|---|---|---|
| 1 | `cargo fmt --check`, `build yakogl-capi (no_std)`, `clippy host-test,gpu-powervr` | 03:16:28 | ~5 min |
| 2 | `test host-test,gpu-powervr` (1889 lib tests + integration binaries) | 03:20:06 | ~3.5 min |
| 3 | 2 build-only clippy rows, `clippy` + `test` of the `test-knobs` row (8 tests) | 03:27:54 | ~8 min |
| 4 | `clippy gpu-v3d` + `test gpu-v3d --lib` (2653 tests) | 03:36:52 | ~9 min |
| 5 | V3D `v3d_sim_mesa`, `v3d_cube_noseamless`, `v3d_glmark2_mem`, `clippy ...,report-es2`, `--lib shaderc::online::v3d`, two more integration binaries (report-es2) | ≤03:51 | ~14 min |
| 6 | generated-header checks, sha256, 6 `cc` checks, 8 C check programs, rgxlayer | ~04:11 | ~20 min |
| 7 | python tool self-tests, API coverage, then aarch64 `--release` (LTO) artefact builds ("no table / no Mesa reference / no test knob in the artefact") and image-link rows | past 04:21, killed ~04:30 | >19 min, unfinished |

Steps 1-4 are the nine rows the earlier agent saw green. Rows 4 and 5 are the
"V3D library test step", the one where it ran out of time: by then the gate
had been running ~25 minutes (the agent's budget was reached while rustc was
building the `gpu-v3d` test binary and the later configurations, not because
a test hung). **The gate in `--quick` mode is not quick**: it contains
103 `step` lines (grep count; several sit in loops); `--quick` only trims `CONFIGS` to one entry (and skips the
no-compiler rows), it does not skip the V3D, C, tools or product-image rows.

Warm single-command times (target already built, host load 20-35):

| command | wall |
|---|---|
| `cargo test ... host-test,gpu-powervr` (all binaries) | 33 s (lib 25.6 s) |
| `cargo test ... gpu-v3d,... --lib` (2653 pass, 11 ignored) | 65.6 s |
| `... --test v3d_sim_mesa` | 71 s wall, 3 min CPU |
| `... --test v3d_cube_noseamless` | 0.3 s |
| `... --test v3d_glmark2_mem` | 2.9 s |
| `... ,report-es2 --lib shaderc::online::v3d` (139 pass) | 28.6 s |

So the V3D tests themselves sum to ~3 minutes warm. There is no hang.

## Slowest V3D tests (`--lib`, `-- -Z unstable-options --report-time`)

Raw output: `build/v3d-lib-time.txt` (not committed).

| duration | test |
|---|---|
| 47.2 s | `shaderc::online::v3d::chain::tests::compiled_corpus_words_are_pinned` |
| 47.1 s | `shaderc::online::v3d::chain::tests::corpus_coverage_is_pinned` |
| 16-24 s each | ~145 `v3d::backend::backend_tests::*` (e.g. `a_failed_wait_is_reported` 24.0, `a_refused_draw_leaves_the_data_arena_where_it_stood` 23.7, `a_failed_tfu_job_is_tiled_on_the_cpu` 23.4 ...) |
| 1 s | most other GL-suite tests |

Run alone with `--test-threads=2` at load 25 the two corpus tests took 120 s
each. The wall time of the whole lib run (65.6 s) is about the length of one
corpus test.

## Cause

1. **CPU-bound compiler loops over a big corpus.** Each of the two `chain`
   tests compiles every `real` pair of
   `tests/fixtures/v3d_mesa_programs.txt` (287 vertex+fragment pairs, read
   from `tools/shaders/`) through the whole in-process compiler, in an
   opt-level-1 build with debug assertions. 47 s on a busy host, 120 s when two
   of them share a loaded machine. `tests/v3d_sim_mesa.rs`
   (`our_programs_agree_with_the_ir_and_with_mesa`) is the same kind: it runs
   the QPU interpreter (`src/shaderc/online/v3d/sim.rs`) over the corpus, ~70 s
   wall and 3 min of CPU.
2. **A process-wide lock serialises every compile in the test binary.**
   `store::with_scratch` takes `store::test_lock()` (`#[cfg(test)]`, a global
   mutex; src/shaderc/online/store.rs:580-586, 955). The ~145 `backend_tests`
   use the compiler (their rig builds the clear program,
   backend_tests.rs:1048, 1180) and the whole GL suite compiles shaders, so
   the 16-24 s durations of the backend tests are mostly **time waiting for
   that lock behind the corpus tests**, not their own work. The 2653 tests
   run on 22 threads but the compile-heavy part runs on one.
   `testutil::TestGl` takes a second global (`SERIAL`, testutil.rs:338/1493),
   so every GL-suite test is serialised too.
3. **The real cost is the cold build, not the tests.** Each feature
   combination is a separate rustc compile of the crate (lib, tests, clippy
   check and `--all-targets` are separate artefacts): this tip's `--quick`
   builds about a dozen of them, plus aarch64 `--release` LTO archives, from
   an empty `target/`, on a host that was at load 1.5x its CPU count.
   Cold segments 1-5 took ~40 minutes, the tests inside them ~7 minutes.
4. **Not the cause**: no test hung; `sim.rs` is only slow in the one
   integration test above; no corpus is unexpectedly huge (287 pairs).

## Recommendations

Savings are estimates from the numbers above. "Changes gate" = changes what
the gate verifies.

| # | change | expected saving | changes gate? |
|---|---|---|---|
| R1 | Give workers a **persistent, shared `CARGO_TARGET_DIR`** (or at least a warm copy of `target/`) per feature set; `kache` is already the rustc wrapper; whether it hits across workspaces was not checked. Cold -> warm turns ~40 min of rustc into ~1-2 min | 30-35 min of the ~80 | No |
| R2 | Run `--quick` with a **wall-clock budget printed per step** (needs a change to `step()` in `tools/check.sh`: `SECONDS` before/after, one line) so the next agent can see the time without bracketing it | 0 (diagnostic) | No |
| R3 | `--quick` should mean what it says: keep fmt, capi build, clippy+test of the PowerVR config and the C/tool checks; move the V3D rows, aarch64 release/LTO artefact builds, image-link rows behind the full gate (or `--v3d`, `--artefacts`) | `--quick` falls from ~80 to ~15 min cold (~5 min warm) | **Yes**: items that touch V3D or the product images must run the extra flag |
| R4 | Mark the two `chain` corpus tests and `v3d_sim_mesa::our_programs_agree...` as the "slow" set (`#[ignore = "slow: ..."]` plus one explicit `cargo test -- --ignored` row in the full gate), or run them on every Nth corpus pair in `--quick` | -47 s lib wall warm (and the backend tests drop from ~20 s to ~1 s each because the lock is free), -70 s sim_mesa; ~-2 min warm, up to -8 min cold-loaded | **Yes** in `--quick` only (full gate unchanged) |
| R5 | `[profile.test] opt-level = 2` (or `[profile.dev.package.yakogl] opt-level = 3` only for test runs) | corpus tests ~2-3x faster (47 s -> ~20 s est.), but cold compile of every test binary is slower (+1-3 min each); net loss on cold, gain only on warm. Not recommended unless R1 is done | Debug assertions stay on, overflow checks in `test` are unaffected at opt-level 2; no loss of coverage, but **overflow-check behaviour differs from the dev build** if `overflow-checks` is not set explicitly |
| R6 | Remove the global compile lock in tests by making the scratch arena per-thread (`thread_local` instead of `SCRATCH_BUSY` static) so the 2653 tests really run in parallel; the `test_lock` is only needed because `SCRATCH_BUSY` is shared | the compile-bound lib run: 65 s -> ~15-20 s on 22 CPUs (the two corpus tests still 47 s each, in parallel) | No (same tests), but it is a source change to `store.rs`: a separate item, with the risk that the single-threaded-by-contract assumption (`SCRATCH_BUSY`) is hiding a real aliasing test (`a_compile_inside_a_compile_is_refused_rather_than_aliased`) |
| R7 | Run independent gate rows concurrently (clippy and test of different feature sets use different `target` fingerprints but share the cargo lock; use a separate `CARGO_TARGET_DIR` per row group) | up to 2-3x on a 22-CPU host if it is not already loaded; on a host at load >30 (this run) ~0 | No |
| R8 | Do **not** run more than one gate per machine at a time, or give each worker a `CARGO_BUILD_JOBS`/`-j` share (22 / N) | avoids the 2-3x slow-down seen at load 33-36 | No |

Recommended order for the plan: R1 + R8 (infrastructure, no repo change), then
R2 (one-line diagnostic, in `clippy-gate-fix` or any item that already edits
`check.sh`), then R3 as an owner decision because it changes what an item's
acceptance proves.

## Caveats

- Cold times are bracketed from output arrival times, ±1 min, and depend on
  the load of the day; warm times are measured.
- The 120 s figure for the corpus tests and the 47 s figure differ by load,
  not by test: both are CPU-bound single-thread work.
- The final rows of the gate (image links, the summary line) were not reached;
  their durations are not in this document.
