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

### What a run prints (UART on the AM62P and the Pi 4; the i.MX8MP: section 2, "Per scene without a console")

After each scene's line the sample prints `glmark2-frame:` (the cost per
frame), `glmark2-picture:` (the picture value, below) and `glmark2-mem:`.

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

### The picture value (`glmark2-picture:`)

A score says how fast a scene ran, not that it drew the right picture. For
every scene that ran, one line gives what its last frame looked like, so two
boards (or a board and the host) can be compared per scene:

```text
glmark2-picture: <scene info string>: 800x600; probes c RRGGBBAA, tl RRGGBBAA, tr RRGGBBAA, bl RRGGBBAA, br RRGGBBAA; crc16 XXXX, crc16q XXXX; mean R G B; black yes|no
glmark2-picture: <scene info string>: absent (the readback failed)
```

- **Probes.** Five pixels, RGBA as 8 hex digits (R first): `c` the centre
  (400,300), `tl` (200,150), `tr` (600,150), `bl` (200,450), `br` (600,450);
  the size is the window's. Alpha depends on the window format (XRGB gives a
  constant), so judge R, G and B.
- **crc16** is CRC-16/CCITT-FALSE (poly 0x1021, init 0xffff) over the RGB
  bytes of a 40x30 grid: the centre of each 20x20 cell, row by row from the
  top, left to right. It is exact: any one-bit difference changes it. Equal
  crc16 means the same picture on that grid.
- **crc16q** is the same with each channel shifted right by 4. It survives
  rounding differences of up to a few levels between GPUs, but not a pixel
  that moves across a quantisation step; treat it as "probably the same".
- **mean** is the mean R, G, B (0..255) over the grid. It is the value to
  compare with a tolerance when the crcs differ (for example within 8 levels
  per channel); the probes serve the same purpose pixel by pixel.
- **black** is `yes` when every probe and grid pixel has R = G = B = 0: a
  scene that drew nothing. A real glmark2 scene is never all black, so
  `black yes` on a board means a lost picture. The host harness
  (`tools/hostharness/glmark2`) renders nothing, so it prints `black yes`
  with `crc16` of an all-zero grid: that is its expected value, not a
  failure.

The frame is read with `glReadPixels` after the scene's last `update()` and
before its last `eglSwapBuffers`, so the pixels are the frame the scene
ended on (the back buffer holds the next frame after the swap). The
scene's own clock stops in `update()`, so its FPS is unchanged, and the
readback's cycles are taken out of the `glmark2-frame:` line's `frame` total.
The readback waits for the last frame's render, so that frame's `swap`
(which would have waited) is shorter by that wait: one frame in N of the
`glmark2-frame:` means, outside the score. The code is in `driver/samples/zephyr/glmark2/src/main.cpp`
(`capture_picture`) and, copied, in `tools/hostharness/glmark2/host_main.cpp`,
which prints the same line on stderr from the same probe and grid code;
change both together. The host reads the XRGB window buffer it gave EGL
instead of calling `glReadPixels` (after `glFinish`): its non-V3D back ends
map the window without a CPU view, so `glReadPixels` there raises
`GL_OUT_OF_MEMORY` (the gate's GL-error check, `glmark2_corpus`, fails on it).
A board whose EGL does not double-buffer the window and maps it without a CPU
view would print `absent (the readback failed)` and count one GL error per
scene: the first board run must show it.

### The frame dump (`CONFIG_GLMARK2_FRAME_DUMP`, a picture for a page)

Where the console is the only way out (the AM62P has no network and no Linux
during the run), a scene's last frame can be sent over it as text. Off by
default and off in every gate and common-set image; set it with
`CONFIG_GLMARK2_FRAME_DUMP=y` and name one scene in `GLMARK2_ARGS` (for
example `-b shading:shading=phong`, the horse), because each scene prints its
whole window. At the picture value's place (after the scene's last `update()`,
before its last swap, and only when the readback worked) the image prints

