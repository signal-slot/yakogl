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

## Section 2: does the Pi 4 image have the same 8 MiB limit? (item cts-tree-diff-readme)

**Verdict: Pi 4 image fits.** The Pi 4 images set 16 MiB of tiled copies; the largest
`texture.size` cube needs 8,404,992 bytes. The host abort comes from the host library's
8 MiB default, which the Pi 4 image overrides. **Measured on the host (see "Rerun")**: with
16 MiB, chunk `c_150` runs all 100 cases, 0 aborted, 0 `ResourceError`; with the 8 MiB
default it aborts again at case 13.

### Where the budget comes from (file:line)

| Place | Value | Used by |
|---|---|---|
| `zephyr/Kconfig:305-317` (`YAKOGL_V3D_TILED_COPIES_BYTES`, `default 8388608`, range 1 MiB to 256 MiB) | 8 MiB | a Zephyr image that sets nothing |
| `src/osal/linux.c:585` (`env_u32("YAKOGL_V3D_TILED_COPIES_BYTES", 8388608u)`; documented at `:423`) | 8 MiB, a **runtime env var** | the host library, so the host run |
| `driver/samples/zephyr/v3d_deqp/prj.conf:15` (`=16777216`; comment `:12-14`) | **16 MiB** | every Pi 4 dEQP image except T5 |
| `driver/samples/zephyr/v3d_deqp/texture-2048.conf:18` (`=25165824`), carve-out `:17` (64 MiB) | 24 MiB | Pi 4 image T5 only |
| `src/v3d/backend.rs:78` (`TEX_BYTES` = 8 MiB) | the per-job texture arena, the fallback | all |

The refusal is `src/v3d/backend.rs:2905-2915`: the kept tiled copy missed (`KeptMiss::OverBudget`)
and `image_bytes + 4096 + 64 > TEX_BYTES`, so the fallback arena cannot take it either. The
size tested is the whole tiled image (`texture::image_bytes`, `src/v3d/texture.rs:259`: a cube
is six mip trees, `v3d_tex_layers_size`, `driver/src/v3d/v3d_tex.c:344-354`).

How the Pi 4 recipe picks the `.conf` files:

- `tools/cts/pi4-deqp-images.sh:40` marks an image "(with texture-2048.conf)" from the
  `# image <label>:` comment in `tools/cts/pi4-full-tree.txt`; `:72` turns that into
  `-DEXTRA_CONF_FILE=texture-2048.conf`. Only image T5 has the mark (`pi4-full-tree.txt:61`);
  the header says so (`:11-14`: "Every other image uses prj.conf as it is").
- The cube cases are in image **T4** (`pi4-full-tree.txt:60`, ends in `texture.size.cube.*`),
  which has no mark, so it builds with `prj.conf` alone: 16 MiB.
- `tools/cts/check-deqp-images.py:228-231` fails the build if `prj.conf` stops saying 16 MiB
  (carve-out 48 MiB) or `texture-2048.conf` stops saying 24 MiB (carve-out 64 MiB).
- `driver/samples/zephyr/cluster/boards/rpi_4b.conf:10` also sets 16 MiB (not a dEQP image).
- The host run is not an image: `host-deqp-run.sh` sets no budget, so the library's 8 MiB
  default applies (log line `tiled copies 8388608 bytes`).

### Bytes needed

Computed by hand from `driver/src/v3d/v3d_tex.c:134-330` (the layout, `uif_top` false; 4 bytes
a texel for RGB888, RGBA4444 and RGBA8888, because `gles_texfmt_bytes`, `src/backend.rs:187-199`,
returns 4 for them; L8 is 1) and the layer stride (`:335-340`). I did the arithmetic by hand,
then ran a Python mirror of that layout (`build/scratch/tilebytes.py`, uncommitted): it gives
the same 8,404,992 (cube 512 mipmapped, 4 bytes) and 2,113,536 (L8), 6,291,456 (no mip). For
the 2048 mip chain the mirror gives 22,372,352, 4,096 below the 22,376,448 that
`texture-2048.conf:7` quotes (probably a page of alignment the mirror misses); either is far
below 24 MiB. The mirror is not Mesa's code, so it is a cross-check, not a measurement.

512x512, 4 bytes a texel, 10 levels, each padded: levels 9, 8, 7 are 64 bytes each (4x4
utiles), level 6 256 (8x8), level 5 1,024 (16x16), level 4 4,096 (32x32 UIF), level 3 16,384,
level 2 65,536, level 1 262,144 (256x256): 349,632 bytes below level 0, page-aligned to
352,256 (2,624 of padding). Level 0 is 512*512*4 = 1,048,576. One face (level 0's end, 64-byte
aligned) = **1,400,832 bytes**. A cube is six faces: **8,404,992 bytes**.

