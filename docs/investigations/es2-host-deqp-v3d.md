# ES2 host dEQP-GLES2 on the V3D back end (item es2-host-deqp)

Every number below is **HOST-ONLY**, as in `es2-host-deqp-baseline.md`: the library is built
with the Linux OSAL, which keeps the command stream instead of executing it, so every
`glReadPixels` reads black. Only compile/link outcomes, info logs and API error behaviour are
meaningful. No render verdict is quoted.

Status: **measured, with one abort.** The V3D library runs dEQP on the host. 0 compile gaps,
878 correct refusals, negative_api clean. But the run **aborted in one chunk**: 87 of 17165
cases never began, because a draw was refused for a large cube texture (below).

## Identification

- Commit: `47aa17aea9c7` (integration tip) plus this item's docs commits; no source differs.
  The script labelled the run `ea27ae6d`.
- Feature set: `YAKOGL_GPU=v3d`, `YAKOGL_OSAL=linux`, `YAKOGL_SHADERC=online`,
  `YAKOGL_REPORT_ES2=ON`, `YAKOGL_SHARED=ON` (the CMake route; `CMakeLists.txt` already
  accepts `v3d`). The cargo rows of `tools/check.sh` (`gpu-v3d,report-es2` etc.) were not what
  was run; `cargo build --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --lib`
  also builds.
- dEQP: VK-GL-CTS at `93bca01` (pinned in `docs/es3-host-env.md`), cloned into the workspace's
  `build/VK-GL-CTS` (the deps directory was not writable by an approved command), 
  `external/fetch_sources.py` run, `tools/cts/cts-yakogl-target.patch` applied.
  `deqp-gles2` built by the script into `build/host-deqp/deqp`.
- Mustpass list `tools/cts/gles2-mustpass.txt`, default chunking (100 per process).
- Library log line: `v3d carve-out 50331648 bytes ... draws a job 1024, tiled copies 8388608
  bytes`.

## Commands

```sh
# dEQP, per docs/es3-host-env.md (into build/ here)
GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
  git clone https://github.com/KhronosGroup/VK-GL-CTS build/VK-GL-CTS
git -C build/VK-GL-CTS checkout 93bca01
python3 build/VK-GL-CTS/external/fetch_sources.py
git -C build/VK-GL-CTS apply "$PWD/tools/cts/cts-yakogl-target.patch"
# the working copy (appendix) of tools/cts/host-deqp-run.sh
bash build/scratch/host-deqp-run-v3d.sh      # -> build/host-deqp/run-ea27ae6d-v3d
```

