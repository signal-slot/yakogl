<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: the slow jobs' render setup and texture state against Mesa's

Study, no code change (plan #329, item `v3d-blur-jobs-config`; #1209). The
item's question: are terrain's blur jobs slow (7.9 and 9.4 ms of render,
`pi4-terrain-board-jobs.md` and `pi4-slow-terrain.md`, "Board runs") because
of something in how the job is set up, apart from the shader code? That
covers the render control list, the tile setup, the texture and sampler
words, the stores, loads and clears, and the cache operations. Each is
compared with what Mesa's v3d driver emits for the same draw. Shader
instruction, thread and TMU counts are `v3d-terrain-shaders.md`'s
(v3d-terrain-shader-study) and are not repeated here.

Tree read: yakogl `410f3a6e` (the integration tip). `driver/src/v3d`,
`src/osal/zephyr_v3d.c` and `driver/samples/zephyr/glmark2` are the same on
origin/main `0ea865ce` (`git diff --stat` empty). glmark2 is the pinned
source in `$CONNECTED_DEPS_DIR/glmark2-src`. **Mesa**: the vendored
`third_party/mesa-26.2.1` holds only the packet XML, the format table and
the common tiling code, not the gallium driver. The gallium v3d and v3dv
sources were read from a full Mesa 26.2.1 tree in the dependency cache
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, the same release that
`third_party/mesa-26.2.1/UPSTREAM` names). Mesa line numbers below are that
tree's. Each claim says where it comes from: **[tree]** is read in yakogl,
**[mesa]** is read in that Mesa tree, **[knowledge]** is my own
recollection with no file in either tree. No board was run. The host
harness (`tools/hostharness/pi4-terrain/run.sh`) was **not** run either: the
words below are derived from the code that emits them, not from a dump.

## Answer in short

1. **Jobs 2 and 4 of the frame-10 line are not the blur passes.** The
   `glmark2-jobs` line lists the jobs that the swap completed. It starts
   with the previous frame's last job (`main.cpp:1401-1403` [tree]). The
   TFU chains behind each entry fix the order (below). **Entry 2 (render
   7931 us) is the height-map noise pass** (256x256, `terrain-noise.frag`,
   no texture). **Entry 4 (render 9374 us, bin 3529 us, an 800x600 chain
   behind it) is the terrain draw.** The two tilt-shift blurs are **entry 8
   (tilt-shift H, render 2801 us)** and **entry 1 (tilt-shift V into the
   window, render 4436 us `~`**, an upper bound because the OSAL saw its end
   late).
2. **The blur jobs are set up as Mesa sets them up.** The tile size, the
   bpp, MSAA and double-buffer choice, the supertile layout, the texture and
   sampler words, the colour store format and the cache invalidates all
   match. Tilt-shift H at 2.8 ms for 9 taps over 480,000 pixels
   (0.65 ns a pixel-tap) runs at or above the sampling rate that gl_test
   case 220 measured on the same board (`pi4-slow-terrain.md`, case 220:
   512x512, one tap, 0.24 ms, about 0.92 ns a sample). No job-setup change
   can make the blurs much faster.
3. **The 7.9 ms noise job has no texture state and no depth buffer.** Its
   setup is a 16-tile single-target job and matches Mesa's. Its time is
   fragment ALU work: about 1,900 instruction slots a pixel at 500 MHz over
   8 QPUs, which fits four calls of a 3-D simplex noise (below). That is the
   shader study's question, not a setup question.
4. **Differences from Mesa that do exist**, each small (ranked under "Fix
   candidates"):
   - **depth kept as D24S8 for a `DEPTH_COMPONENT16` renderbuffer**, where
     Mesa keeps Z16 (half the bytes);
   - early Z always off;
   - a per-tile clear in jobs that do not clear;
   - an L2T clean after every job;
   - Mesa's Vulkan driver (not its GL driver) turns on double-buffer mode
     for light-geometry jobs.

   The largest is the depth format, **about 0.4 ms a frame**, on the overlay
   job (entry 7, 1754 us). That job is bound by tile-buffer memory traffic:
   7.68 MB in 1.75 ms, the same 4.4 MB/ms rate as the TFU.

## Which entry is which job

glmark2's frame (`scene-terrain.cpp:332-339`, `init_renderers` `:59-125`
[tree, glmark2]) is: height map (noise) → normal map → terrain → bloom H →
bloom V → overlay → tilt-shift H → tilt-shift V (window). That is 8 jobs a
frame since the dead window clear went (`pi4-slow-terrain.md`, "Third run").
The frame-10 line of `a4b9c551` (`pi4-terrain-board-jobs.md`):

