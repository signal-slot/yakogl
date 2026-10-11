<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2-es2 on Zephyr: the performance measure

YakoGL's performance is measured with a standard benchmark, never with our
own demo's frame rate (the user's rule, 2026-10-03). A speed claim is a
same-board glmark2 run; the public scores in section 3 are other stacks under
other conditions, listed for reference only. The benchmark is **glmark2-es2**: Linaro's OpenGL ES 2.0
benchmark, https://github.com/glmark2/glmark2, used here at commit
`22c527cb0556f3a1ac4445aaa52cc532760928d5` (2025-09-29, version 2023.01).

Licence: glmark2 is GPL-3.0-or-later (libmatrix MIT); Apache-2.0 may be combined with it; upstream is fetched at build at commit `22c527cb0556f3a1ac4445aaa52cc532760928d5`, not committed.

The image is `driver/samples/zephyr/glmark2/`. Upstream is compiled by path,
unchanged except one file (`libmatrix/util.cc`, patched in the build
directory).

## 1. What glmark2-es2 needs, and what the port gives it

Paths are in the upstream tree (`src/`) at the pinned commit.

| Need | Where upstream asks | In the image |
|---|---|---|
| C++17, exceptions, RTTI, iostreams, `std::map`/`std::string`, `std::filesystem::path` | `meson.build:4` (`cpp_std=c++17`), `-DUSE_EXCEPTIONS` (`src/meson.build`), `libmatrix/util.h:21` | full libstdc++ of the Zephyr SDK 1.0.1 (GCC 14.3), as dEQP has (`prj.conf`). `std::filesystem::path` is in its `libstdc++.a`. |
| A window system ("flavour"): a `NativeState` | `native-state.h:28-70`; `main.cpp` picks one per flavour | **ours**: `NativeStateBoard` in `src/main.cpp`. The window is a `struct gles_native_window` on `board_window(W, H)`; `flip()` is `board_show()`. |
| EGL + GLES through dlopen/dlsym and glad | `gl-state-egl.cpp:38-49` (`load_proc`), `:326-331` (`dlopen` libEGL), `:665` (libGLESv2) | **ours**: `include/dlfcn.h` + `src/platform.c`. `dlsym` is YakoGL's `eglGetProcAddress`, which answers for every EGL and GL entry point (`src/gl/egl.rs:3272`). |
| `EGLNativeWindowType` | glad's `EGL/eglplatform.h` has no Zephyr arm | YakoGL's `include/` comes first, so glad uses YakoGL's `eglplatform.h` (`struct gles_native_window *`). |
| The canvas | `canvas-generic.cpp` (CanvasGeneric over a NativeState and a GLState) | upstream, **unchanged**, with upstream's `GLStateEGL`. |
| Data files: `get_resource` (`std::ifstream`), `list_files` (`std::filesystem::directory_iterator`) | `libmatrix/util.cc:233-256` | **patched** (`patches/libmatrix-util-zephyr.patch`): an istream over the linked-in bytes (in place, seekable) and the linked-in paths, on the model of upstream's own ANDROID arm (`util.cc:257-324`). |
| CPU count, process times, idle time | `util.cc:326-381` (`sysconf`, `getrusage`, `/proc/uptime`) | patched: `arch_num_cpus()`; process times are NaN (not measured), so a `--results cpu` line prints `nan`, never a made-up zero; idle time 0.0, as upstream returns when `/proc/uptime` is missing. |
| `getopt_long`, `optarg` | `options.cpp:26,282` | Zephyr's `CONFIG_POSIX_C_LIB_EXT` + `CONFIG_GETOPT_LONG`. `include/getopt.h` adds `<unistd.h>` for `optarg`, as glibc's getopt.h does. |
| `isatty(fileno(stdout))` (colour or not) | `libmatrix/log.cc:126,149,169,189` | Zephyr has no `isatty` with picolibc. `src/platform.c` answers "not a terminal", so the UART log has no colour codes. |
| `M_PI` | `scene-build.cpp:175`, `model.cpp:282`, ... | `_GNU_SOURCE`, as g++ predefines on Linux. Zephyr's `_POSIX_C_SOURCE=200809L` alone hides picolibc's XSI names. |
| PNG/JPEG decoding | `image-reader.cpp` (libpng, libjpeg-turbo) | upstream's bundled `src/libpng`, `src/zlib`, `src/libjpeg-turbo` (the file lists of their `meson.build`). zlib gets `Z_HAVE_UNISTD_H`, as its configure sets on POSIX. |

