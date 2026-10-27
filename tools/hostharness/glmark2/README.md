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
  plus the draws refused, the GL errors raised and the draws that sampled an
  incomplete texture per scene (`yakogl_draws_refused`,
  `yakogl_gl_errors_raised`, `yakogl_incomplete_texture_draws`);
* the EGL config glmark2 chose.

On the V3D it also records the GPU memory: linux.c models the Pi 4's
carve-out (its size from the Pi 4's glmark2 image), so an allocation the
board would refuse is refused here, with the board's "out of GPU memory"
line, and each entry's high-water is listed. On the GC7000 it models the
i.MX8MP's carve-out (its size from the i.MX8MP glmark2 image, issue #814):
an allocation past it fails as `gc_render_alloc` fails on the board, which
the GL core turns into a GL error.

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
order; `shim.c`), `GM2_PROFILE`, `GM2_PERF` and `GM2_ENTRIES` (Profile,
below), the V3D's settings `YAKOGL_V3D_*` and the GC7000's `YAKOGL_GC_CARVEOUT_BYTES`
(below). It needs
gcc, g++, cmake, cargo, objcopy and python3, and no network.

### The V3D's settings

linux.c reads the settings the Zephyr V3D OSAL takes from Kconfig from the
environment instead (`src/osal/linux.c`, "the V3D"): `YAKOGL_V3D_CARVEOUT_BYTES`,
`YAKOGL_V3D_BIN_OVERFLOW_BLOCKS`, `YAKOGL_V3D_CARVEOUT_TOP_BELOW`,
`YAKOGL_V3D_DRAWS_PER_JOB`, `YAKOGL_V3D_TILED_COPIES_BYTES`,
`YAKOGL_V3D_SPLIT_ZS`, `YAKOGL_V3D_S8_STENCIL`, `YAKOGL_V3D_PACK_SKIP`,
`YAKOGL_V3D_TFU`, `YAKOGL_V3D_TFU_MIN_BYTES`, each defaulting to its Kconfig
default. `run.sh` sets every one that the Pi 4's glmark2 board file
(`driver/samples/zephyr/glmark2/boards/rpi_4b.conf`) sets and the environment
does not: today the carve-out, 100663296 bytes (96 MiB), the tiled-copy
budget, and the TFU on (`YAKOGL_V3D_TFU=1`: linux.c's model of the TFU runs
the jobs, below). `YAKOGL_V3D_TFU=0` runs the CPU path instead.
To see where a smaller carve-out fails, set it:

```sh
YAKOGL_V3D_CARVEOUT_BYTES=50331648 GM2_WRITE=0 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-48m v3d
```

The values a run used are in `out/v3d-settings.txt` and in the GPU memory
section of `FINDINGS-v3d.md`.

### The GC7000's carve-out

