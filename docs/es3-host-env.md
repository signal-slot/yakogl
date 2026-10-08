# Host dEQP environment for GLES3 (plan #3, cts-host)

Every number below is **HOST-ONLY**: the library is built against the LINUX
OSAL, which records but does not execute the command stream, so render
verdicts mean nothing; only GL API errors and shader compile/link outcomes
are real (see the header of `tools/cts/host-deqp-run.sh`).

## Checkout

- Path: `$CCFARM_DEPS_DIR/VK-GL-CTS`
  (`/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS` on this farm node).
- Commit: `93bca0186` (the revision of `tools/cts/RESULTS.md` §1p/§1r), detached HEAD.
- Built into it, uncommitted: `targets/yakogl/*` from `tools/cts/cts-yakogl-target.patch`,
  and the external sources from `external/fetch_sources.py`.

```sh
git clone https://github.com/KhronosGroup/VK-GL-CTS "$CCFARM_DEPS_DIR/VK-GL-CTS"
git -C "$CCFARM_DEPS_DIR/VK-GL-CTS" checkout 93bca01
python3 "$CCFARM_DEPS_DIR/VK-GL-CTS/external/fetch_sources.py"
git -C "$CCFARM_DEPS_DIR/VK-GL-CTS" apply "$PWD/tools/cts/cts-yakogl-target.patch"
```

No file in the repository needed a fix for this CTS revision.

## Host builds and runs

`host-deqp-run.sh` configures and builds everything itself (Ninja, Release,
`-DDEQP_TARGET=yakogl -DSELECTED_BUILD_TARGETS=deqp-<module>`). It has no `info`
command: its argument is a substring filter on case names.

```sh
export CTS="$CCFARM_DEPS_DIR/VK-GL-CTS"
tools/cts/host-deqp-run.sh info                          # gles2 (default)
MODULE=gles3 tools/cts/host-deqp-run.sh dEQP-GLES3.info. # gles3
```

| Module | deqp build dir | library dir | run dir |
|---|---|---|---|
| gles2 | `build/host-deqp/deqp` (log `deqp-build.log`, 713 steps) | `build/host-deqp/lib` | `build/host-deqp/run-<commit>` |
| gles3 | `build/host-deqp/deqp-gles3` (log `deqp-gles3-build.log`, 748 steps) | `build/host-deqp/lib-es3` (`YAKOGL_REPORT_ES2=OFF`) | `build/host-deqp/run-<commit>-gles3` |

The gles3 case list is the union of `$CTS/android/cts/main/gles3-main-*.txt`.

## Results (HOST-ONLY)

- gles2, filter `info`: 169 of 169 cases began, 2 chunks, both exit 0, 0 compile refusals.
  (The filter also matches other cases containing "info".)
- gles3, filter `dEQP-GLES3.info.`: 6 of 6 cases began, exit 0, 0 refusals.
  - `info.version`: `OpenGL ES 3.0 YakoGL`
  - `info.shading_language_version`: `OpenGL ES GLSL ES 3.00`
  - `info.renderer`: `Vivante GC7000UL (YakoGL)`, `info.vendor`: `Signal Slot`
- Run times: the cold gles2 build plus run took roughly 10-15 minutes on this node
  (it passed the 10-minute foreground limit); the gles3 deqp build was interrupted
  once by a worker restart and resumed by Ninja. Exact per-step times were not
  recorded. The info runs themselves take seconds once built.

## Caveats

- The default GPU is `vivante`; `GPU=powervr` uses `lib-powervr-es3`. V3D has no
  host selection in this script.
- Run directories are removed and recreated on each invocation (`rm -rf "$OUT"`),
  so parallel runs of the same module and commit must set `OUT=`.