Nothing a scene uses is stubbed. `shared-library.cpp`, `options.cpp`,
`results-file.cpp` and `text-renderer.cpp` are compiled as they are.
`--results-file` needs a file system and is refused. The show-fps and title
decorations draw extra text over every frame, which the published runs do
not have, so they are refused too.

### The scenes (`default-benchmarks.h:44-76`: 33 entries, 10 s each, `scene.cpp:49`)

| Scene (entries) | What it stresses | Data files |
|---|---|---|
| build (2: `use-vbo=false/true`) | vertex throughput, client arrays against VBOs | `models/horse.3ds` |
| texture (3: nearest, linear, mipmap) | texture filtering | `models/cube.3ds`, `textures/crate-base.png` |
| shading (4: gouraud, blinn-phong-inf, phong, cel) | per-vertex and per-pixel lighting | `models/cat.3ds` |
| bump (3: high-poly, normals, height) | normal and height maps against geometry | `models/asteroid-{low,high}.3ds`, `textures/asteroid-*.png` |
| effect2d (2: 3x3 and 5x5 kernels) | fragment convolution over the whole window | `textures/effect-2d.png` |
| pulsar (1) | blending, many small quads | none (`texture=false`) |
| desktop (2: blur, shadow) | render-to-texture composition passes | `textures/desktop-*.png`, `effect-2d.png` |
| buffer (3: map / subdata, interleave) | buffer-object updates | none. **Two of the three need GL_OES_mapbuffer** (`scene-buffer.cpp:330-337`). |
| ideas (1) | many draw calls, a classic animated scene | none (data in `scene-ideas/*.cc`) |
| jellyfish (1) | vertex animation, 32 caustics textures, blending | `models/jellyfish.jobj`, `textures/jellyfish*.png` |
| terrain (1) | a multi-pass render-to-texture chain, vertex texture fetch (`scene-terrain.cpp:238-250`) | `textures/terrain-*.jpg` |
| shadow (1) | a depth-texture shadow map (needs GL_OES_depth_texture, `scene-shadow.cpp:479-486`) | `models/horse.3ds` |
| refract (1) | two FBO passes, a depth texture (`scene-refract.cpp:92-99`) | `models/bunny.obj`, `textures/nasa1.png` |
| conditionals, function, loop (3 + 2 + 3) | branches, function calls and loops in shaders | none |

**Static gaps in YakoGL that decide part of the score** (read from our tree,
not measured):
- GL_OES_mapbuffer: fixed. It is advertised and `glMapBufferOES` exists
  (land183, 985077e1; gl_test case 176 mapbuf passed on all three boards).
  Before that, `buffer:...update-method=map` (2 of 33 entries) reported
  "Unsupported" on every board.
- GL_OES_depth_texture: advertised with `REPORT_ES2` on every GPU
  (`DEPTH_TEXTURE_EXTENSIONS`, `src/gl/state.rs`). Until v3d/depth-texture
  the V3D did not claim it, so shadow and refract (2 of 33) were
  "Unsupported" on the Pi 4. gl_test case 169 dstex passed on the Pi 4 on
  2026-10-04 (`tools/cts/RESULTS.md`). glmark2's shadow also needs a
  depth-only framebuffer on the V3D (`DRAWS_WITHOUT_COLOUR`, case 177).
- terrain needs vertex texture units. YakoGL reports 16
  (`src/backend.rs:220-252`), except on the GC7000 when its `vs_units16`
  switch is off.

### The score

- Per scene, `Scene::average_fps()` is frames / elapsed seconds, truncated to
  an unsigned integer (`scene.cpp:139-142`). The clock starts after the
  scene's `load()` and `setup()` (`scene.cpp:246-251`), so set-up (shader
  compiles, texture decoding) is not timed.
- The score is the integer mean of those values **over the scenes whose
  set-up succeeded** (`main-loop.cpp:115-120`, `:53-58`). A scene that is
  unsupported or fails is left out of both the sum and the count, so a run
  with fewer scenes can score higher. Our summary line prints the counts;
  the run's verdict (`board_finish`) is a pass only when no scene failed or
  was unsupported and the score is not 0.
- The printed per-scene FPS is `ceil(1 / average frame time)`
  (`main-loop.cpp:178`), which rounds the other way from the score's
  truncation.

### What a run prints (UART on the AM62P and the Pi 4)