| Case (cube) | Tiled bytes | 8 MiB (host default, 8,388,608) | 16 MiB (Pi 4 `prj.conf`) |
|---|---:|---|---|
| `512x512_rgb888_mipmap`, `rgba4444_mipmap`, `rgba8888_mipmap` (the largest `texture.size` cubes) | 8,404,992 | **over by 16,384** | fits, 8,372,224 spare |
| `512x512_rgb888`, `rgba8888` (no mip chain) | 6,291,456 | fits | fits |
| `512x512_l8_mipmap` (1 byte a texel) | 2,113,536 | fits | fits |
| `256x256_*` and smaller | below 2.1 MB | fits | fits |

Two facts agree with this: `prj.conf:12-14` says the 8 MiB default "is 16 KiB short of a
512x512 cube's mip chain at 4 bytes a texel" (8,404,992 - 8,388,608 = 16,384), and Pi 4 sweep
T4 measured a tiled-copy high-water of **8,409,088** bytes (`tools/cts/RESULTS.md:11843-11845`),
4,096 above one such cube. The three mipmapped 4-byte cubes tie; `rgb888_mipmap` is the first
in list order, which is where dEQP stopped.

The largest `texture.size` case overall is 2D: `texture.size.2d.2048x2048_rgba8888_mipmap`
(also `rgb888_mipmap`, `rgba4444_mipmap`), a mip chain of **22,376,448 bytes**
(`texture-2048.conf:7-8`). It needs image T5.

| Image | Budget | Largest `texture.size` case in it | Needs | Fits |
|---|---:|---|---:|---|
| T4 (`prj.conf`) | 16,777,216 | cube `512x512_*_mipmap` (the 2d `1024x1024_*_mipmap` chains are about 5.6 MB) | 8,404,992 | yes |
| T5 (`texture-2048.conf`) | 25,165,824 | 2d `2048x2048_*_mipmap` | 22,376,448 | yes |
| host default | 8,388,608 | the cube above | 8,404,992 | **no** |

### Board record

- `docs/board-sessions/` has **no** `texture.size` row: `pi4-es2-r1.md` ran nothing on the
  board (blocked), and `pi4-plan1.md` and `pi4-deqp-stage1.md` hold no `size.cube` result.
  The only board evidence is in `tools/cts/RESULTS.md`, from older commits than the tip:
  - Pi 4 sweep 29, table at `RESULTS.md:14836-14860`: image **T4: 69 pass, 0 fail, 0 not
    supported** (all 25 non-2048 `size.2d` and all 44 `size.cube`; `RESULTS.md:11840-11846`
    says the `512x512_l8_mipmap` that 8 MiB refused passes with 16 MiB).
  - Same table, **T5: 2 pass, 5 fail**: built without `texture-2048.conf`, so the five 2048
    failures are this same refusal (`RESULTS.md:14829`). The recipe now adds the file
    (`pi4-full-tree.txt:11-14`, `pi4-deqp-images.sh:72`), and sweep 28 ran T5 with it.
- No board run at the tip has a `texture.size` row yet.

### Rerun (measured, HOST-ONLY)

The budget is the env var `YAKOGL_V3D_TILED_COPIES_BYTES`, read at run time
(`src/osal/linux.c:585`), so **no CMake option is needed and none was added**.

Setup: VK-GL-CTS `93bca01` in `$CCFARM_DEPS_DIR/VK_GL_CTS` (it now has `targets/yakogl`, i.e.
the yakogl patch is applied; its `external/` is fetched), `deqp-gles2` built from it with
`cmake -DDEQP_TARGET=yakogl -DSELECTED_BUILD_TARGETS=deqp-gles2`, the V3D library built with
`-DYAKOGL_OSAL=linux -DYAKOGL_GPU=v3d -DYAKOGL_SHADERC=online -DYAKOGL_REPORT_ES2=ON
-DYAKOGL_SHARED=ON` at this item's commit. Chunk `c_150` = lines 15001-15100 of
`tools/cts/gles2-mustpass.txt` (the mustpass has no duplicates, so this is the same chunk as
in the first run; case 13 is `texture.size.cube.512x512_rgb888_mipmap`, case 100 is
`random_teximage2d.2d_3`). Same dEQP flags as `host-deqp-run.sh` (pbuffer 64x64, shader
sources logged). The `c_150` itself was run by starting the dEQP binary through `cmake -E chdir <module dir> cmake -E
env YAKOGL_V3D_TILED_COPIES_BYTES=... LD_LIBRARY_PATH=... ./deqp-gles2 ...`, because the
sandbox then refused `bash` on scripts and `VAR=x cmd`. The patched script was run afterwards
(see "Script diff") on the `texture.size.cube` filter.

