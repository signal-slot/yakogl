<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
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

On the V3D it also records the GPU memory: linux.c models the Pi 4's
carve-out (its size from the Pi 4's glmark2 image), so an allocation the
board would refuse is refused here, with the board's "out of GPU memory"
line, and each entry's high-water is listed.

The results are in [FINDINGS.md](FINDINGS.md) (PowerVR, GC7000) and
[FINDINGS-v3d.md](FINDINGS-v3d.md) (V3D), beside this file because the run
rewrites their generated parts. The de-duplicated shader texts
are written to `build/glmark2-host/shaders/` by the run and never committed
([`tools/shaders/glmark2/README.md`](../../shaders/glmark2/README.md)).

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

The script builds everything in `build/glmark2-host/`, runs the three GPUs,
writes the shader corpus to `build/glmark2-host/shaders/` and rewrites the
generated parts of `FINDINGS.md` and `FINDINGS-v3d.md`. Arguments: `run.sh <GLMARK2_SRC> [out] [powervr]
[vivante] [v3d]`. Environment: `GM2_FRAMES` (frames per entry, default 3),
`GM2_SIZE` (default `800x600`, glmark2's default and the boards'),
`GM2_WRITE=0` (leave the findings files alone), `GM2_JOBS`, `GM2_TIMEOUT` (seconds per
entry, default 600), `GM2_KICKS=1` (PowerVR: a `gm2: draw <n> fb <fb> kick <k>
submitted <s>` line after every draw, the framebuffer it rendered into, the
kick holding it and the kicks submitted so far, to read a scene's pass
order; `shim.c`), and
the V3D's settings `YAKOGL_V3D_*` (below). It needs
gcc, g++, cmake, cargo, objcopy and python3, and no network.

### The V3D's settings

linux.c reads the settings the Zephyr V3D OSAL takes from Kconfig from the
environment instead (`src/osal/linux.c`, "the V3D"): `YAKOGL_V3D_CARVEOUT_BYTES`,
`YAKOGL_V3D_BIN_OVERFLOW_BLOCKS`, `YAKOGL_V3D_CARVEOUT_TOP_BELOW`,
`YAKOGL_V3D_DRAWS_PER_JOB`, `YAKOGL_V3D_TILED_COPIES_BYTES`,
`YAKOGL_V3D_SPLIT_ZS`, `YAKOGL_V3D_S8_STENCIL`, `YAKOGL_V3D_PACK_SKIP`, each
defaulting to its Kconfig default. `run.sh` sets every one that the Pi 4's
glmark2 board file (`driver/samples/zephyr/glmark2/boards/rpi_4b.conf`) sets
and the environment does not: today the carve-out, 100663296 bytes (96 MiB).
To see where a smaller carve-out fails, set it:

```sh
YAKOGL_V3D_CARVEOUT_BYTES=50331648 GM2_WRITE=0 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-48m v3d
```

The values a run used are in `out/v3d-settings.txt` and in the GPU memory
section of `FINDINGS-v3d.md`.

## How it works

| file | what it is |
|---|---|
| `run.sh` | Compiles glmark2's sources unchanged (the list the Zephyr image compiles, with upstream's own `libmatrix/util.cc`), builds `libyakogl.a` per GPU with the top-level CMake (`YAKOGL_OSAL=linux`, `YAKOGL_GPU=powervr\|vivante\|v3d`, `YAKOGL_SHADERC=online`, `YAKOGL_REPORT_ES2=ON`, release: the board images' shader back end and reported version), links, and runs one process per list entry: `--size 800x600 -b :nframes=3 -b <entry>`. Then it calls `report.py`, once for the PowerVR and the GC7000 and once for the V3D. |
| `host_main.cpp` | Upstream's `main.cpp` with a window system of its own. The native window is a `struct gles_native_window` over a malloc'd XRGB8888 buffer, which linux.c maps at its host address (the V3D: with `map_phys`, at a GPU VA of its own, as the Pi 4's OSAL does). A `MainLoop` subclass prints `gm2: scene-begin` and `gm2: scene-end <scene>: <status> frames= draws= refused=`. |
| `shim.c`, `shim.h` | `dlopen`/`dlsym` for glmark2's `SharedLibrary` (renamed in `shared-library.o` only, with `objcopy --redefine-sym`): YakoGL is linked in, as on the Zephyr image. Every symbol is a wrapper of this file or what `eglGetProcAddress` answers. The wrappers log `gm2:` lines on stderr, where linux.c logs the library's own lines, so the order is kept. They also write each shader text to `out/capture/<gpu>/<fnv64>.<stage>`. |
| `report.py` | Reads the logs and captures. It writes `<out>/shaders/*.glsl` and `index.txt` (with `--shaders`), and the generated parts of `FINDINGS.md` and `FINDINGS-v3d.md` (the V3D's with a GPU memory table: each entry's carve-out high-water, the longest free run at exit, the allocations refused). |

There is one process per entry so that each scene's once-per-process lines are
its own (the incomplete-texture warning, `src/gl/draw.rs`, is logged once
per process). The cost is that a scene does not inherit the previous scene's
memory state, as it does in one board run of the whole list. The pool
exhaustion on the AM62P is a property of the PowerVR OSAL's memory, which
linux.c does not model. The Pi 4's carve-out it does model (below), per
entry, from an empty carve-out.

## GPUs

* **gpu-powervr** and **gpu-vivante**: linux.c's per-heap host memory,
  page-granular fake device addresses, no limit but 256 MiB a heap.
* **gpu-v3d**: linux.c's model of the Zephyr V3D OSAL (`src/osal/zephyr_v3d.c`):
  one carve-out for every heap label, the board's GPU address layout (a
  512 KiB page table and a scratch page at its front, allocations from GPU VA
  1 MiB), its 256-byte granule allocator (`src/osal/v3d_granules.h`), its
  free rule (a free is held while the backend records a job,
  `src/osal/v3d_frees.h`), the binner's overflow blocks taken at init, and
  windows mapped with `map_phys` at their own GPU VA. Every submit completes
  inside `submit()`, after the kick is checked (magic, lists and tile memory
  inside GPU memory) and the tile state zeroed. Not modelled: the binner
  (no pool reading), the TFU (off, the board's default), hangs and resets.

## Limits

* Nothing is rendered. A finding that depends on pixels (wrong colours, a
  black texture) shows only through the line the library logs about it, for
  example a GL error or an incomplete-texture line. That is how terrain's
  black texture showed before `GL_OES_texture_npot`.
* The fake GPU never fails, and linux.c's submits complete at once, so no
  in-flight path runs. Only the V3D can run out of memory (its carve-out);
  the PowerVR and the GC7000 cannot.
* Frames: 3 per entry by default. Per-frame lines are counted, so their
  count scales with `GM2_FRAMES`.