```text
glmark2: upstream 22c527cb... (2023.01), size 800x600, args ""
=======================================================
    glmark2 2023.01
=======================================================
    <canvas.print_info(): GL_VENDOR, GL_RENDERER, GL_VERSION, surface config, size>
=======================================================
[build] use-vbo=false: FPS: <n> FrameTime: <ms> ms        (one line per scene, upstream's)
...
=======================================================
                                  glmark2 Score: <n>
=======================================================
glmark2: score <n>: <r> scenes ran, <f> failed, <u> unsupported (33 list entries); window 800x600, shown yes, GPU resets <g>
glmark2: PASS | glmark2: FAIL rc=<rc>                      (board_finish)
yk-record: 594b02.. ...                                   (kind APP: the verdict)
```

## 2. The port

```text
driver/samples/zephyr/glmark2/
  CMakeLists.txt     checks GLMARK2_SRC is the pinned commit with no local change;
                     builds upstream's common sources + glad + gl-state-egl and the
                     three image libraries as static libraries (upstream's warnings);
                     links the 134 data files with .incbin (glmark2_assets.S)
  Kconfig            GLMARK2_SIZE (default 800x600), GLMARK2_ARGS (default empty:
                     the default list)
  prj.conf           C++ runtime, POSIX getopt, online shader compiler, 1 MiB stack,
                     96 MiB heap, watchdog lifetime 900 s
  boards/            per board, as slint/boards (all three boards)
  patches/libmatrix-util-zephyr.patch
  include/           dlfcn.h, getopt.h, unistd.h, glmark2_platform.h
  src/main.cpp       NativeStateBoard, the run, the outcome counts, board_finish
  src/platform.c     dlopen/dlsym, the data-file table, isatty
```

Build (the Verdin AM62P):

```sh
git clone https://github.com/glmark2/glmark2 build/glmark2-upstream
git -C build/glmark2-upstream checkout 22c527cb0556f3a1ac4445aaa52cc532760928d5
west build -p always -d <ws>/build/gm2-am62p -b verdin_am62p/am62p54/a53 \
    driver/samples/zephyr/glmark2 -- \
    "-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh <ws>)" \
    -DGLMARK2_SRC=$PWD/build/glmark2-upstream
RUN_BOUND=960 tools/board.sh am62p run <ws>/build/gm2-am62p/zephyr/zephyr.bin <logdir> glmark2
```

`RUN_BOUND=960`: the default list is 33 x 10 s plus set-up (shader compiles
take about 0.2 s each on the A53, `deqp/README.md`), about 6-7 minutes, past
the runner's default 420 s bound.

### Why 800x600 on every board

- It is glmark2's own default (`options.cpp:37`), and the size of the
  published Pi 4 and i.MX8MP runs that give a size (section 3).
- It fits all three panels (1024x600, 1280x800, 1920x720), so the run is
  on-screen everywhere at the same pixel count.
- The common "--fullscreen 1920x1080" runs cannot be on-screen here (no panel
  has 1080 lines). For those, `GLMARK2_SIZE="1920x1080"` with
  `GLMARK2_ARGS="--off-screen"` renders into FBOs of that size (the AM62P
  window may be up to 8100 KiB, `boards/verdin_am62p_am62p54_a53.conf`).
  Off-screen is a different measurement from on-screen (section 3).
- Compare per pixel when the sizes differ (Mpx/s = fps x W x H / 10^6).

### The data files, per board

All 134 files of `data/shaders`, `data/models` and `data/textures`:
9,420,183 bytes, linked in as they are. Decoding them offline into raw
pixels was considered and rejected. The PNG textures decode to 21.9 MB and
the three JPEGs to 2.4 MB: 24.3 MB, about 4.6 times the 5.28 MB of compressed textures. Upstream's
own readers would also no longer be the ones running. Decoding happens in a
scene's set-up, which is not timed (section 1, the score).

| Board | Image (zephyr.bin) | RAM footprint | Status |
|---|---|---|---|
| Verdin AM62P | 12,755,176 B (sha256 `0ec6383ca936f803...` at e0eacc3e) | 156,311,552 B (incl. the 96 MiB heap, the 8100 KiB window buffer, the 1 MiB stack) | built, links |
| Verdin iMX8M Plus | 12,256,088 B (sha256 `e8f4aa37562c67b7...` at e0eacc3e) | 156,192 KiB of the 256 MiB RAM region | built, links (no UART: the GLMARK2 record carries the numbers) |
| Raspberry Pi 4 | 12,325,040 B (sha256 `8f72f7f52a98182c...` at e0eacc3e) | 194,192 KiB of the 224 MiB region at the 48 MiB carve-out; the carve-out is now 96 MiB (refract's 56.2 MB peak, terrain's NPOT chains) and the region 288 MiB (`boards/rpi_4b.overlay`, ends at 0x12200000, below the firmware's 0x3c100000), not rebuilt since; kernel VM 512 MiB as v3d_deqp | built, links; `board.sh --dry-run pi4 run` rc 0 |

The whole set fits each board's RAM. The Pi 4's ~12 MB of free space was
for an image that also carries the cluster demo; this image does not, and
its region is 288 MiB (the decode buffers come from the 96 MiB heap inside
it). The Pi 4's boot partition must hold the 12.3 MB image (check `df`
before staging). If a
smaller set is ever needed, the default list does not use
`textures/nasa2.png` and `nasa3.png` (1.04 MB).

