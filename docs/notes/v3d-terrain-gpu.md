<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: where glmark2 terrain's GPU time goes on the Pi 4 once the CPU copies are gone (study)

Study, no code change (plan #329, item `v3d-terrain-gpu-study`; #1209).
`pi4-slow-terrain.md` finds that about 130 ms of terrain's 161.8 ms frame is
a render-image table that thrashes, and expects about 30 ms after the fix.
What is left at 30 ms is the GPU. This note splits that GPU time by job,
from the code and the existing notes, and ranks what would cut it.

Code read at main `d0f5813e` (2026-11-07). `src/v3d`, `driver/src/v3d`,
`src/osal/zephyr_v3d.c` and `src/gl/draw.rs` are the same as at `e2a4e86b`
(`git diff --stat` empty); line numbers are that tree's. glmark2 is the
pinned `22c527cb` (`$GLMARK2_SRC`, read; paths `src/...` and
`data/shaders/...` below are its). **Nothing ran on a board, and no host
harness run was possible in this session** (the unattended session was
refused `tools/hostharness/glmark2/run.sh`, as the earlier studies were), so
there is **no V3D instruction count of terrain's shaders** and no
`GM2_PROFILE` figure of its own here. The shader cost below comes from the
shader sources and sample counts, and says so where it matters.

## Answer in short

1. **The GPU work of a frame is two things, in this order.**
   Texture samples, then the TFU. Terrain makes 9 render jobs and, once the
   render-image table holds all its targets, 5 TFU chains. Counting bilinear
   samples from the shader sources gives **about 27 M a frame, 24 M of them
   in three jobs: the terrain draw (7.2 M) and the two 800x600 tilt-shift
   blurs (8.6 M each)**. At one bilinear sample a clock a slice (2 slices,
   500 MHz: an assumption, below) that is 27 ms, against the 27.3 ms the
   board measured for the render jobs (29.1 ms of frame 10 less 1.8 ms of TFU).
   The TFU is **2.6 ms** after the fix (5 chains, 11.8 MB of traffic at the
   rate the board's own TFU times give), and the TLB's stores and loads are
   **18.3 MB** a frame, about 4 ms if they were not hidden under the shading.
2. **Expected frame after the fix: GPU 29.9 ms (27.3 + 2.6), frame about
   30 to 31 ms**, GPU-bound as refract is. This agrees with
   `pi4-slow-terrain.md`'s 30 to 35.
3. **What would cut it, ranked** (details and the board values in "Fix
   candidates after the fix"):

   | rank | candidate | predicted gain a frame | certainty |
   |---|---|---:|---|
   | 1 | the blur passes sample one level (terrain reads their chains at level 0, `terrain-mip-usage.md`) | up to ~9 ms of samples, + 0.9 to 1.8 ms of TFU | the TFU part is bytes; the sample part needs the TMU to skip its second fetch, **not shown** |
   | 2 | skip the chains no draw reads below level 0 (TFU bytes alone) | 0.1 to 1.8 ms | high (bytes), picture risk by chain |
   | 3 | the window clear that is cleared again (job 0): 9 jobs to 8 | 0.5 to 0.8 ms | high |
   | 4 | depth stores/loads of the terrain target (5.8 MB) | at most 1.3 ms, likely under 0.5 | low: no safe lifetime rule |
   | - | early Z for the terrain draw (not asked; found) | up to ~1.7 ms | low: overdraw not measured |
   | - | levels the terrain draw does not sample (inside one chain) | about 0.1 ms | high: a chain's tail is geometric |

4. **The per-job times are not in the tree.** The image prints them
   (`glmark2-jobs:` line, `print_jobs`, `driver/samples/zephyr/glmark2/src/main.cpp:1255`)
   but the record that has them (`records/pi4/20261022-0308-glmark2-c4af4917/`)
   is gitignored and `pi4-glmark2-cpu.md:556` quotes only the sums. The
   split below is a model; the first thing the Pi 4 board session
   (`pi4-board-before`) should do is read the line and replace it (table
   "What the board line should show").

## What the 29.1 ms is, and is not

`pi4-glmark2-cpu.md:556`: "The GPU work of frame 10 is 29.1 ms (8 jobs; 4 TFU
jobs 1.8 ms of it)". Refract: "16.8 ms at frame 10, the third with a 3.2 ms TFU
job" (`:562`). Reading the OSAL's fields (`src/osal/zephyr_v3d.c`):

- `total` of a job is its submit to the moment the OSAL finished it, **queued
  TFU jobs included**: `finish_inflight` waits for FRDONE (`:1281`), cleans
  the caches (`:1304`), runs the TFU jobs queued behind the job
  (`tfu_queue_run`, `:1313`, defined `:1113`), then stamps `c_end` and
  `total` (`:1314-1316`). `render` ends at FRDONE (`:1285`); the TFU time of
  those jobs is `tfu K/Tus` of the same line (`note_job_done`, `:761-786`).
- A render whose FRDONE was already up when the OSAL looked has `render_late`
  set (`:1277`) and its `render` includes the CPU's time elsewhere. In frame
  10 of the record the CPU had drained the GPU before each of its long loops
  (`settle_texture`'s `finish`, `pi4-slow-terrain.md`, "CPU waits"), so the
  jobs after a loop are waited for at once and the 29.1 ms is close to real
  GPU time. It is still the **thrashing** frame: two of terrain's targets
  had no render image and stored raster (and, after a CPU filter, both,
  `raster_wanted`, `v3d-lazy-raster.md` "As built"). After the fix the
  stores differ by under 2 MB (about 0.4 ms), inside this note's error.
- "8 jobs" is the number of entries the ring held for that swap
  (`main.cpp:526-537`: the jobs the swap completed; the frame's last job
  completes at the next submit). The frame has **9** (host
  `v3d: jobs 9.00 a frame`, board 560 over 62): see job 0 below.
- The TFU is **serial** with the render: `osal_v3d_tfu` finishes everything
  first (`:1346`), and a queued chain runs only after its job's FRDONE and
  clean (`:1311-1313`). The hardware could overlap them; nothing in terrain
  could use it, because every chain's next reader is the very next job
  (below). So the frame is the sum of the parts, not their maximum.

## The frame's nine jobs

Order is the glmark2 call order (`terrain-mip-usage.md`, "One steady frame")
and the back end's job rule: a new target ends the open job
(`src/v3d/backend.rs:6089`). Tiles: RGBA8 is internal bpp 32, so 64x64 tiles
(`driver/src/v3d/v3d_frame.c:59-90`, `driver/src/v3d/v3d_job.c:272`);
800x600 is 13 x 10 = **130 tiles**, 256x256 is 4 x 4 = **16**. Bytes are
unpadded (w x h x 4); colour stores of the 6 FBO targets go to their render
images (UIF) and **only there** (`uif_only`, `v3d_job.c:1127`, the lazy
raster store of `v3d-lazy-raster.md`, because glmark2 reads no target on the
CPU), the window's go to its raster surface.

| job | target | what it draws (glmark2) | tiles | colour load | colour store | depth | TFU behind it |
|---|---|---|---:|---:|---:|---|---|
| 0 | window 800x600 | **nothing**: the benchmark loop's `canvas_.clear()` (`src/main-loop.cpp:132`) flushed by the next glClear | 130 | - | 1.92 MB raster | cleared, not stored | - |
| 1 | height map 256x256 | `terrain-noise.frag`: 4 calls of `snoise`, no texture | 16 | - | 0.26 MB | none | - |
| 2 | normal map 256x256 | `terrain-normalmap.frag`: 3 fetches of the height map | 16 | - | 0.26 MB | none | - |
| 3 | terrain target 800x600 | `terrain.vert`/`.frag`: 393,216 vertices (131,072 triangles, not indexed), 2 vertex fetches, 5 fragment fetches | 130 | - | 1.92 MB | stored 1.92 MB | chain A, 800x600 |
| 4 | bloom H 256x256 | `terrain-blur.frag`, radius 2: 5 taps of the terrain target | 16 | - | 0.26 MB | none | chain, 256x256 |
| 5 | bloom V 256x256 | the same, 5 taps of bloom H | 16 | - | 0.26 MB | none | chain, 256x256 |
| 6 | terrain target again | `terrain-overlay.frag`: 1 fetch of bloom V, blended, **no clear** | 130 | **1.92 MB** (image) | 1.92 MB | **loaded 1.92 MB, stored 1.92 MB** | chain B, 800x600 |
| 7 | tilt-shift H 800x600 | blur, radius 4: 9 taps of the terrain target | 130 | - | 1.92 MB | none | chain C, 800x600 |
| 8 | window | tilt-shift V: 9 taps of tilt-shift H | 130 | - | 1.92 MB raster | cleared, not stored (the swap discards it, `v3d_surface_swapped`, `backend.rs:4858-4866`) | - |

Sources: `scene-terrain.cpp:59-125` (sizes, radii, which renderers have a
depth renderbuffer: only the terrain target, `setup_offscreen(screen_res, true)`),
`scene-terrain/base-renderer.cpp:64-81` (depth test is on for the window and
for a target with a depth renderbuffer) and `:143-167`,
`texture-renderer.cpp:53-69` (clear, draw, mipmap), `overlay-renderer.cpp:72-90`
(no clear), `terrain-renderer.cpp:43-57`, `blur-renderer.cpp:87` (taps
`2 * radius + 1`). Stores and loads: `emit_tile_list`, `v3d_job.c:1009-1212`;
which depth is stored: `flush_job`, `backend.rs:2887-2898`
(`zs_store = touched && !zs_discard`; `split_zs` is off by default, `:1393`).

**Job 0 is inferred, not seen.** The GL core emits a pending clear of one
framebuffer when a glClear of another arrives (`settle_pending_clear`,
`src/gl/draw.rs:2523-2540`, from `pending_clear_for`, `:2655`). The window's
`canvas_.clear()` is that pending clear when terrain's first FBO is cleared
(the host trace of `terrain-mip-usage.md` shows `clear fb 0` first). On the
V3D a clear with no draw opens a job on the window (`backend.rs:6124-6176`),
and the next draw into another target ends it (`:6089`). Job 8's own
glClear (every `TextureRenderer::render` clears) makes that first clear
dead. This accounts for 9 jobs where the record's ring shows 8, but it is the
same defect the PowerVR item `pvr-dead-clear-kick` fixes from the GL core
(a full pending clear stays pending across a bind), and the record's line
(first entry: a job with `render` of a few hundred us and no TFU) shows it.

Per frame: colour stores **10.65 MB** (5 x 1.92 MB at 800x600 + 4 x 0.26 MB),
depth stores **3.84 MB** (jobs 3 and 6), loads **3.84 MB** (job 6: colour and
depth). TLB traffic **18.3 MB**.

## The TFU chains: bytes and time

Layouts from `v3d_tex_layout` (`driver/src/v3d/v3d_tex.c:175-320`, a port of
Mesa's `v3d_setup_slices`; a scratch Python port reproduces both totals the
tree already records, 2,780,160 B for 800x600 x 10 levels and 11,001,856 B
for 1600x1200 x 11, `pi4-slow-terrain.md` and `FINDINGS-refract-budget.md`
section 2):

| chain | levels | L0 | levels 1.. | all levels | TFU reads (L0..Ln-2) | TFU writes (L1..) | traffic |
|---|---:|---:|---:|---:|---:|---:|---:|
| terrain/tilt target 800x600 | 10 | 1,920,000 | 855,424 | 2,775,424 | 2,775,360 | 855,424 | **3.63 MB** |
| bloom 256x256 | 9 | 262,144 | 87,488 | 349,632 | 349,568 | 87,488 | **0.44 MB** |
| refract 1600x1200 | 11 | 7,680,000 | 3,318,144 | 10,998,144 | 10,998,080 | 3,318,144 | **14.32 MB** |
| (one-level 256x256 map, `tfu_tile`) | 1 | 262,144 | 0 | 262,144 | 262,144 | 262,144 | 0.52 MB |

(Level sizes of the 800x600 chain: L1 UIF 416x304 505,856 B, L2 256x256
262,144, then 65,536, 16,384, 4,096, 1,024, 256 and two of 64. The page
alignment the layout adds, 4,736 B, is left out here.) The job packs one TFU
job a chain (`generate_mipmap`, `backend.rs:2330-2439`; queued behind the
job that stored the image, `:2395`; `tfu_tile`, `:4010`, refuses a chain).

**Rate.** Refract's chain is the only TFU time on the board in the tree:
3.2 ms for 14.32 MB, **4.47 MB per ms** (the 3.2 is the record's rounding,
about 2 percent). Applied to the thrashing frame of the record, which made
two 800x600 chains, one 256x256 chain and one one-level tiling (3 chains
and 1 tiling, `pi4-slow-terrain.md`): 3.63 + 3.63 + 0.44 + 0.52 = 8.22 MB,
**1.84 ms against the record's 1.8 ms**. That is one parameter fitted on one
point and checked on a second, with a different mix; it is good evidence for
bytes, not for the exact rate.

After the fix (5 chains: three 800x600, two 256x256): 3 x 3.63 + 2 x 0.44 =
11.78 MB, **2.64 ms, 5 TFU jobs** (the host counts the same 5 a frame,
`pi4-slow-terrain.md` section 1). That is 9 percent of a 30 ms frame.

Where the chains sit, so that nothing can overlap: chain A feeds job 4 (bloom
H samples the target at lod 1.64), chain B feeds job 7, chain C feeds job 8,
the bloom chains feed jobs 5 and 6. The next job after each reads the chain
it follows (`terrain-mip-usage.md`, trace), so a TFU that ran beside the next
render would race its reader; `v3d-tfu-order.md` section 1 keeps the order.

## Shader work: samples

No V3D compile of terrain's shaders was possible here. What the sources fix
is the **fetch count**: a `texture2D` with a mipmap-linear sampler is two
bilinear samples (two levels) unless the hardware skips the second; the
terrain target and the blur sources are `GL_LINEAR_MIPMAP_LINEAR` at every
sampling draw (`terrain-mip-usage.md`, "(b)"). A pixel count is the target's;
the terrain draw shades more (no early Z, below).

| job | fetches | pixels or vertices | bilinear samples |
|---|---|---|---:|
| 1 noise | 0 (ALU: 4 `snoise`) | 65,536 | 0 (ALU about 0.5 ms, model: 500 instructions, 8 QPUs) |
| 2 normal map | 3 x LINEAR | 65,536 | 0.20 M |
| 3 terrain, fragment | 4 mipmap-linear (`tDetail`, `tDiffuse1`, `tDiffuse2`, `tSpecular`) + 1 LINEAR (`tDisplacement`) = 9 | about 0.62 M (480,000 x overdraw 1.3, assumed) | 5.6 M |
| 3 terrain, vertex | 2 LINEAR (`terrain.vert:40,45`), run twice: the coordinate shader at binning and the vertex shader at render | 393,216 | 1.6 M |
| 4 bloom H | 5 mipmap-linear | 65,536 | 0.66 M |
| 5 bloom V | 5 | 65,536 | 0.66 M |
| 6 overlay | 1 (magnified, so one level in the spec; the TMU may still fetch two) | 480,000 | 0.5 to 1.0 M |
| 7 tilt-shift H | 9 mipmap-linear | 480,000 | **8.6 M** |
| 8 tilt-shift V | 9 | 480,000 | **8.6 M** |
| **frame** | | | **26.5 to 27.0 M** |

Jobs 7 and 8 are 64 percent of the samples, the terrain draw 27 percent.
Terrain covers the whole 800x600 window (all four corner probes of the
`pi4` picture are terrain colours, `pi4-glmark2-cpu.md:371`), so a pixel
count of 480,000 is right for the target and the overdraw is the unknown.

**The TMU rate is an assumption.** The Pi 4's V3D is 4.2.14 with 2 slices and
8 QPUs (`driver/docs/rpi4-v3d.md:163`, `pi4-v3d-bringup.md:226`), clocked at
500 MHz under Linux; the board image raises it to the firmware's maximum at
start (`driver/zephyr/lib/board/rpi4/board_rpi4.c:172-177`, which prints
`board: V3D clock <from> -> <to> Hz`; the printed value is in the console
record and is not in the tree). One bilinear sample a clock a slice is my
recollection of V3D 4.x and no document of this repository states it. With it
the 26.5 to 27.0 M samples take 26.5 to 27 ms, which matches the 27.3 ms
measured for the render jobs. The match is why the sample count leads the
ranking. It is one number against one number; if the real rate is two a clock
the samples are half the render time and the other half is bin, setup,
no-early-Z shading and per-job cost that this note cannot place.

## The per-job split (model), and what the board line should show

Model time of a job is its samples at 1 ns, or the TLB bytes at 4.47 MB/ms
when that is larger (the two overlap on the GPU; they are not added), plus
the TFU that follows it. Terrain's two 800x600 stores are small beside the
blur samples, so only jobs 0 and 1 are not sample-bound.

| job | model render ms | TFU ms behind it | what the `glmark2-jobs` entry should read | what decides it |
|---|---:|---:|---|---|
| 0 | 0.4 (1.92 MB store) | - | `render` a few hundred us, no `tfu` | the dead clear |
| 1 | 0.5 (ALU) | - | `render` about 0.5 | the noise shader's instructions |
| 2 | 0.2 | - | `render` about 0.2 | |
| 3 | 7.2 | 0.81 (chain A) | `render` about 7, `tfu 1/810us` | samples, overdraw, binning of 131,072 triangles |
| 4 | 0.7 | 0.10 | `render` about 0.7, `tfu 1/100us` | |
| 5 | 0.7 | 0.10 | the same | |
| 6 | 0.5 to 1.0 | 0.81 (chain B) | `render` about 1, `tfu 1/810us` | the TMU's second fetch when magnified |
| 7 | 8.6 | 0.81 (chain C) | `render` about 8.6, `tfu 1/810us` | samples |
| 8 | 8.6 | - | `render` about 8.6 | samples |
| sum | 27.4 to 27.9 | 2.64 | `total` about 30 | measured: 27.3 + 1.8 in the thrashing frame |

(The `tfu` of an entry is the TFU time run since the previous job completed,
which is the chains queued behind that entry's job: `note_job_done`,
`zephyr_v3d.c:761-786`, with `tfu_queue_run` before it at `:1313`.)

The two checks that do not depend on the TMU rate: jobs 7 and 8 together
must be the largest part of `render` (if job 3 is, the terrain draw's
binning or overdraw is the floor, not the samples), and the `tfu` times must
read 0.8 ms for an 800x600 chain and 0.1 for a bloom chain.

**The swap column.** With two jobs outstanding at most
(`flush_job`, `backend.rs:2918-2923`; `wait_queued`, `:3043`), a flush waits
for the job two back, and the swap's `finish_previous` waits for the older of
the last two (`:6258-6264`). In a GPU-bound frame the CPU is blocked at
every flush, so the waits fall in the GL calls that end a job. Terrain has 9
waits a frame, one per job (`wait_job`, `:3051`; host and board both 9). The
swap column should read about job 7 plus its chain (9 ms), and the GL column
the rest, about 20 ms: **unlike refract (swap 15.4 of 17.27 ms, 3 jobs)**.
That is not a defect.

## Fix candidates after the fix

Gains are ms a frame against the model's 29.9 GPU ms, and are predictions.
"Board value" is a record line the Pi 4 session reads; all of them exist in
the image today unless marked.

### 1. The blur passes read one level

`terrain-mip-usage.md` shows that of the five chains only the first (job 4's
source) is read below level 1 (lod 1.64); the sources of jobs 5, 6, 7 and 8
are read at lod 0 up to a skew of 0.0001 to 0.019. Two kinds of saving:

- **The chains.** tex9 (job 6's source, magnified: lod -1.64) and the
  terrain target's second build (job 7's source, lod up to 0.00013, a weight
  that rounds to 0 at 1/256) are exact skips: chains B and the bloom V chain,
  0.81 + 0.10 = **0.9 ms** of TFU. The other two (bloom H's chain, tilt-shift
  H's chain) have a level-1 weight up to 1.1 and 1.9 percent (at most 4.9 per
  channel): chains C and bloom H, another **0.9 ms**, not exact. Total
  1.8 ms of the 2.64.
- **The samples.** If a draw that reaches level 0 only is sampled with the
  base level (`v3d_tex.c:849-861` already clamps the LOD for a non-mipmap
  min filter: `mip_none`), jobs 7 and 8 drop from 18 to 9 samples a pixel:
  **up to 8.6 ms** (4.3 each); job 6, 0.5. This is the large number and the
  least proven: it needs the TMU to fetch one level, not two, with the LOD
  clamped. **Not shown anywhere in the tree.** A gl_test case that times a
  1:1 full-window quad twice, with `GL_LINEAR` and `GL_LINEAR_MIPMAP_LINEAR`
  on a chained texture, answers it on the board in one run without touching
  terrain.

What it needs in code is the sampler LOD bound of `terrain-mip-usage.md`
("Candidate 1, the bound and the order of work"; it is in `src/shaderc`,
`src/gl/draw.rs`, `src/gl/texture.rs`, held by #325, #310, #308). Risk: the
level-1 weight, a picture within the tolerance of
`tools/cts/glmark2-pictures.txt`.

**Board value that confirms it:** `glmark2-jobs:` entries of jobs 7 and 8,
`render` 8.6 to about 4.3 ms; the `TFU mipmap at frame N` line's `chains` a
frame 5 to 3 (exact set) or 1 to 2 (all); `tfu K/Tus` of the entries.

### 2. TFU chain bytes alone

The 2.64 ms of TFU is 9 percent of the frame. Chains it does not need are
the same set as candidate 1. **Levels inside one chain** do not matter: for
the terrain target the levels above 2 are 0.09 MB of 3.63 (2.5 percent), so
stopping the first build at level 2 saves **about 0.1 ms** and is not worth
a code path. Nor can the TFU run beside a render (above). Gain without the
sample part: **0.1 to 1.8 ms**. Board value: `tfu` time of the entries
(0.81 ms an 800x600 chain, 0.10 a bloom chain), `TFU mipmap` line `chains`.

### 3. Job merging: the window's dead clear

The only pair of jobs that can be merged. Job 0 is a full pending clear of
the window that job 8 clears again (`texture-renderer.cpp:57`), so it is
dead. Dropping it removes one job (a 130-tile clear and a 1.92 MB raster
store, 0.4 ms at the TLB rate) and its fixed cost (the submit, tile-state
zeroing, binner start, cache clean: not measured, `idle` and `clean` of the
record's entries give it). Gain **0.5 to 0.8 ms**. It is the GL-core
fix of `pvr-dead-clear-kick` (a full pending clear stays pending across a
bind to another framebuffer, `src/gl/draw.rs:2523`); it needs no V3D code.

The other pairs cannot merge. Jobs 3 and 6 draw into the same target but the
two blurs between them read the target (their chains) and one writes what
job 6 blends in. Jobs 1/2 and 4/5 are separate targets, and each reads the
one before. **Board value:** the jobs a frame 9.0 to 8.0 (`glmark2-frame`
line, `jobs`), the `glmark2-jobs` line with 8 entries, no first entry of
a few hundred us.

### 4. Depth and stencil stores of the terrain target

The terrain target's depth renderbuffer (`DEPTH_COMPONENT16`,
`base-renderer.cpp:149`) is never sampled and never read on the CPU, and is
stored at job 3, loaded and stored at job 6: **5.76 MB**, 31 percent of the
TLB's 18.3 MB, **1.3 ms** at the TFU's rate if the TLB were the bound (it is
hidden under the shading of 130-tile jobs that are sample-bound, so likely
under 0.5 ms). Job 3's store feeds job 6's depth test (the overlay is
depth-tested, `base-renderer.cpp:73-76`); only job 6's store (1.92 MB, 0.43
ms) is dead, because the next job on the target (job 3 of the next frame)
clears depth. The backend cannot know that at job 6's submit: ES 2.0 has no
`glInvalidateFramebuffer`, and the contents are observable by a later draw
without a clear. Mesa's v3d stores it as well. **No safe cut; at most a
rule "the next use is a full clear" applied one frame late, which is not
possible after the submit.** Board value: none today (a store counter:
`ColourStores`, `backend.rs:268`, counts colour only).

### Not asked, found: early Z for the terrain draw

`early_z_disable = true` in every job (`v3d_job.c:1248`, "legal, only
slower", `v3d_job.h:29-31`). The terrain fragment shader has no `discard` and
writes no depth, so early Z is legal for job 3: fragments behind nearer
terrain skip their 9 samples. With overdraw 1.3 (assumed) that is up to
1.3 M samples, **about 1.7 ms**, more if hills hide more. It is a change in
`driver/src/v3d/v3d_job.c` and `src/v3d` (plans #308, #321 hold them).
Board value: `render` of job 3 falls; the GL-core `draws` do not change.

## What was not done

- **No V3D instruction count of terrain's shaders.** To get it, as
  `gc-terrain-shaders.md` did for the GC7000: write the corpus with the
  harness (`GM2_WRITE=0 GM2_ENTRIES='^terrain$' GM2_FRAMES=1
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out> powervr` writes
  `<out>/shaders/`, the texts are GPU-independent), then compile each text
  with `tests/glmark2_corpus.rs`'s `compile` under `--features
  host-test,gpu-v3d,shaderc-online,no-shader-table` and print the code
  size. It answers the one open ALU number (job 1's noise, job 3's
  fragment).
- **No overdraw of the terrain draw.** A fragment count with and without the
  depth test would bound candidate "early Z"; the host model could count
  fragments if its rasteriser did. It does not.
- **The TMU rate and the second fetch** (candidate 1): the board test above.
- **The V3D clock under the image**: the `board: V3D clock` console line.
  If the firmware's maximum is below 500 MHz, every ms figure here scales by
  500 / clock.
- The CPU side after the fix (what the 30 ms frame spends on the A72, about
  1 to 3 ms) is `v3d-frame-cpu-model.md`; the swap column prediction above
  follows from it only through "the CPU is far ahead".

## Board values, in one place

Read from the next Pi 4 session's console, `glmark2-frame:` and
`glmark2-jobs:` lines of terrain, and the `yakogl/v3d: TFU mipmap at frame
N` line:

| value | before the mark fix (`c4af4917`) | after it, predicted |
|---|---|---|
| frame ms | 161.79 | 30 to 31 |
| gl ms / swap ms | 156.06 / 5.62 | about 20 / about 9 |
| jobs a frame | 9.0 | 9.0 (8.0 with candidate 3) |
| TFU chains a frame | 3 | 5 |
| `tfu` time a frame | 1.8 ms (3 chains + 1 tiling) | 2.64 ms (5 chains) |
| `glmark2-jobs` render, jobs 7 + 8 | not in the tree | about 17 ms of ~27 |
| `glmark2-jobs` render, job 3 | not in the tree | about 7 ms |