On the board every GC7000 allocation comes from one pool over the device
tree's `gpu-carveout` (`src/osal/zephyr_gc.c`, `gc_render_alloc`), which also
holds the board layer's 800x600 window and its 1280x800 screen copy
(`driver/zephyr/lib/board/imx8mp/board_imx8mp.c`). `run.sh` sets
`YAKOGL_GC_CARVEOUT_BYTES` to the carve-out of the i.MX8MP glmark2 image (the
`gpu-carveout` node of `driver/samples/zephyr/glmark2/boards/verdin_imx8mp_mimx8ml8_a53.overlay`,
else of the board's device tree) less those two buffers, unless the
environment sets it; linux.c refuses an allocation that would take the live
bytes of all heaps past it. The value used is in `out/gc-settings.txt`. The
model is generous: it does not charge the MMU tables and the ring at the
carve-out's head (384 KiB), the board's held frees (`gc_frees.h`) or the
pool's fragmentation. So the gate's glmark2 row fails on any GL error line
and also on an entry whose GC peak (the sum of its heaps' high-waters) is
above 3/4 of the modelled carve-out: at the board's 64 MiB, terrain peaked
at 83% (51 MB) and refract at 79% here with no error, while every i.MX8MP
common set raised GL errors; with the glmark2 image's 240 MiB the board
raised none (b421b447, issue #814).

## Profile

The CPU time of each scene's frames, per GL and EGL entry point and, with
perf, per function of the GL core and the back end:

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-prof v3d
GM2_WRITE=0 GM2_PERF=1 GM2_FRAMES=20 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-perf v3d
```

* `GM2_PROFILE=1`: `shim.c` wraps every entry point it hands glmark2 in a
  timing wrapper (one per prototype of YakoGL's `include/GLES2/gl2.h`,
  `gl2ext.h`, `include/EGL/egl.h`, `eglext.h`; `gen_timed.py` writes the list
  into `<out>/glmark2-obj/gm2-timed.h` at every build). Each process ends
  with a `gm2: gl-time` line per entry point it called (calls, wall ns and
  thread cpu ns in three phases) and one `gm2: gl-frames` line. The phases
  are cut at `eglSwapBuffers`: *setup* is the scene's setup and first frame
  (shader compiles, first-use builds), *frames* are frames 2 to
  `GM2_FRAMES` (the counted frames), *teardown* the rest. `report.py
  --profile` writes `<out>/profile-<gpu>.md`: per entry the counted frames'
  wall and cpu ms a frame, the share spent in GL and EGL calls, and the
  entry points ranked by their wall ms a frame. The thread's cpu clock is a
  system call, so a call of well under a microsecond (`glBindTexture`)
  shows more cpu than wall time: about 0.6 us of clock reading a call. Rank
  by the wall column. The runs go one at a time
  (`GM2_JOBS` defaults to 1), so they do not share the CPU.
* The draw steps. The library times the steps of each draw itself
  (`src/draw_steps.rs`: `GlProgram` .. `GlState` in the GL core, `Gc*` in the
  GC7000 back end) and hands them to the OSAL, which on the host
  (`src/osal/linux.c`, every back end) adds them up and answers the clock the
  laps read. With `GM2_PROFILE=1`, `shim.c` prints one line a run, the totals
  between the first swap and swap `GM2_FRAMES`:
  `gm2: draw-steps counted=<n> scopes=<draws> untimed=<laps> ns=<12 ns, in
  Step order>`. `report.py` turns it into a *Draw steps* table in
  `profile-<gpu>.md`: us a draw in each step and its share. A back end times
  its own steps only (the PowerVR has none beyond the GL core's five).
  `GM2_NOCLK=1` makes the OSAL refuse the clock, as a board without one: no
  lap is timed (`untimed` counts them, the `ns` are zero), so the difference
  of a draw's wall time with and without it is the laps' own cost (about 1.4
  us a lap-read on the V3D host build, `docs/notes/gl-draw-fixed-cost.md`). The
  times are the host's; read ratios. `report-selftest.sh` runs `report.py`
  on the fixtures in `testdata/draw-steps/` against the tables it must write.
* `GM2_PERF=1` (implies `GM2_PROFILE=1`): each entry runs under `perf
  record -e cpu-clock --call-graph dwarf`, started with its events off; `shim.c`
  turns them on at the end of the first swap and off at the end of swap
  `GM2_FRAMES` through perf's control fifos, so the samples are the counted
  frames'. `logs/<gpu>/<i>.perf-self.txt` (a function's own samples) and
  `.perf-children.txt` (with its callees) are `perf report --sort symbol`;
  `profile-<gpu>.md` lists their top 40 (less `_start`, `main` and libc's
  start-up) as ms of the frames' cpu time. perf's own cost makes the frames
  slower (terrain 13.1 ms cpu a frame alone, 15.3 ms under perf): read the
  milliseconds from a `GM2_PROFILE=1` run and the shares from the perf one.
  `GM2_PERF_FREQ` sets the rate (default 2000 Hz). perf must be allowed to
  sample the user's own process (`kernel.perf_event_paranoid` 2 or lower).
* `GM2_ENTRIES=RE` runs the list entries matching the extended regex only,
  and needs `GM2_WRITE=0`.

### Before and after

`compare.py` turns two output directories of `GM2_PROFILE=1` runs (same
scenes, same GPU) into a markdown table of ms per frame per scene. It reads
them with `report.py`'s parser. Run the before tree and the after tree each
with its own output directory, then:

```sh
python3 -I tools/hostharness/glmark2/compare.py [--entry-points [N]] [--metric gl|wall|cpu] \
  [--before-label L] [--after-label L] [--title T] build/glmark2-before build/glmark2-after v3d
```

The GPU is `powervr`, `vivante` or `v3d`. The figure is the counted frames'
time in GL and EGL calls (`--metric wall`: the whole frame; `cpu`: the
thread's cpu time); delta is after minus before, ratio after over before.
`--entry-points` adds the N (default 3) entry points that took most wall time
in the before run. A scene without counted frames in either run is listed
and left out. Pass the commits as labels. The sample is the selftest's fixture
(made-up times, `testdata/compare/`; `compare-selftest.sh` diffs the tool's
output against `expected*.md` and is a row of `tools/check.sh`):

```sh
python3 -I tools/hostharness/glmark2/compare.py --before-label dcbfb30b --after-label fix \
  tools/hostharness/glmark2/testdata/compare/before tools/hostharness/glmark2/testdata/compare/after v3d
```

| scene | dcbfb30b ms/frame | fix ms/frame | delta ms | ratio |
|---|---|---|---|---|
| `terrain` | 63.000 | 18.000 | -45.000 | 0.29x |
| `shadow` | 50.000 | 53.000 | +3.000 | 1.06x |

Not in both runs (left out): `refract`.

What the numbers are: host CPU time on the machine that ran them, not the
Pi 4's (its Cortex-A72 at 1.5 GHz is several times slower, and reads
non-cacheable GPU memory, which the host's malloc'd memory is not). The
ranking of the costs carries over; the milliseconds do not. GPU waits are
not in them: linux.c's submits complete at once. The V3D's TFU jobs are the
board GPU's: linux.c runs them in its model of the unit (the TFU on, as the
Pi 4's glmark2 image has it, `CONFIG_YAKOGL_V3D_TFU=y`), and `shim.c` takes
the model's time (`osal_linux_gpu_time`) out of the call that submitted the
job and out of the frames. The profile gives it apart: a `gm2: gpu-time`
line per entry point (jobs and wall ns per phase), `gpu_jobs` and `gpu_ns`
in `gm2: gl-frames`, and a column of `profile-<gpu>.md`. That column is the
model's host time, not the board's TFU time; on the board the CPU waits for
each job (`zephyr_v3d.c` `osal_v3d_tfu` polls it), which the host cannot
show. perf samples the model too (linux.c's `tfu_*` functions and the
`v3d_tex_*` calls under them). `YAKOGL_V3D_TFU=0` gives the CPU path's
profile. On the V3D the profile also counts the back end's CPU waits for a
job (`v3d_counters`): over the counted frames (`gm2: v3d-frames`) and
inside each entry point (`gm2: v3d-waits`). Those waits cost the host
nothing, so on the host they, not the milliseconds, show a change to the
V3D's job order.

## How it works

| file | what it is |
|---|---|
| `run.sh` | Compiles glmark2's sources unchanged (the list the Zephyr image compiles, with upstream's own `libmatrix/util.cc`), builds `libyakogl.a` per GPU with the top-level CMake (`YAKOGL_OSAL=linux`, `YAKOGL_GPU=powervr\|vivante\|v3d`, `YAKOGL_SHADERC=online`, `YAKOGL_REPORT_ES2=ON`, release: the board images' shader back end and reported version), links, and runs one process per list entry: `--size 800x600 -b :nframes=3 -b <entry>`. Then it calls `report.py`, once for the PowerVR and the GC7000 and once for the V3D. |
| `host_main.cpp` | Upstream's `main.cpp` with a window system of its own. The native window is a `struct gles_native_window` over a malloc'd XRGB8888 buffer, which linux.c maps at its host address (the V3D: with `map_phys`, at a GPU VA of its own, as the Pi 4's OSAL does). A `MainLoop` subclass prints `gm2: scene-begin` and `gm2: scene-end <scene>: <status> frames= draws= refused= gl_errors= incomplete_draws=` (the last two are the GL core counters the board's GLMARK2 record carries; `report.py` checks them against the library's own lines). |
| `shim.c`, `shim.h` | `dlopen`/`dlsym` for glmark2's `SharedLibrary` (renamed in `shared-library.o` only, with `objcopy --redefine-sym`): YakoGL is linked in, as on the Zephyr image. Every symbol is a wrapper of this file or what `eglGetProcAddress` answers. The wrappers log `gm2:` lines on stderr, where linux.c logs the library's own lines, so the order is kept. They also write each shader text to `out/capture/<gpu>/<fnv64>.<stage>`. With `GM2_PROFILE=1` every symbol is wrapped once more in a timing wrapper (Profile, above). |
| `gen_timed.py` | Writes the timing wrappers' entry-point list (`gm2-timed.h`) from YakoGL's GLES2 and EGL headers; `run.sh` runs it at every build. |
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
  page-granular fake device addresses, no limit but 256 MiB a heap; on the
  GC7000 also the board's carve-out over every heap together (above).
* **gpu-v3d**: linux.c's model of the Zephyr V3D OSAL (`src/osal/zephyr_v3d.c`):
  one carve-out for every heap label, the board's GPU address layout (a
  512 KiB page table and a scratch page at its front, allocations from GPU VA
  1 MiB), its 256-byte granule allocator (`src/osal/v3d_granules.h`), its
  free rule (a free is held while the backend records a job,
  `src/osal/v3d_frees.h`), the binner's overflow blocks taken at init, and
  windows mapped with `map_phys` at their own GPU VA. Every submit completes
  inside `submit()`, after the kick is checked (magic, lists and tile memory
  inside GPU memory) and the tile state zeroed. With `YAKOGL_V3D_TFU=1` (the
  glmark2 image's setting) the TFU jobs the backend submits (a render
  target's mip chain, the tiled copies of 64 KiB or more) run on the CPU in
  a model of the unit (`src/osal/linux.c`, "the TFU"): the bytes the
  library's CPU path writes (`tests/v3d_tfu_host.rs`), timed as GPU work.
  Not modelled: the binner (no pool reading), hangs and resets.

## Limits

* Nothing is rendered. A finding that depends on pixels (wrong colours, a
  black texture) shows only through the line the library logs about it, for
  example a GL error or an incomplete-texture line. That is how terrain's
  black texture showed before `GL_OES_texture_npot`.
* The fake GPU never fails, and linux.c's submits complete at once, so no
  in-flight path runs. Only the V3D and the GC7000 can run out of memory
  (their carve-outs); the PowerVR cannot.
* Frames: 3 per entry by default. Per-frame lines are counted, so their
  count scales with `GM2_FRAMES`.