Beside the 96 MiB carve-out, the Pi 4's glmark2 image raises the V3D's
tiled-copy budget to 24 MiB
(`CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES=25165824` in `boards/rpi_4b.conf`;
the Kconfig default stays 8 MiB for the other boards). refract's 1600x1200
RGBA8 colour render target tiles to 11,001,856 B with its mip chain, over
the 8 MiB default, so one of its draws was refused (3 x GL error 0x505).
Measured on the host harness at the 96 MiB carve-out: 24 MiB clears the
refusal and keeps the 7,684,096 B depth copy beside it (refract peak
69,093,376 B of the carve-out, 0 refused allocations across the whole
list; 16 MiB also clears it but evicts the depth copy every frame and peaks
higher, at 70,146,048 B; both run refract at 23 FPS, so the choice rests on
memory, not speed) - `tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md` sections
3.2-3.4. The budget is a workaround for the per-frame CPU tiled copy of a
render target: item `v3d-rt-sample` (it owns `src/v3d/**`) may sample the
render target without the copy at all, and then the Pi 4 board file can
drop this line and go back to the 8 MiB default.

### The record: kind 18 GLMARK2 (`record.rs`)

The i.MX8MP has no UART and its SNVS keeps only the last record, so the run
ends with one record that is also its verdict, stored after the summary
line and kept by `board_finish`:

- w1 = glmark2's score;
- w2 = flags << 28 | status (1 pass, 2 fail) << 24 | scenes ran << 16 |
  failed << 8 | unsupported (each count saturating at 255; flag
  `GLMARK2_OFFSCREEN` = 1);
- w3 = W << 16 | H.

`yakogl-tools record 594b1203 000002b9 021f0002 03200258` prints
`GLMARK2 (18) seq 3: score 697 status 2 (fail) scenes ran 31 failed 0 unsupported 2 size 800x600 on-screen`.
A run that fails before the benchmark loop ends with `board_finish`'s APP
record instead. Per-scene numbers on the i.MX8MP would need pages chosen by
the selector, as DEQP_FAILS does; not done.

## 3. Public scores (opened 2026-10-03)

Reference only: these are Linux stacks with the vendor or Mesa driver, under
the conditions each row states. They are not our measurement and not a
comparison we make (the website quotes them the same way). The first runs of
our image are `tools/cts/log/benchmarks-baseline/glmark2-first-runs-2026-10.md`.

Every row was read from the page itself. Rows marked (*) were also re-opened
by the author of this file, and the quoted lines matched.

### Raspberry Pi 4 (V3D 4.2, Mesa v3d)

| Score | Command | Size | On/off-screen | Window system | Driver | OS | Date | Source |
|---|---|---|---|---|---|---|---|---|
| 697 (*) | glmark2-es2, flags not stated; "Surface Size: 800x600 windowed" | 800x600 | on-screen (windowed) | not stated | Mesa 23.2.1-1~bpo12+rpt3, GL ES 3.1, glmark2 2023.01 | "Debian GNU/Linux 11 (bullseye) aarch64" as printed (the Mesa string is a bookworm backport) | 2023-01 | https://github.com/geerlingguy/sbc-reviews/issues/4 |
| 450 | `glmark2-es2-drm --off-screen --size 800x600` | 800x600 | off-screen | DRM | Mesa 19.3.2 / 20.1-devel (the thread mixes them) | Raspbian / Ubuntu 64-bit | 2020-04-14 | https://forums.raspberrypi.com/viewtopic.php?t=270951 |
| 168; 256 / 155 | `--off-screen --size 800x600 --visual-config 'red=8:green=8:blue=8:alpha=8:buffer=0'` (256 = performance governor, 155 = ondemand) | 800x600 | off-screen | DRM | Mesa 19.3.2 | Raspbian | 2020-05 | same thread |
| 69 / 50 | `--size 800x600 --visual-config ...` | 800x600 | on-screen | DRM (50: a 60 Hz TV) | Mesa 19.3.2 | Raspbian | 2020-05 | same thread |

