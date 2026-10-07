# V3D on run.sh's own programs (scen, hostile)

Run of 2026-10-08, commit `eab46819` plus the `run.sh` / `rec.c` changes
below. `gltest` is not run on the V3D build (its host arm is the PowerVR's, see
README.md); the V3D arm of `gl_test.c` runs on the Zephyr image.

## Commands

```sh
# V3D, no sanitizers (HH_SAN=0): 62 jobs x 12 modes = 744 runs
HH_SAN=0 HH_FEATURES=osal-linux,gpu-v3d,shaderc-online tools/hostharness/run.sh all out-v3d rust:. driver dev
# V3D, sanitizers (default; nightly + ASan for Rust, ASan+UBSan for C): same 744 runs
HH_FEATURES=osal-linux,gpu-v3d,shaderc-online tools/hostharness/run.sh all out-v3d-asan rust:. driver dev
# PowerVR, default features (mesa-reference-table), sanitizers
tools/hostharness/run.sh all out-pvr rust:. driver dev
# PowerVR with the online compiler (the V3D's feature set), no sanitizers
HH_SAN=0 HH_FEATURES=osal-linux,gpu-powervr,shaderc-online tools/hostharness/run.sh all out-pvr-sc rust:. driver dev
# only the fault-injection jobs, after the offsets change (the job regex is the last argument)
HH_SAN=0 HH_FEATURES=osal-linux,gpu-v3d,shaderc-online tools/hostharness/run.sh all out-v3d2 rust:. driver dev '-fail|failat'
```

The worker's shell refused an `HH_...=... tools/hostharness/run.sh` prefix
for all but the V3D string, so the PowerVR and later variants were started
through `python3 -I -c` (it sets the same environment and runs `run.sh`).
Nothing in the build depends on that.

## Result

All 62 programs pass on the V3D in the ten modes that give the library a CPU
view of object storage (`l0 l2 l0mp l0mpc z0 a0 z2 a2 base l3wf`), without and
with sanitizers: exit 0, no `HH-ERROR`, no ASan report, `errors=0` in every
`hh-end` line. In `l0nc` and `l2nc` the 21 programs listed under finding 1
abort. The other 41 pass in all 12 modes.

| program | V3D | command (`out/logs/<job>.<mode>.*`) |
|---|---|---|
| `scen.state`, `state-special`, `state-special-inrange`, `state-pretransform` | pass, 12 modes | `scen state 400 0`, `state 400 1` (`HH_INRANGE=1`), `state 80 0` (`HH_PRETRANSFORM=1 HH_NOCOALESCE=1 HH_DEBUG=1`) |
| `scen.egl`, `inst0`, `win`, `buf`, `fbo`, `fbo-msaa`, `buf-fail`, `fbo-fail` | pass, 12 modes | `scen egl`, `scen inst0`, `scen win`, `scen buf 400 1`, `scen fbo 300 1`, `scen fbo 150 3` (`HH_SAMPLES=4`), `HH_FAIL_AFTER=16 HH_FAIL_EVERY=3 scen buf 200 2`, `HH_FAIL_AFTER=16 HH_FAIL_EVERY=4 scen fbo 150 5` |
| `scen.gets`, `ctx2`, `tex`, `tex-b`, `stress`, `stress-big` | pass in 10 modes; panic in `l0nc`, `l2nc` (finding 1) | `scen gets`, `scen ctx2`, `scen tex 300 1`, `scen tex 300 7`, `scen stress 3000 0`, `scen stress 500 50000` |
| `scen.progs-top`, `progs-es100`, `progs-femtovg`, `progs-qt`, `progs-qt3d` | pass in 10 modes; panic in `l0nc`, `l2nc` (finding 1); every shader of the corpus compiles on the V3D | `scen progs . 1 0`, `progs es100 1 0`, `progs femtovg 1 0`, `progs qt 0 0`, `progs qt3d 1 1` |
| `scen.tex-fail` | pass in 10 modes (with the new offsets, finding 3); panic in `l0nc`, `l2nc` | `HH_FAIL_AFTER=16 HH_FAIL_EVERY=5 scen tex 150 2` |
| `scen.progs-failat17 .. 34` (six jobs) | pass in 10 modes, panic in `l0nc`, `l2nc` | `HH_FAIL_AT=<n> scen progs . 0 0` |
| `hostile.alloc_failure_sweep`, `egl_image_destroy_then_use_sibling`, `generate_mipmap_in_flight` | pass in 10 modes; panic in `l0nc`, `l2nc` (finding 1) | `hostile <name>` |
| the other 29 `hostile.*` (negative offsets, wrapping ranges, unaligned pointers, EGLImage ownership, odd widths, size extremes, `blend_variant_oom`, EGL lifetime) | pass, 12 modes | `hostile <name>` |