| entry | render us | bin us | tfu behind | job | target |
|---:|---:|---:|---|---|---|
| 1 | 4436 `~` | 42 | - | tilt-shift V, previous frame | window 800x600 |
| 2 | **7931** | 9 | - | **height map, `terrain-noise.frag`** | 256x256 |
| 3 | 191 | 6 | - | normal map | 256x256 |
| 4 | **9374** | **3529** | 1/805 (800x600 chain) | **terrain draw** (393,216 vertices) | terrain target 800x600 + depth |
| 5 | 460 | 7 | 1/113 (256 chain) | bloom H | 256x256 |
| 6 | 287 | 7 | 1/113 (256 chain) | bloom V | 256x256 |
| 7 | 1754 | 36 | 1/795 (800x600 chain) | overlay (blend, depth test, loads colour and depth) | terrain target |
| 8 | **2801** | 36 | 1/806 (800x600 chain) | **tilt-shift H** | 800x600 |

The proof is the order of the chains. Five chains are built a frame, in
the order terrain, bloom H, bloom V, overlay, tilt-shift H
(`v3d-terrain-gpu.md`, "The TFU chains"). The 805, 113, 113, 795 and 806 us
TFU times fit only entries 4 to 8. Two more facts agree. Entry 4 has the
only large bin, and only the terrain draw has 131,072 triangles to bin.
Entry 2 has `idle` 80 and `prep` 1: it is the first job after a swap that
waited for every job. `v3d-terrain-gpu.md` predicted noise at about
0.5 ms. The board's 7.9 ms is the largest miss in that model.