The two git variables work around a broken git credential helper on this worker
(`refusing to work with credential missing host field`, also hit by dEQP's own sub-clones
and by CMake's FetchContent of `shaderc`); the cargo path works around a kache `cargo` shim
with no `rustc` beside it (`no rustc next to .../kache/shims/cargo`). Both are worker
faults, not library faults. The negative_api and abort figures were read straight from the
`.qpa`/`.out` files with grep (the scratch `groups.py` of the baseline was not available).

## Counts

| | GC7000 | PowerVR | **V3D** |
|---|---|---|---|
| Cases asked | 17165 | 17165 | 17165 |
| Cases begun | 17165 | 17165 | **17078 (87 short)** |
| Chunks | 172, 0 aborted | 172, 0 aborted | 172: 21 exit 0, 151 exit 1, 0 exit 134; **1 chunk aborted by dEQP** (`c_150`, 13 of 100 began) |
| Compile refusals | 878 / 92 reasons | 878 / 92 | 878 / 92 reasons |
| Gaps (dEQP failed) | 0 | 0 | **0** |
| NotSupported refusals | 0 | 0 | 0 |
| Correct refusals | 878 on 876 cases | 878 on 876 | 878 (all 92 reasons are negative cases dEQP passed) |
| Back-end (code generator) refusals | 0 | 0 | 0 seen |
| negative_api | 237 pass / 0 fail / 9 NS | 237 / 0 / 9 | **237 pass / 0 fail / 9 NS** (246 cases) |

The refusal buckets are the same as the other back ends (top reasons: no implicit
conversion 264, expected a name 57, not enough constructor arguments 54, ...).

### negative_api on V3D

246 cases (chunks `c_036`-`c_039`): 237 Pass, 9 NotSupported, **0 Fail** — identical to
GC7000 and PowerVR. The nine are the same `negative_api.texture.compressedtexsubimage2d_*`
cases ("No suitable compressed formats found": `GL_NUM_COMPRESSED_TEXTURE_FORMATS` is 0). No
ES 3-only enum or format was accepted. (Counted from the `.qpa` files by pairing each
negative_api `CasePath` line with its `<Result StatusCode>` line.)

## The one abort (named)

Chunk `c_150` (cases 15001-15100 of the list) ended at its 13th case:

- Last case begun: `dEQP-GLES2.functional.texture.size.cube.512x512_rgb888_mipmap`, result
  `ResourceError` ("Set texturing state: glGetError() returned GL_OUT_OF_MEMORY at
  es2fTextureSizeTests.cpp:313"); dEQP then reports "Test run was ABORTED!" (exit 1, not 134;
  no panic).
- Library log: `yakogl/v3d: draw refused: a texture larger than the tiled-copy budget
  (CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES) and the texture arena (8 MiB tiled)`, then
  `glDrawElements was not recorded (-22)` and `GL error 0x505 in glDrawElements`.
- Not begun: the remaining **87 cases**, from
  `texture.size.cube.512x512_rgba4444_mipmap` through
  `texture.specification.random_teximage2d.2d_3` (lines 14-100 of `c_150`).

The cause is the host model's tiled-copy budget (`tiled copies 8388608 bytes`, 8 MiB): a
512x512 RGB888 cube with mipmaps does not fit. That is a V3D library/board-config limit
(`CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES`), not a compiler gap; whether the Pi 4 image sets
a larger budget is a board-side fact this host run does not know, and the Pi 4 board rows
(`pi4-es2-r1b`) show whether the same case fails there. Nothing else aborted. On GC7000/PowerVR
the same chunk ran to completion, so this is V3D-specific.

## What the zero means

Same caveats as the baseline: 0 gaps means the V3D online compiler refuses no valid
dEQP-GLES2 shader on the host. It says nothing about accepted-but-wrong shaders, invariance,
clear scissor/mask, texture wrap/completeness or the offline table. The new V3D-specific
finding is the 8 MiB tiled-copy budget abort above, which the compile/link tool cannot see
for the 87 cases after it (they are unmeasured, not passed).

## Appendix: the working-copy diff

Against `tools/cts/host-deqp-run.sh` (unchanged by this item; the copy is
`build/scratch/host-deqp-run-v3d.sh`, uncommitted). Hunks 1, 3-5 are worker workarounds that a
repository switch does not need; hunk 2 is the switch.

```diff
-HERE=$(cd "$(dirname "$0")" && pwd)
+HERE=$(cd "$(dirname "$0")/../../tools/cts" && pwd)   # working copy lives in build/scratch
@@ case "$GPU" in
     powervr) LIB=$BASE/lib-powervr; SUFFIX=-powervr ;;
+    v3d) LIB=$BASE/lib-v3d; SUFFIX=-v3d ;;
-    *) echo "host-deqp-run: GPU must be vivante or powervr, not '$GPU'" >&2; exit 1 ;;
+    *) echo "host-deqp-run: GPU must be vivante, powervr or v3d, not '$GPU'" >&2; exit 1 ;;
-CTS=${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}
+PATH=$HOME/.cargo/bin:$PATH   # worker: the kache cargo shim has no rustc beside it
+export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null   # worker: broken git credential helper
+GPU=${GPU:-v3d}
+CTS=${CTS:-$(cd "$(dirname "$0")/../.." && pwd)/build/VK-GL-CTS}
@@ library cmake call
-    -DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=$REPORT_ES2 -DYAKOGL_SHARED=ON > "$LIB-build.log" 2>&1 ||
+    -DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=$REPORT_ES2 -DYAKOGL_SHARED=ON -DYAKOGL_CARGO="$HOME/.cargo/bin/cargo" -UYAKOGL_CARGO_FOUND > "$LIB-build.log" 2>&1 ||
```

The header comment's `GPU  vivante | powervr` line should gain `| v3d`. The script's
"begun against asked" line already reports the 87 shortfall; it does not say that dEQP
aborted a chunk (exit 1, not 134), so an adopter may want to grep `Test run was ABORTED` in
the `c_*.out` files.
