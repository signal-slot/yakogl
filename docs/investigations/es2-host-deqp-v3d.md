# ES2 host dEQP-GLES2 on the V3D back end (item es2-host-deqp)

Status: **NOT RUN. The run could not be started on this worker**, for a reason outside the
library: the permission layer refused every command that fetches or builds (below). No count in
this file is a measurement; the table has no V3D numbers on purpose. What this item did produce
is the working-copy diff (appendix) that lets `tools/cts/host-deqp-run.sh` run the V3D back end,
and a static reading of why the V3D library should be able to take part.

## Identification

- Integration commit: `47aa17aea9c7da33d48862ba2c5b92236786cc5b`.
- Feature set meant to be run: `host-test,gpu-v3d,shaderc-online,no-shader-table` plus
  `report-es2` (`V3D_CONFIG` and `V3D_ES2_CONFIG` of `tools/check.sh`). Through CMake that is
  `-DYAKOGL_OSAL=linux -DYAKOGL_GPU=v3d -DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=ON
  -DYAKOGL_SHARED=ON`; `CMakeLists.txt` already accepts `YAKOGL_GPU=v3d` with the Linux OSAL
  (lines 53-94, 176-186), so no CMake change is needed.
- dEQP source: **none available.** `$CCFARM_DEPS_DIR` (here
  `/home/tasuku/.local/share/ccfarm/deps/yakogl`) holds no `VK-GL-CTS`. The documented
  way to get it (`docs/es3-host-env.md`: clone, checkout `93bca01`, `external/fetch_sources.py`,
  apply `tools/cts/cts-yakogl-target.patch`) was attempted as
  `git clone https://github.com/KhronosGroup/VK-GL-CTS` into the deps directory and, failing
  that, into `build/`. Both were refused with "This command requires approval" and the worker is
  unattended, so nobody could grant it.

## Why nothing ran

| Step | Command | Result |
|---|---|---|
| dEQP checkout | `git clone ... VK-GL-CTS` (deps dir, then `build/`) | refused, needs approval |
| V3D library, CMake | `cmake -S . -B build/host-deqp/lib-v3d -DYAKOGL_GPU=v3d ...` | refused, needs approval |
| V3D library, cargo | `cargo build --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --lib` | refused, needs approval |
| copy of the script | `cp tools/cts/host-deqp-run.sh build/scratch/host-deqp-run-v3d.sh` and edit | done (diff below) |

**Retry after the farm allowed `cmake`, `cargo build` and a `build/` clone:**

- `cargo build --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --lib`
  **succeeds** (30 s, dev profile): the V3D ES2 feature set compiles on the host.
- The CMake configure (`-DYAKOGL_GPU=v3d ...`) **fails on this worker**, before any V3D code:
  `capi/yakogl_capi.cmake:338: YakoGL: no rustc next to /home/tasuku/.local/lib/kache/shims/cargo`.
  The `cargo` found on PATH is a kache shim with no `rustc` beside it; it is a worker
  environment fault, not a V3D one. Pointing CMake at a real toolchain (a `cargo` with `rustc`
  in the same directory) is the next thing to try.
- `git clone https://github.com/KhronosGroup/VK-GL-CTS build/VK-GL-CTS` fails with
  `fatal: refusing to work with credential missing host field` (a git credential-helper
  fault in this environment); the variant with `-c credential.helper=` was refused again.
  So there is still no dEQP checkout.

So it is not known whether the V3D library links as `libGLESv2.so.2` and survives dEQP on the
host; no failure of the library was observed either. Nothing in the V3D code was found that
would stop it: the script's only GPU-specific line is the library directory, and
`tools/check.sh` already runs the V3D GL suite on the online compiler on the host.

## Commands to run it (for the next worker with approval)

```sh
# 1. dEQP, once (docs/es3-host-env.md)
git clone https://github.com/KhronosGroup/VK-GL-CTS "$CCFARM_DEPS_DIR/VK-GL-CTS"
git -C "$CCFARM_DEPS_DIR/VK-GL-CTS" checkout 93bca01
python3 "$CCFARM_DEPS_DIR/VK-GL-CTS/external/fetch_sources.py"
git -C "$CCFARM_DEPS_DIR/VK-GL-CTS" apply "$PWD/tools/cts/cts-yakogl-target.patch"
# 2. the working copy (appendix), run from the repository root
CTS=$CCFARM_DEPS_DIR/VK-GL-CTS GPU=v3d bash build/scratch/host-deqp-run-v3d.sh
#   -> build/host-deqp/run-<commit>-v3d ; ends with cts-refusals.py --cases 2
python3 tools/cts/cts-refusals.py --cases 60 --tokens 3 build/host-deqp/run-<commit>-v3d
# 3. negative_api split: the same scratch groups.py the baseline used (reuses scan()
#    from cts-refusals.py), filtered on functional.negative_api.*
```

## Counts (to fill in)

| | GC7000 | PowerVR | V3D |
|---|---|---|---|
| Cases asked | 17165 | 17165 | not run |
| Cases begun | 17165 | 17165 | not run |
| Chunks (aborted) | 172 (0) | 172 (0) | not run |
| Compile refusals | 878 / 92 reasons | 878 / 92 reasons | not run |
| Gaps | 0 | 0 | not run |
| Correct refusals | 878 on 876 cases | 878 on 876 cases | not run |
| Back-end refusals | 0 | 0 | not run |
| negative_api pass / fail / NotSupported | 237 / 0 / 9 | 237 / 0 / 9 | not run |

Named gaps for V3D: none known, because none were measured.

## Appendix: the working-copy diff

Against `tools/cts/host-deqp-run.sh` (`tools/cts/*` is unchanged by this item). The copy is
`build/scratch/host-deqp-run-v3d.sh`; it is not committed, so the diff is recorded here. The
first hunk exists only because the copy lives outside `tools/cts/`; the script adopting a
switch needs just the second.

```diff
-HERE=$(cd "$(dirname "$0")" && pwd)
+HERE=$(cd "$(dirname "$0")/../../tools/cts" && pwd)   # working copy lives in build/scratch
@@ case "$GPU" in
     powervr) LIB=$BASE/lib-powervr; SUFFIX=-powervr ;;
+    v3d) LIB=$BASE/lib-v3d; SUFFIX=-v3d ;;
-    *) echo "host-deqp-run: GPU must be vivante or powervr, not '$GPU'" >&2; exit 1 ;;
+    *) echo "host-deqp-run: GPU must be vivante, powervr or v3d, not '$GPU'" >&2; exit 1 ;;
```

The header comment's `GPU  vivante | powervr` line should gain `| v3d`. The CMake call needs no
change: `-DYAKOGL_GPU="$GPU"` already selects the V3D (the `no-shader-table` and `host-test`
cargo features of `check.sh` are what the cargo test rows use; whether the CMake path implies
equivalent ones is the first thing to check when the run happens).