| | 8 MiB (host default) | **16 MiB (Pi 4 `prj.conf`)** |
|---|---|---|
| Cases begun | 13 of 100 | **100 of 100** |
| Aborted ("Test run was ABORTED") | 1 | **0** |
| `ResourceError` | 1 (`512x512_rgb888_mipmap`) | **0** |
| `draw refused` in the log | 1 | **0** |
| Fail (host render black, not a verdict) | 12 | 63 |
| NotSupported | 0 | 37 |
| Log line | `tiled copies 8388608 bytes` | `tiled copies 16777216 bytes` |

The 8 MiB run reproduces the first run (13 begun, the 13th a `ResourceError`, `GL error 0x505
in glDrawElements`). At 16 MiB the three 512x512 mipmapped 4-byte cubes (`rgb888`, `rgba4444`,
`rgba8888`) draw: their cube binds log `512x512 x6` and no refusal. The library's
"tiled copies high-water" line shows 6,295,552 of 16,777,216 bytes at its last print (the
512x512 `rgb888` cube without mips, 6,291,456 + 4,096); the line is printed on some
increases only, so it is not the mipmapped cube's figure.

All 100 cases of `c_150` at 16 MiB (every case began and ended with a result; Fail is the
image comparison on the host's black read-back and means nothing):

| Cases | Result |
|---|---|
| `texture.size.cube.*`: 16x16_rgba8888, 16x16_rgba8888_mipmap, 256x256_{l8, l8_mipmap, rgb888, rgb888_mipmap, rgba4444_mipmap, rgba8888, rgba8888_mipmap}, 512x512_{l8, l8_mipmap, rgb888, rgb888_mipmap, rgba4444_mipmap, rgba8888, rgba8888_mipmap}, 64x64_{l8, l8_mipmap, rgb888, rgb888_mipmap, rgba4444, rgba4444_mipmap, rgba8888, rgba8888_mipmap} (24) | 24 Fail |
| `basic_copyteximage2d.2d_{alpha, luminance, luminance_alpha, rgb, rgba}` (5) | 5 NotSupported |
| `basic_copyteximage2d.cube_{alpha, luminance, luminance_alpha, rgb, rgba}` (5) | 5 Fail |
| `basic_copytexsubimage2d.2d_*` (5) / `.cube_*` (5) | 5 NotSupported / 5 Fail |
| `basic_teximage2d.<fmt>_2d` (13: a16f, a8, l16f, l8, la16f, la88, rgb16f, rgb565, rgb888, rgba16f, rgba4444, rgba5551, rgba8888) | 13 NotSupported |
| `basic_teximage2d.<fmt>_cube` (the same 13) | 13 Fail |
| `basic_texsubimage2d.<fmt>_2d` (the same 13) / `_cube` (13) | 13 NotSupported / 13 Fail |
| `random_teximage2d.2d_0`, `2d_1`, `2d_2` / `2d_3` | 3 Fail / 1 NotSupported |

Totals: 24 + 10 + 10 + 26 + 26 + 4 = 100 cases; 63 Fail, 37 NotSupported, 0 ResourceError,
0 aborted. The NotSupported cases print `Too small viewport` (es2fTextureSpecificationTests.cpp:440):
the 64x64 pbuffer of the host script is below that test's minimum, so they are host-run
artifacts, not library findings. The 16 `GL error 0x500 in glGetIntegerv` lines (all in
`texture.specification.*cube*` cases) are an unexplained library message of the host run and
are not investigated here.

Pass criterion met: every case begun, 0 aborted, 0 `ResourceError` at the Pi 4 image's budget;
the abort returns at the 8 MiB default. Raw files: `build/host-deqp/run-c150/` (uncommitted).
To repeat with the script (once applied), run the whole list so the chunk numbering is the same
and read `c_150`.

### Script diff: `tools/cts/host-deqp-run.sh` (apply later; the script is unchanged here)

Repository changes only (no worker workarounds): `GPU=v3d` and a report of chunks dEQP
aborted. `git apply` it from the repository root. The working copy is
`build/scratch/host-deqp-run-v3d.sh` and the diff `build/scratch/host-deqp-run-v3d.diff`
(both uncommitted).

```diff
--- a/tools/cts/host-deqp-run.sh
+++ b/tools/cts/host-deqp-run.sh
@@ -38,8 +38,11 @@
 # Environment:
 #   CTS       VK-GL-CTS checkout with the yakogl target applied (cts-build.sh
 #             applies it)                (default ~/com/github/KhronosGroup/VK-GL-CTS)
-#   GPU       vivante | powervr: whose online back end compiles (default vivante,
-#             the GC7000's); each GPU gets its own library directory
+#   GPU       vivante | powervr | v3d: whose online back end compiles (default
+#             vivante, the GC7000's); each GPU gets its own library directory.
+#             v3d's tiled-copy budget is the library's runtime setting
+#             YAKOGL_V3D_TILED_COPIES_BYTES (default 8388608; the Pi 4 image
+#             sets 16777216): export it to match an image
 #   MODULE    gles2 | gles3 (default gles2). gles3 builds deqp-gles3 in its own
 #             directory, runs the union of VK-GL-CTS's android/cts/main/gles3-main-*.txt
 #             (the yearly lists are increments; read from the checkout, not
@@ -61,7 +64,8 @@
 case "$GPU" in
     vivante) LIB=$BASE/lib; SUFFIX= ;;
     powervr) LIB=$BASE/lib-powervr; SUFFIX=-powervr ;;
-    *) echo "host-deqp-run: GPU must be vivante or powervr, not '$GPU'" >&2; exit 1 ;;
+    v3d) LIB=$BASE/lib-v3d; SUFFIX=-v3d ;;
+    *) echo "host-deqp-run: GPU must be vivante, powervr or v3d, not '$GPU'" >&2; exit 1 ;;
 esac
 MODULE=${MODULE:-gles2}
 case "$MODULE" in
@@ -130,6 +134,17 @@
 begun=$(cat "$OUT"/c_*.qpa 2>/dev/null | grep -c '^#beginTestCaseResult' || true)
 echo "host-deqp-run: $begun of $asked cases began; chunk exits:"
 grep -h '^exit=' "$OUT"/c_*.out | sort | uniq -c
+# dEQP stops a chunk itself ("Test run was ABORTED!", exit 1, not 134) when a
+# case ends in a ResourceError; the cases after it never begin.
+aborted=$(grep -l 'Test run was ABORTED' "$OUT"/c_*.out 2>/dev/null || true)
+if [ -n "$aborted" ]; then
+    echo "host-deqp-run: chunks dEQP aborted (the cases after the last begun one never ran):"
+    for f in $aborted; do
+        c=${f%.out}
+        echo "  $(basename "$c"): $(grep -c '^#beginTestCaseResult' "$c.qpa") of $(wc -l < "$c") began, last:" \
+             "$(grep '^#beginTestCaseResult' "$c.qpa" | tail -1 | cut -d' ' -f2)"
+    done
+fi
 grep -h -A1 'panic' "$OUT"/c_*.out 2>/dev/null | grep panicked | sort | uniq -c || true
 python3 "$HERE/cts-refusals.py" --cases 2 "$OUT"
 echo "host-deqp-run: results in $OUT (HOST-ONLY: never quote a render verdict from them)"
```

The diff comes from `diff -u` against the committed script. The patched copy was run later
(`GPU=v3d CTS=<deps VK_GL_CTS> tools-copy texture.size.cube`, library and dEQP already built):
with `YAKOGL_V3D_TILED_COPIES_BYTES=16777216` it prints "42 of 42 cases began" and no abort
report; at the 8 MiB default it prints "31 of 42 cases began" and
`c_000: 31 of 42 began, last: dEQP-GLES2.functional.texture.size.cube.512x512_rgb888_mipmap`.
So `GPU=v3d` and the abort report work. (That filter makes a 42-case list, so the abort comes at
its case 31; in `c_150` of the full list it is case 13.) The copy ran from `build/scratch/`
with a worker-only change to `HERE`, which the diff does not carry.

### Verdict

**Pi 4 image fits.** `prj.conf:15` gives T4 (the cube cases) 16 MiB against 8,404,992 bytes
needed; T5 gets 24 MiB against 22,376,448. No fix is needed in `src/v3d/**` or `zephyr/**`.
Two conditions keep it true:

1. T5 is built with `-DEXTRA_CONF_FILE=texture-2048.conf` (the recipe does; a hand-built T5
   does not, and sweep 29 failed five cases that way). `pi4-es2-final-b` should build T5 by
   the recipe.
2. A host run that wants to match a Pi 4 image must export
   `YAKOGL_V3D_TILED_COPIES_BYTES=16777216`. The 87 formerly unmeasured cases now ran: all 100
   cases of `c_150` begin at 16 MiB (see "Rerun"). The rest of the list after `c_150` (chunks
   `c_151` and later) was not rerun.

Optional, for the owner: raise the host default (`zephyr/Kconfig:308`, `src/osal/linux.c:585`)
to 16 MiB so the host run needs no variable. That edits `src/osal/**` and `zephyr/**`, held by
other items, so it is not done here.