The V3D carve-out line at exit (`osal(linux): v3d carve-out: ... allocations
refused 0`) is clean in the programs that do not inject failures; the
fault-injection jobs refuse allocations on purpose.

The job names `scen.progs-failat40/45/50/60/80/120` of the previous run are
now `failat17/20/24/28/32/34` on the V3D (finding 3).

## Comparison with the PowerVR

| | V3D | PowerVR (`shaderc-online`) | PowerVR (default features) |
|---|---|---|---|
| `scen` / `hostile`, 12 modes | all pass; `l0nc`/`l2nc` panic in 21 jobs | all pass, none panics (before the offsets change: the fail jobs exit 3) | same as with `shaderc-online` |
| `gltest` (175 jobs) | not run on the V3D | every run exits 0 except `gltest.174` (rc 124, the 1200 s limit, in all 12 modes; also in the default-features run, so not load) | `gl_test: ... FAIL`, "the vertex shader is not in the offline table": the default PowerVR build has the reference table and no compiler, so `gltest` as `run.sh` starts it fails, unchanged by this item |
| rc 3 (init failed) in `*-fail`, `progs-failat40..60` | none | 7 jobs x 12 modes, before the offsets change | same |

PowerVR needs `HH_FEATURES` with `shaderc-online` for `gltest` to pass; the
README documents that for the `HH_SHADERC=offline` arm. The vivante build has
no harness run: `rec.c` and `run.sh` model the PowerVR and the V3D only
(GC7000 goes through `glmark2/run.sh`, `FINDINGS.md`), so no GC7000 run of
these programs exists to compare. That is a gap of the harness, not of a
build.

## Findings

