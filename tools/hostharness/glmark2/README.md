# glmark2 on the host harness

glmark2-es2's default benchmark set (the 33 entries of upstream's
`src/default-benchmarks.h`, the list the board images run) on x86-64 Linux,
against YakoGL built for each GPU with the Linux OSAL (`src/osal/linux.c`:
GPU memory is malloc'd, every submit completes at once). Nothing is rendered.
The GL core, the in-process compiler and the back end's command generation all
run, so every shader glmark2 hands the driver is compiled and every draw is
built. What it records per GPU:

* every shader text passed to `glShaderSource`, and every compile or link failure;
* every GL error with its entry point (the library's own `GL error 0x… in gl…`
  line, every one: `YAKOGL_ERROR_LOG`);
* every refused draw and every incomplete-texture line the library logs,
  plus the draws refused per scene (`yakogl_draws_refused`);
* the EGL config glmark2 chose.

The results are in [FINDINGS.md](FINDINGS.md), and the de-duplicated shader
texts are in [`tools/shaders/glmark2/`](../../shaders/glmark2/README.md).

## Command

```sh
tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"
```

`GLMARK2_SRC` is glmark2 at `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the
commit the Zephyr image pins (`driver/samples/zephyr/glmark2/CMakeLists.txt`).
It can be a git checkout without local changes, which the script verifies:

```sh
git clone https://github.com/glmark2/glmark2 "$GLMARK2_SRC"
git -C "$GLMARK2_SRC" checkout 22c527cb0556f3a1ac4445aaa52cc532760928d5
```

It can also be GitHub's archive of that commit, extracted. The directory is
then `glmark2-22c527cb0556f3a1ac4445aaa52cc532760928d5`, and the commit is
the archive's name, since there is no git tree to check:

```sh
curl -fsSL -o glmark2.tar.gz \
  https://github.com/glmark2/glmark2/archive/22c527cb0556f3a1ac4445aaa52cc532760928d5.tar.gz
```

The script builds everything in `build/glmark2-host/`, runs both GPUs and
rewrites `tools/shaders/glmark2/` and the generated part of `FINDINGS.md`.
Arguments: `run.sh <GLMARK2_SRC> [out] [powervr] [vivante]`. Environment:
`GM2_FRAMES` (frames per entry, default 3), `GM2_SIZE` (default `800x600`,
glmark2's default and the boards'), `GM2_WRITE=0` (leave the tree alone),
`GM2_JOBS`, `GM2_TIMEOUT` (seconds per entry, default 600). It needs gcc,
g++, cmake, cargo, objcopy and python3, and no network.

## How it works

| file | what it is |
|---|---|
| `run.sh` | Compiles glmark2's sources unchanged (the list the Zephyr image compiles, with upstream's own `libmatrix/util.cc`), builds `libyakogl.a` per GPU with the top-level CMake (`YAKOGL_OSAL=linux`, `YAKOGL_SHADERC=online`, `YAKOGL_REPORT_ES2=ON`, release: the board images' shader back end and reported version), links, and runs one process per list entry: `--size 800x600 -b :nframes=3 -b <entry>`. Then it calls `report.py`. |
| `host_main.cpp` | Upstream's `main.cpp` with a window system of its own. The native window is a `struct gles_native_window` over a malloc'd XRGB8888 buffer, which linux.c maps at its host address. A `MainLoop` subclass prints `gm2: scene-begin` and `gm2: scene-end <scene>: <status> frames= draws= refused=`. |
| `shim.c`, `shim.h` | `dlopen`/`dlsym` for glmark2's `SharedLibrary` (renamed in `shared-library.o` only, with `objcopy --redefine-sym`): YakoGL is linked in, as on the Zephyr image. Every symbol is a wrapper of this file or what `eglGetProcAddress` answers. The wrappers log `gm2:` lines on stderr, where linux.c logs the library's own lines, so the order is kept. They also write each shader text to `out/capture/<gpu>/<fnv64>.<stage>`. |
| `report.py` | Reads the logs and captures. It writes `tools/shaders/glmark2/*.glsl` and `index.txt`, and the generated part of `FINDINGS.md`. |

There is one process per entry so that each scene's once-per-process lines are
its own (the incomplete-texture warning, `src/gl/draw.rs`, is logged once
per process). The cost is that a scene does not inherit the previous scene's
memory state, as it does in one board run of the whole list. The pool and
carve-out exhaustion on the boards (refract on the AM62P, refract and terrain
on the Pi 4) is a property of the board OSALs' memory, which linux.c does not
model, so this run cannot show it either way.

## GPUs

* **gpu-powervr** and **gpu-vivante** run.
* **gpu-v3d** does not. The Linux OSAL cannot drive the V3D back end:
  * `l_submit` refuses a kick descriptor shorter than `struct pvr_render_kick`,
    and the V3D's `struct v3d_kick` (`src/v3d/v3d_kick.h`) is 40 bytes;
  * the V3D kick holds 32-bit device addresses, but linux.c hands out
    addresses from `0x8000000000` up;
  * `l_query` answers none of the `OSAL_QUERY_V3D_*` queries the back end
    makes at init and per job.

  The CMake host build does not offer `YAKOGL_GPU=v3d` either. The V3D's
  coverage of these shaders is a corpus compile (glmark2-corpus), not this run.

## Limits

* Nothing is rendered. A finding that depends on pixels (wrong colours, a
  black texture) shows only through the line the library logs about it.
  terrain's black texture shows as its `glGenerateMipmap` error and the
  incomplete-texture line.
* The fake GPU never fails and never runs out of memory, and linux.c's
  submits complete at once, so no in-flight path runs.
* Frames: 3 per entry by default. Per-frame lines are counted, so their
  count scales with `GM2_FRAMES`.