```text
glmark2-fb: <scene info string> 800x600 begin
glmark2-fb: <y> <crc16 of the row's RGB, hex> <base64 of the row's RGB888>   (y = 0 is the top row)
glmark2-fb: end crc32 <hex>
```

800x600 is about 1.9 MB of text: a few minutes at 115200 baud, and the scene
waits for it (the readback is outside the score either way). Every row has its
number and a CRC-16/CCITT-FALSE (the picture value's), and the end line has
the CRC-32 of all rows, so a lossy UART costs the rows it touched and says
which. `tools/cts/glmark2-fb.py <console log> <out.png> [--scene S]` writes the
PNG (damaged or missing rows magenta), lists those rows and exits 1; rerun the
board then, pixels are never patched. With every row good it checks the crc32,
then recomputes the probes, crc16, crc16q and mean of the scene's
`glmark2-picture:` line from the dumped pixels: the picture is what the
picture value described. `glmark2-fb.py --self-test` (run by
`tools/cts/cts-selftest.sh`, check 26) decodes a synthetic log.

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
memory, not speed) - `tools/hostharness/glmark2/FINDINGS-refract-budget.md` sections
3.2-3.4. The budget was a workaround for the per-frame CPU tiled copy of a
render target. Since item `v3d-rt-sample` a colour render target the scene
samples is stored by the tile list into a render image the TMU reads as it
is, and its glGenerateMipmap (refract's and terrain's, every frame) is one
TFU job in that image, so the image also sets `CONFIG_YAKOGL_V3D_TFU=y`
(the OSAL runs TFU jobs only with it; the option's default stays `n`
elsewhere). The copies of the render targets are gone from the second
frame of a scene on; the budget still covers the uploaded textures and the
first frame's copies, and can go back to the 8 MiB default once a board run
has read the "render images" and "tiled copies high-water" lines at this
setting (`docs/online-shaderc/v3d-backend.md`, "Render targets sampled from
their render image").

### The record: kind 18 GLMARK2 (`record.rs`)

The i.MX8MP has no UART and its SNVS keeps only the last record, so the run
ends with one record that is also its verdict, stored after the summary
line and kept by `board_finish`. `record.rs` (the `GLMARK2` row of `KINDS`,
`glmark2_words`) is the definition; today's images write this layout (flags
`GLMARK2_SCENES` and `GLMARK2_GL`, plus `GLMARK2_OFFSCREEN` = 1 off-screen):

- w1 = score (16 bits, saturating) | the first failed entry << 16 | why it
  failed (`GLMARK2_WHY_*`) << 22 | the first unsupported entry << 26;
- w2 = flags << 28 | status (1 pass, 2 fail) << 24 | scenes ran << 18 |
  failed << 12 | unsupported << 6 | the second unsupported entry (counts
  saturate at 63; an entry is its index in the benchmark list, 63 none);
- w3 = W (12 bits) | H << 12 (12 bits) | GL errors raised << 24 | draws that
  sampled an incomplete texture << 28.