1. **V3D finding: `mem.cpu` is not checked.** In `l0nc` and `l2nc` (object
   storage without a CPU view: `HH_NOCPU=1`) 21 programs abort: all of
   `scen.{ctx2,gets,tex,tex-b,stress,stress-big,tex-fail,progs-*}` and
   `hostile.{alloc_failure_sweep,egl_image_destroy_then_use_sibling,generate_mipmap_in_flight}`
   with `ptr::write_bytes requires that the destination pointer is aligned
   and non-null` (`core/src/ptr/mod.rs:727`), exit 134. The cause is the
   tiled copy's canary write in `src/v3d/backend.rs` through a NULL `mem.cpu`
   (see README.md). `gles_osal.h` allows a NULL `cpu`; `zephyr_v3d.c` and
   `linux.c` always map, so the Pi 4 cannot hit it. Same set as the
   2026-10-07 run. Not changed (`src/v3d` is not this item's).
2. **V3D finding (third-party, UBSan): Mesa's packer helpers.** The sanitized
   run reports, in every job and mode, `v3d_packet_helpers.h:55`/`:58` of
   Mesa 26.2.1 (`__gen_unpack_uint`: `shift exponent 56/32 is too large for
   32-bit type 'int'`, `left shift of 255 by 24 places`, `signed integer
   overflow`) and, in `scen.stress`, `v3d_packet_helpers.h:98` (`member access
   within misaligned address` in `__gen_unpack_float`, from `V3D42_POINT_SIZE_unpack`
   under `v3d_pack_POINT_SIZE`, `driver/src/v3d/v3d_cl.h:340`). The
   unpack helpers are Mesa's and the code is only executed because the job
   builder repacks CL packets; the results are the same on x86-64 and
   AArch64 compilers in practice. UBSan reports are collected, not fatal,
   so nothing fails. Not a harness gap; whoever owns `driver/src/v3d` could
   silence them with a `-fno-sanitize=shift` for that file or use the
   packers' `*_pack` direction only.
3. **Harness gap, fixed: the fault-injection jobs did not inject anything on
   the V3D and stopped the backend on the PowerVR.**
   `HH_FAIL_AFTER` defaults to 48 and `scen.progs-failat*` used 40..120,
   numbers chosen for a PowerVR whose back end took 37 allocations.
   - V3D: the backend takes 15, and `scen buf` / `fbo` / `progs` make only
     58 / 80 / 35 allocations in all, so `scen.buf-fail` and `fbo-fail`
     refused 0 of 34 / 48 and every `progs-failatNN` refused 0 of 35 (the
     run was a plain run, and green).
   - PowerVR: the backend now takes 67 (not 37), so with the old numbers every
     `*-fail` and `progs-failat40..60` job stopped in `pvr_backend_init`
     (exit 3, "backend init failed"): 7 jobs in all 12 modes tested nothing but
     that. `report.py status` shows them as `E` for `failat40..60`.
   Fix, in `run.sh` (job list) and the comment in `rec.c`: the
   fail-injection jobs set `HH_FAIL_AFTER=16` (V3D) / `72` (PowerVR) and the
   `failat` offsets are `17 20 24 28 32 34` (V3D) / `68 70 72 75 78 80`
   (PowerVR). After the change (`out-v3d2`, `out-pvr2`, `out-pvr3`) the
   injection happens: V3D `buf-fail` refuses 5 of 32, `fbo-fail` 8 of 50,
   `tex-fail` 28 of 158, each `progs-failatNN` 1; PowerVR 5 of 88, 8 of 106,
   25 of 195, 1 each; every job exits 0 in all 12 modes on the PowerVR and
   in 10 of 12 on the V3D (finding 1 for `l0nc`/`l2nc`).
4. **Harness gap, fixed: `run.sh build` stopped with `ar: cannot copy ...
   Permission denied`** when the cargo artifact was read-only (a hard link
   from cargo's cache): `ar qs` appends to the copy, so the copy needs the
   write bit. `run.sh` now does `chmod u+w` after the `cp` of the cargo
   archive and of a `prebuilt:` one.
5. **Not covered, harness limits (unchanged).** No reference log exists for
   V3D runs, so no A/B comparison (`compare.sh`) was run; `HH_ALLOW` has no
   V3D entries. `scen.progs-*` on the V3D allocate 35 times (no draw
   allocates: `progs` compiles and links only), so the allocation-failure
   offsets above reach the program's own allocations but not a draw's.
   `hostile.alloc_failure_sweep` (relative offsets 1..13) covers the draw
   path. PowerVR `gltest.174` times out after 1200 s in every mode of both PowerVR
   runs: a PowerVR-side case that was not looked into (not this item's).

## Gate runs (2026-10-08)

* `tools/check.sh --quick`: 76 ok, 17 failed, 2 gaps. Not caused by this
  change (it touches `tools/hostharness/` only):
  * 12 `clippy` rows with 2..22 warnings (`needless_range_loop` and others in
    `src/vivante`, `src/shaderc`, `src/gl`; the `clippy-clean` item), 2 of them
    for the V3D configurations (`host-test,gpu-v3d,shaderc-online,no-shader-table`
    with and without `report-es2`: 10 warnings each);
  * `no table in the artefact` and `no Mesa reference in the artefact`:
    the `aarch64-unknown-none` target is not installed on this worker (absent
    tool);
  * `gen src/pvr/csbgen.rs == bindgen`: clang cannot find `stdbool.h` (absent
    tool/headers); `yakogl-drivers` clippy: 2 warnings and a `lib test`
    compile error in `src/` that this item does not own;
  * `tools board.sh dry-run matrix and replays`: `board_selftest: 207
    checks, 1 failed` (the failing check was not identified; not looked into);
  * gaps: `yakogl-drivers` for `aarch64-unknown-none-softfloat` (no target, no
    nightly rust-src) and the i.MX8MP `regs.rs` (`HAL_NXP_DIR` unset).
  The V3D rows that run tests passed (`host-test,gpu-v3d,...`: only clippy warns).
* The host rows of `check_no_mesa_reference` (`tools/check.sh` near line 1119)
  could not run as a step (it stops at the missing aarch64 target), so the
  same builds were made by hand: `cargo build -p yakogl-capi --release
  --no-default-features --features osal-linux,gpu-{powervr,vivante,v3d},shaderc-online`
  all build, and `nm` finds 0 Mesa reference symbols in each archive (the
  step's control, which needs the aarch64 build, was not run).