**Caveat for every note that says "jobs 2 and 4 are the blurs"**
(`pi4-terrain-board-jobs.md`, `pi4-slow-terrain.md` "What is left": "the
second and fourth entries ... 74 % of the render sum"): the two big entries
are the noise pass and the terrain draw. The blurs are 2.8 ms and at most
4.4 ms.

## The words of the blur jobs, ours against Mesa's

Ours are what `v3d_job_begin` and `v3d_job_end`
(`driver/src/v3d/v3d_job.c:324-377`, `:1323-1482`), `emit_tile_list`
(`:1145-1321`) and `v3d_tex_pack_state` and `pack_sampler`
(`driver/src/v3d/v3d_tex.c:778-884`) emit for a single-sampled RGBA8 target
of 800x600 that one draw clears and fills [tree]. Mesa's are what
`v3dX(start_binning)` (`v3dx_draw.c:45-101`), `v3dX(emit_rcl)` and
`emit_render_layer` (`v3dx_rcl.c:566-765`), `v3d_rcl_emit_stores`
(`:240-342`) and the format table (`v3dx_format_table.c:65`) emit [mesa].

### Binning (BCL prologue)

| field | ours | Mesa | effect |
|---|---|---|---|
| TILE_BINNING_MODE_CFG width/height | 800 x 600 | draw size, 800 x 600 | - |
| render targets, max bpp | 1, 32 (enum 0) | `nr_cbufs` 1, `internal_bpp` 32 | same tile size |
| 4x MSAA | 0 | 0 | - |
| initial tile-alloc block / block | 128 B / 64 B (`v3d_job.c:24`, `:372-373`) | 128 B / 64 B (`v3d_limits.h:94-99`, `v3dx_draw.c:78-81`) | - |
| FLUSH_VCD_CACHE, OQ counter, START_TILE_BINNING | yes | yes | - |
| VCM_CACHE_SIZE | the compiled shaders' `vcm_cache_size` (`v3d_job.c:1022-1025`) | `vpm_cfg.Vc` from the same value (`v3dx_draw.c:868-871`, `vir.c:937`) | shader-side |

### Rendering mode (RCL)

| field | ours | Mesa | effect |
|---|---|---|---|
| tile size | 64 x 64, 13 x 10 = 130 tiles (`v3d_frame.c:62-106`, port of `v3d_choose_tile_size`) | the same table | - |
| supertiles | 1 x 1 tiles, frame 13 x 10 (same loop, `v3d_frame.c:108-125`) | `v3dx_rcl.c:580-617` | - |
| supertile order | raster order, all 130 (`v3d_job.c:1474-1481`) | raster order inside the draw bounds (`v3dx_rcl.c:670-697`, "XXX perf: ... Morton order") | full-screen draw: same |
| COMMON `double_buffer_in_non_ms_mode` | 0 | 0 in gallium unless `V3D_DEBUG=db` (`v3d_job.c:400`, `v3d_debug.c:95`) | same; **v3dv differs, below** |
| COMMON `early_z_disable` | 1 always (`v3d_job.c:1361`) | 1 for a job with no depth buffer (`v3dx_rcl.c:745-750`) | blurs have no depth: same |
| COMMON `internal_depth_type` | DEPTH_24 always (`:1362`) | 0 (left unset) without a zsbuf (`:720-727`) | no depth buffer: no effect [knowledge: the TLB's Z memory is fixed size, the type only changes packing] |
| COLOR rt0 | bpp 32, type 8, clamp as `rt_internal` | `v3d_setup_render_target` of RGBA8 | - |
| CLEAR_COLORS PART1 | the clear word | the same | - |
| initial dummy tiles | 2, the clear in the first (`:1454-1467`) | 2 (GFXH-1742), the clear in the first unless double-buffered (`v3dx_rcl.c:638-657`) | - |
| FLUSH_VCD_CACHE before the generic list | yes | yes | - |

### Generic tile list (once per tile, 130 times)

| step | ours | Mesa | effect |
|---|---|---|---|
| loads | none (the job cleared) | none | - |
| store, tilt-shift H (FBO target, sampled next) | **one**: RT0 into the render image, UIF (XOR or not, from the layout), RGBA8, sample 0 (`:1260-1269`; the raster store is left out by `uif_only`, `:1254`) | one: RT0 into the texture's level 0 in its own tiling (`store_general`, `v3dx_rcl.c:120-168`) | same bytes (1.92 MB) |
| store, tilt-shift V (window) | one raster store into the window surface, R/B swap as the surface needs (`:1133-1141`) | one store into the back buffer in its tiling. **[knowledge]** a scanout back buffer on the Pi 4 is linear (the HVS does not scan out UIF), so it is raster too | same bytes |
| CLEAR_TILE_BUFFERS | **always** (`:1314-1317`) | only when `job->clear_tlb` (`v3dx_rcl.c:330-336`) | blurs clear: same. The overlay (entry 7) does not clear, and we still clear its tiles. See candidate 3 |
| END_OF_TILE, RETURN | yes | yes | - |

### Texture and sampler words of the sampled target

Tilt-shift H samples the terrain target after the overlay (chain B).
Tilt-shift V samples tilt-shift H's target (chain C). Both are RGBA8 render
images with a full chain of 10 levels, `GL_LINEAR_MIPMAP_LINEAR` and
`GL_CLAMP_TO_EDGE` (`scene-terrain.cpp:107-117`).

| field | ours (`v3d_tex.c:605-609`, `:739-826`, `:833-884`) | Mesa (`v3dx_format_table.c:65`, `v3dx_state.c` texture and sampler state) | effect |
|---|---|---|---|
| texture type | RGBA8 | RGBA8 (`R8G8B8A8_UNORM` row) | - |
| return size | 16 (two 16-bit halves a word, `Output Type 32-bit` 0) | 16 (the row's `16`) | same TMU return width |
| swizzle | R, G, B, A | XYZW | - |
| base/max level | 0 / 9 | the view's first/last level | - |
| level 0 UIF, XOR, `ub_pad`, `extended` | from `v3d_tex_layout`, a port of `v3d_setup_slices` | the same | - |
| array stride | level 0 end / 64 | `cube_map_stride` / 64 | - |
| min/mag/mip filter | linear / linear / linear | linear / linear / linear | the same TMU work; case 220 measured trilinear at lod 0 as free (ratio 1.00) |
| min/max LOD, bias | 0 / clamp(1000, 0, 15) = 15 / 0, u4.8 and s8.8 as `v3d_upload_sampler_state_variant` | the same | - |
| anisotropy, sRGB | off, no | off, no | - |
| wrap | clamp to edge | clamp to edge | - |
| sampled as | the render image the TLB stored, which the TFU then mipmaps (`v3d-terrain-copies.md`; `rt.direct` 7.976 a frame) | the texture itself, mipmapped by the TFU (`v3dx_tfu.c`) | same memory: no copy on either side |

### Between jobs: cache operations

| point | ours | Linux + Mesa | effect |
|---|---|---|---|
| before the bin | L2T flush, slice caches invalidated (`v3d_hw.c:366-375`, `:413`) | `v3d_invalidate_caches` before the bin job **[knowledge, `drivers/gpu/drm/v3d/v3d_sched.c`; not in either tree]** | same |
| before the render | the same again (`:427`) | the same, with the kernel comment about render-to-texture between bin and render **[knowledge]** | same |
| after the render | `v3d_caches_clean`: TMU write-combiner flush, then L2T clean, each waited for (`v3d_hw.c:377-389`, `zephyr_v3d.c:1302`) | **no clean after a render job.** The kernel queues `v3d_clean_caches` only for compute jobs **[knowledge]** | see candidate 4 |
| TFU | after the clean, serial (`zephyr_v3d.c:1309-1313`) | the TFU job is scheduled after the render it depends on | same order |

## The two slow jobs' setup

**Entry 2, height map (7.9 ms).** 256x256 is 4 x 4 = 16 tiles of 64x64.
It has one RT, no depth buffer, no texture, one quad, and is cleared and
stored once into the render image (0.26 MB). Every setup word matches the
blur table above. There is no texture state and no depth state to differ.
Its time cannot be memory traffic: 0.26 MB at 4.4 MB/ms is 0.06 ms. At
500 MHz (`v3d-terrain-gpu.md`, "The TMU rate is an assumption", for the
clock), 7.9 ms is 3.95 M clocks x 8 QPUs x 4 lanes a clock over 65,536
pixels, which is about **1,930 instruction slots a pixel**. That fits four
calls of glmark2's 3-D simplex noise (`terrain-noise.frag:26-98`, about 70
vector statements each, mostly vec3/vec4, issued one component at a time).
**[knowledge]** for the 4 lanes a clock and the component-at-a-time issue.
The number that settles it is the compiled instruction count of
`terrain-noise.frag`, which is `v3d-terrain-shaders.md`'s. One setup aspect
belongs with that count: the fragment shader's thread mode in the shader
record (4, 2 or 1 threads), which decides how much TMU or SFU latency is
hidden. It is the shader study's number.

**Entry 4, terrain draw (9.4 ms).** It differs from Mesa in two words:

- `early_z_disable` 1 where Mesa would set 0 (LEQUAL, depth write, no
  `discard`: `v3dx_rcl.c:729-744` [mesa], `v3d-early-z.md`). The measured
  overdraw is 1.030 (`terrain-overdraw.md`), so the gain is
  4.3 ms x 0.03, about **0.13 ms**.
- **The depth buffer is D24S8, 4 bytes a pixel.** The tile list loads and
  stores it as `D24S8` (`v3d_job.c:1235-1244`, `:1280-1290`) and the common
  config says `DEPTH_24` (`:1362`). glmark2's renderbuffer is
  `DEPTH_COMPONENT16` (`base-renderer.cpp:149`). Mesa gives a Z16 surface
  `internal_depth_type` DEPTH_16 (`v3d_formats.c:193`) and stores and loads
  it as `D16` (`v3dx_format_table.c:153`, `store_general` output format
  `v3d_get_rt_format`, `v3dx_rcl.c:142-147`), Z only (`zs_buffer_from_pipe_bits`,
  `:173-185`: no stencil bit for a depth-only surface): **2 bytes a pixel.**
  Entry 4 stores 1.92 MB of depth where Mesa stores 0.96 MB. Entry 7 loads
  and stores it again: 3.84 MB against 1.92 MB.

The rest of entry 4's 9.4 ms is vertex work (393,216 vertices, not indexed,
each shaded twice, at bin and at render, with 2 vertex texture fetches,
`terrain.vert:40,45`) and fragment work (9 bilinear samples a fragment).
Both are shader-side. The 3.5 ms bin is not render time and is
`pi4-terrain-board-jobs.md`'s.

## The overlay job is the one memory-bound job

Entry 7 (overlay, 1754 us) has a light shader: one fetch of bloom V at
magnification, blended. Its tile traffic is a colour load from the render
image, a depth load, a colour store and a depth store: 4 x 1.92 MB =
**7.68 MB, which is 4.38 MB per ms**. That is the same rate the TFU gives
(4.47 MB/ms, `v3d-terrain-gpu.md`, "Rate"), so the job runs at memory
speed. It is the one job where the depth format shows directly. The other
800x600 jobs are bound by sampling or ALU, and their stores are hidden
under the shading.

## Fix candidates, ranked (job setup only)

Expected gains are ms a frame against the 34.7 ms frame and the
`a4b9c551` record. Each change lies in `driver/src/v3d/v3d_job.[ch]` and
`src/v3d` (held by plans #308 and #321; no code was changed here).

| rank | candidate | expected gain | confirming value (`glmark2-jobs`, frame 10) | certainty |
|---|---|---:|---|---|
| 1 | **Keep a `DEPTH_COMPONENT16` depth buffer as D16, Z only**, as Mesa does: `internal_depth_type` DEPTH_16, load and store format `D16`, buffer `Z`, half-size allocation | **0.4 ms** (overlay: 1.92 MB less at 4.4 MB/ms), plus up to 0.2 ms hidden in entry 4 | **entry 7 `render` 1754 to about 1320 us**; entry 4 `render` 9374 unchanged or slightly lower | rate: measured on two jobs. Picture: depth precision of a 16-bit buffer is what the application asked for. Needs the D16 path in the TLB load/store and in `ds_plane` and `zskeep71` users, which assume D24S8 today |
| 2 | Early Z for the terrain draw (`v3d-early-z.md`) | 0.13 ms | entry 4 `render` down about 130 us: inside the run-to-run spread, so not measurable in one run | high (overdraw measured), gain too small to see |
| 3 | No per-tile CLEAR_TILE_BUFFERS in a job that loads every buffer (overlay), as Mesa (`clear_tlb` only) | about 0 **[knowledge: a TLB clear is a few clocks a tile; 130 tiles]** | entry 7 `render` (no visible change expected) | low value; keep only for parity |
| 4 | No L2T clean after a render job that wrote nothing through the TMU (Linux does none) | under 0.05 ms **[knowledge: a clean with no dirty lines is short]**; 8 a frame | the OSAL's `clean` us per job (`osal_v3d_recent_job_us.clean`, **not printed** by `print_jobs`) | low. Needs care: a job with image stores or transform feedback still needs it |
| 5 | Double-buffer mode for light-geometry 800x600 jobs. Mesa's **Vulkan** driver turns it on when the score allows (`v3dv_cmd_buffer.c:545-551`, `v3d_double_buffer_score_ok`, `v3d_compiler.h:1631-1646`: geometry at most 200,000, render work at least 200). Mesa's **GL** driver only does so under `V3D_DEBUG=db` | 0 to about 0.9 ms upper bound (the colour store of tilt-shift H, tilt-shift V and noise, 0.43 + 0.43 + 0.06, hidden behind shading). It can also be negative: tiles halve to 64x32, so per-tile cost and binning double. The terrain draw is excluded (393,216 vertices > 200,000) | entries 8 and 1 `render` (2801 and the `~` 4436) each down up to 0.4 ms. The overlay must not change, because the score excludes it or it is memory-bound | low: Mesa's GL driver has it off by default, and no board number for it exists anywhere |

What **no** setup candidate can do: make the noise job (7.9 ms) or the
terrain draw (9.4 ms, apart from candidates 1 and 2) faster. Both are
shader work (`v3d-terrain-shaders.md`). The blurs (2.8 ms and at most
4.4 ms) are at the measured sampling rate.

## What was read from a tree and what from knowledge

- **Read in yakogl [tree]:** every "ours" word (`v3d_job.c`, `v3d_frame.c`,
  `v3d_tex.c`, `v3d_hw.c`, `zephyr_v3d.c`), the job-line format
  (`driver/samples/zephyr/glmark2/src/main.cpp:1390-1425`), the board
  figures (`pi4-terrain-board-jobs.md`, `pi4-slow-terrain.md`), and the
  glmark2 sources (`$CONNECTED_DEPS_DIR/glmark2-src`).
- **Read in Mesa 26.2.1 [mesa]** (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`,
  not the vendored tree, which lacks the driver): `v3dx_rcl.c` (stores,
  clear, dummy tiles, supertiles, COMMON config, early Z, double buffer),
  `v3dx_draw.c` (binning config, VCM), `v3d_job.c:400` (gallium double
  buffer gated by the debug flag), `v3d_debug.c:95`, `v3d_limits.h:88-99`,
  `v3dx_format_table.c:61-65`, `:153`, `v3d_formats.c:193`, `vir.c:937`,
  `v3d_compiler.h:1631-1646`, `v3dv_cmd_buffer.c:545-551` and `:1113`.
- **From knowledge, not in either tree:** the Linux kernel's cache
  operations around bin, render and compute jobs (`v3d_sched.c`); that the
  Pi 4 display scans out linear buffers, not UIF; that a QPU issues 4 lanes
  a clock and that vector GLSL is issued a component at a time; the cost of
  a TLB clear and of an L2T clean with nothing dirty; that the
  internal-depth-type field has no cost without a depth buffer. The 500 MHz
  clock is `v3d-terrain-gpu.md`'s assumption (the `board: V3D clock` line
  is not in the tree).

## Not done

- **No harness dump of the jobs' words.** `tools/hostharness/pi4-terrain/run.sh`
  with `GLMARK2_SRC` was not run in this session. The words above follow
  from the emitting code for this target shape. A dump would confirm the
  UIF/XOR choice of each render image and the shader-record thread mode,
  which this note leaves to the code and to the shader study.
- No board run (non-goal).