The last two are the GL core's counters for the whole run (issue #13):
`yakogl_gl_errors_raised` (`src/gl/context.rs`: every GL error an entry
point raised, the ones the console's `GL error 0x.. in gl..` lines name) and
`yakogl_incomplete_texture_draws` (`src/gl/draw.rs`: every draw whose
program sampled an incomplete texture, which reads (0,0,0,1)). Each has 4
bits and saturates at 15, which reads "15 or more". They are not a verdict:
a run with GL errors still passes when no scene failed. glmark2 never calls
`glGetError`, so the errors the context kept (`yakogl_gl_errors_recorded`,
at most one without a `glGetError`) are printed on the console line only:

```text
glmark2: GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)
```

The kind-18 words had no free bit, so the size shares w3 with the counters;
a window wider or taller than 4095 drops `GLMARK2_GL` and keeps the full
size, and the decoder then says the counters were not recorded (never 0).
Records from images before 2026-10-12 decode the same way ("not recorded"),
and those before 2026-10-04 (no `GLMARK2_SCENES`) as w1 = score,
w2 = flags << 28 | status << 24 | ran << 16 | failed << 8 | unsupported,
w3 = W << 16 | H.

`yakogl-tools record 594b120e 44960027 62781093 fb258320` (a synthetic
outcome, `record.rs`'s packing example) prints
`GLMARK2 (18) seq 14: score 39 status 2 (fail) scenes ran 30 failed 1 unsupported 2 size 800x600 on-screen; first failed: entry 22 (default list: terrain), why 2 (fragment shader); unsupported: entry 17 (...), entry 19 (...); GL errors raised 11, draws that sampled an incomplete texture 15 or more`.
A run that fails before the benchmark loop ends with `board_finish`'s APP
record instead. Per-scene numbers on the i.MX8MP come from page runs (kind
24 GM_PAGE, below). The host harness prints the two
counters per scene (`tools/hostharness/glmark2`, the `gm2: scene-end` line)
and checks them against the library's own lines.

### The first GL errors by name: kind 23 GL_ERROR (issue #814)

The GL core keeps the first eight GL errors with the entry point that
raised them and a tag the client sets (`src/gl/context.rs`,
`yakogl_gl_error_tag`, `yakogl_gl_error_first`); the sample sets the tag to
the benchmark-list entry at each entry's setup (255 before the first). At
the end it prints one line per kept error:

```text
glmark2: GL error 1 of 37: 0x505 in glTexImage2D, entry 22
```

With the selector `GL_ERROR_SEL | k` (0x800 | k) in the SNVS it also stores
error k as a kind 23 GL_ERROR record after the GLMARK2 verdict, which then
is the run's last record: the only way the i.MX8MP shows which calls failed.
One boot per error: `SEL=$((0x800)) tools/board.sh imx8mp run <GM image>
<logdir> gle0`, then `0x801`, ... `yakogl-tools record` decodes it and names
the entry point from the GL and EGL headers (w2 is the FNV-1a of its name):

```text
GL_ERROR (23) tag 0: error 0 of 8 kept: 0x505 (GL_OUT_OF_MEMORY) in the entry point with FNV-1a 0xebeea496, tag 22 (glmark2: entry 22 (default list: terrain)); GL errors raised 37
  entry point: glTexImage2D
```

(a synthetic record, `tools/testdata/record-decode.txt`). w3 is the whole
count of errors raised, 32 bits, where GLMARK2's field stops at 15.

### Per scene without a console: kind 24 GM_PAGE, kind 25 RATE

The i.MX8MP keeps one record of 96 payload bits a boot, so its per-scene
FPS and picture values travel in pages, one page a boot, chosen by the
selector (the design: `docs/notes/imx8mp-gm-record.md`). With the selector
`GM_SEL | t << 6 | n` (`GM_SEL` = 0x400) the image cuts its benchmark list
to the page's entries, runs them, stores GLMARK2 as always and then the page
as the run's last record. `record.rs` (`gm_page_words`, `GmPage`,
`write_gm_lines`) is the definition:

| t | page | entries run | w1 | w2 | w3 |
|---|---|---|---|---|---|
| 0 | F page n (0..10) | 3n, 3n+1, 3n+2 | scene 3n: fps \| frames << 16 | scene 3n+1: the same | scene 3n+2: the same |
| 1 | P0 of scene n (0..32) | n | probe c \| tl << 16 | tr \| bl << 16 | br \| crc16 << 16 |
| 2 | P1 of scene n | n | crc16q \| fps << 16 | frames \| mean R << 16 \| mean G << 24 | mean B \| black << 8 \| W << 9 (11 bits) \| H << 20 (12 bits) |

- **fps** is the integer of upstream's result line (`ceil(1 / mean frame
  time)`, computed from the same `Scene::stats()`), saturating at 0xfffd;
  0xfffe is "set up failed" and 0xffff "unsupported", both with frames 0.
  **frames** is the scene's frame count, saturating at 0xffff.
- **The probes** are carried at RGB565 and printed widened by shift
  (`R5 << 3`, `G6 << 2`, `B5 << 3`), alpha `00`: the floor of the probe's 565
  step. A real change of one step moves R or B by 8 and G by 4, within the
  table's tolerance 8.
- **crc16, crc16q, mean, black** are the console's values unchanged. W = H
  = 0 in P1 (and P0 all 0) is `absent (the readback failed)`.
- **Dropped** against the console: alpha; the low bits of each probe
  channel (565); the console's `FrameTime` (the decode prints `FrameTime`
  as 1000 / FPS, three decimals; `common-tally.py` reads `FrameTime` when the
  line has it and 1000 / FPS otherwise, so for a pages file the two agree); the `glmark2-frame:` line's `gl`, `swap`, `show`, `other`,
  `frame` and GPU-job means (the decode says `not carried (GM_PAGE)`, so the
  table's `gl ms` is `-`); the `glmark2-mem:` lines; the score and GL error
  counters of a page run (its GLMARK2 record is replaced by the page: they
  stay the full run's).

Kind 25 RATE was the cluster sample's frame rate, one boot of the cluster
image with the selector `GM_SEL | 3 << 6` (0x4c0): tag = the run's status,
w1 = the frames presented, w2 = their milliseconds, w3 = W << 16 | H. The
cluster sample was removed on 2026-10-19 (a demo, not a benchmark), so no
image writes it now; `gm-lines` still decodes an old `cl-rate` run into a
`throughput:` line.

`tools/board.sh imx8mp gm <image> <outdir> fps|pictures` runs the
pages (11 boots for `fps`, 66 for `pictures`; resumable,
`GM_ROWS=22` for one page or scene), then `tools/board.sh imx8mp gm-lines
<outdir>` decodes every `gm-f*`, `gm-p*` and `cl-rate` run's `snvs.txt`
(`yakogl-tools record glmark2-pages`) into `<outdir>/glmark2-pages.txt`:

```text
glmark2-pages: decoded from kind GM_PAGE records: F pages 0-8, 10 (missing 9); P0 pages of scenes 0, 21-22 (missing ...); P1 pages of scenes 0, 21-22, 25 (missing ...)
[terrain] <default>: FPS: 6 FrameTime: 166.667 ms
glmark2-frame: [terrain] <default>: 61 frames; us per frame: not carried (GM_PAGE)
glmark2-picture: [terrain] <default>: 800x600; probes c 6080a000, tl 80a0c000, tr 30507000, bl 18283800, br c0c0c000; crc16 51e4, crc16q 0c3b; mean 92 118 140; black no
[shadow] <default>: Set up failed
glmark2-picture: [conditionals] fragment-steps=0:vertex-steps=0: absent (page P0 of scene 25 missing)
throughput: 1280x800: 3600 frames in 60012 ms, 60.0 fps, 61.4 Mpx/s (kind RATE, status pass)
```

(from the synthetic page session `driver/rust/tests/gm-record`, which the
record tests write and check). The lines are a console's as far as
`tools/cts/common-tally.py` is concerned: `--gm-table`, `--gm-diff`,
`--gm-pictures` and `--gm-picture-lines` take the file in place of a
console. A picture needs both P pages; with one missing the line says which,
and the tally judges it `absent`. A page's scene runs first in a fresh boot
(cold caches and GPU clock), so compare page runs with page runs; the score
stays the full run's.

### The frame breakdown on the GC7000: kind 12 PERF

Where one i.MX8MP frame's time goes, one page a boot (the design and the
step-3 board command: `docs/notes/imx8mp-frame-floor.md`, "The
frame-breakdown record"). Only in a GC7000 build (`CONFIG_YAKOGL_DRV_GPU_VIVANTE`);
the AM62P and Pi 4 images do not compile it. With the selector
`PERF_SEL | page` (`PERF_SEL` = 0x1000; experiment 0, page 0..3: 0x1000,
0x1001, 0x1002, 0x1003), read once before the benchmark loop, the image runs
its list as always, counts each scene's frames for that page, prints one
`glmark2-perf: <scene>: page p: w0 w1 w2 w3` line per scene, stores GLMARK2
and then the last scene's page as the run's last record. So it is meant
for an image of one scene (`CONFIG_GLMARK2_ARGS="-b <scene>"`). Another
`0x10xx` selector (experiment 1 and up, or page 4 to 7) is refused: the run
ends at once with `-EINVAL`. Without a PERF selector nothing of it runs.

w0 = `0x594b0c00 | page`; w1..w3 = six 16-bit values, v0 in w1's low half
(`w1 = v1 << 16 | v0`, `w2 = v3 << 16 | v2`, `w3 = v5 << 16 | v4`);
`yakogl-tools record` prints them as `experiment 0 page p: v0 .. v5`. Each
value saturates at 0xffff; means are rounded down; a mean with no count is 0.
The packing is `driver/samples/zephyr/glmark2/src/gm_perf_page.h`, checked
on the host by `driver/samples/zephyr/glmark2/host/run.sh` (known totals to
known words, then each page decoded by `yakogl-tools record`).

| Page | v0 | v1 | v2 | v3 | v4 | v5 |
|---|---|---|---|---|---|---|
| 0: the frame (us per frame) | frames | frame (t2 - t0) | gl (t1 - t0) | swap | show | frame to frame |
| 1: the swap (per frame) | wait us | waits per 100 frames | `gc_render_wait` sleeps per 100 frames | GPU busy us | kicks per 100 frames | submit us |
| 2: the display | intervals under 1.5 T | in [1.5 T, 2.5 T) | at 2.5 T or more | T, us (0: not measured) | frames with present >= 1000 us | `CONFIG_SYS_CLOCK_TICKS_PER_SEC` |
| 3: the GPU (per interval) | HI total kcycles | HI idle kcycles | KiB read | KiB written | core MHz | 0 |

- **t0, t1, t2** are the `glmark2-frame:` line's stamps (`RecordingLoop::draw`);
  frame, gl, swap and show are that line's means, the picture readback left
  out as there. Under EGL's double buffer (the default) the window is shown
  by the swap, so show is about 0 and the show's time is in swap.
- **Frame to frame** is the time from one frame's t0 to the next, so it
  holds the time outside `draw()` too. A scene of N frames has N - 1
  intervals; page 2's bins and page 3's means are over the intervals.
- **Page 1** is the difference over the scene's frames of `osal_gc_wait_stats`
  (wait us, waits), `gc_render_perf` (sleeps) and `osal_gc_gpu_stats`
  (GPU busy, kicks, submit us), less what the last frame's picture readback
  added.
- **Page 2**: T is measured after the scene's last frame, by four calls of
  the board layer's VSYNC wait (`yakogl_zephyr_wait_vblank`), timed, the
  last three averaged. The bins put each interval at its 32 us histogram
  bucket's start, so an interval can count up to 32 us early. "Present" is
  the frame's swap plus show.
- **Page 3** reads `HI_PROFILE_TOTAL_CYCLES`, `_IDLE_CYCLES`, `_READ_BYTES8`
  and `_WRITE_BYTES8` before every frame's t0 (32-bit, they wrap in about
  5 s at 800 MHz) and adds the differences; bytes = BYTES8 x 8; core MHz =
  total cycles / frame-to-frame us.

## 3. Public scores (opened 2026-10-03)

Reference only: these are Linux stacks with the vendor or Mesa driver, under
the conditions each row states. They are not our measurement and not a
comparison we make (the website quotes them the same way). The first runs of
our image are `ab43a20c5:tools/cts/log/benchmarks-baseline/glmark2-first-runs-2026-10.md`.

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