### Verdin iMX8M Plus / i.MX8M Plus (Vivante GC7000UL)

| Score | Command | Size | On/off-screen | Window system | Driver | OS | Date | Source |
|---|---|---|---|---|---|---|---|---|
| 382 | `glmark2-es2-wayland --off-screen` (raw log) | 800x600 | off-screen | Wayland | Vivante galcore 6.4.11.p2.745085, GL ES 3.1, glmark2 2023.01 | not stated (Phytec phyBOARD-Pollux) | 2025 (NXP thread) | https://community.nxp.com/pwmxy87654/attachments/pwmxy87654/imx-graphics/784/2/glmark_vivante.txt |
| 953 | `glmark2-es2-wayland -s 640x480` | 640x480 (HDMI 1920x1080 display) | on-screen | Wayland | NXP vendor stack, version not stated | Toradex BSP 5.2.0 | not stated | https://developer.toradex.com/hardware/hardware-resources/general-product-information/2d3d-graphics-acceleration-gpu/ |
| 992 / 987 | `glmark2-es2-wayland -s 640x480` | 640x480 | on-screen | Wayland | GL ES 3.1, version not stated | Variscite BSP 5.4.70 (DART / VAR-SOM-MX8M-PLUS), performance governor | 2022-08-25 | https://variscite.com/blog/compare-the-differences-between-the-nxp-i-mx8-graphic-accelerators/ |

### Verdin AM62P / AM62P (PowerVR BXS-4-64)

| Score | Command | Size | On/off-screen | Window system | Driver | OS | Date | Source |
|---|---|---|---|---|---|---|---|---|
| 306.4 (min 306, max 307) (*) | "Glmark2-DRM", command not stated | not stated | not stated | DRM | not stated | TI Processor SDK Linux 12.01.00, AM62Px SK rev E1, A53 1.4 GHz, LPDDR4 3200 MT/s | not stated | https://software-dl.ti.com/processor-sdk-linux/esd/AM62PX/latest/exports/docs/devices/AM62PX/linux/Linux_Performance_Guide.html (Table 2.15) |
| 776.4 (min 765, max 786) (*) | "Glmark2-Wayland", command not stated | not stated | not stated | Wayland | not stated | same | not stated | same |
| 298 / 744 | `glmark2-es2-drm --off-screen` / `glmark2-es2-wayland --off-screen` | not stated | off-screen | DRM / Wayland | not stated | TI SDK 9.02 | 2024 (SPRADG0A) | https://www.ti.com/lit/pdf/spradg0 (section 4.1) |

Not opened, so not used: OpenBenchmarking result pages (403), and two
i.MX8MP search snippets (335 and 283). AM62x numbers (a different GPU,
AXE-1-16M) are left out.

### Comparability caveats

- **Size.** 800x600, 640x480 and "not stated" all occur. The score is not
  per pixel; compare at the same `--size`, or in Mpx/s.
- **Off-screen and on-screen differ by large factors** (Pi 4 forum, one
  setup: 168 off-screen, 69 on-screen DRM). Compare like with like and say
  which one each number is. Ours is on-screen by default.
- **The window system changes the score on one GPU**: TI's DRM and Wayland
  numbers differ by 2.5x on the same board. Our run has no compositor: the
  frame goes from `eglSwapBuffers` to `board_show` (a DSS flip on the AM62P,
  a CPU copy on the Pi 4, the LCDIF on the i.MX8MP).
- **Vsync.** glmark2 asks `eglSwapInterval(0)` (`gl-state-egl.cpp:427`). Most
  sources do not say whether it took effect. YakoGL has no vsync wait in
  `eglSwapBuffers`, but whether `board_show` waits for the display is not
  measured.
- **CPU and governor** change the score (Variscite: up to 40% with the core
  count; the Pi 4 forum: 256 against 155 with the governor). Zephyr runs one
  A53 here.
- **Driver and glmark2 versions** are old or not stated in most rows. Only
  the 697 (Mesa 23.2.1) and the 382 (galcore 6.4.11) give both.
- **The score leaves unsupported scenes out** (section 1). A source that
  lost a scene scores higher. Our run prints the counts. With the mapbuffer
  gap it would lose 2 of 33 on every board, and 2 more on the Pi 4.
