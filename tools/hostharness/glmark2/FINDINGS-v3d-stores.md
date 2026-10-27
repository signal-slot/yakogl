<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: render-target stores a frame per glmark2-es2 scene (baseline for lazy raster)

The baseline that `docs/notes/v3d-sampled-copies.md` (proposal 1, lazy raster)
needs: per scene, how many render-target (RT 0, colour) stores a frame the V3D
back end makes, split into **raster stores** (into the GL core's storage or the
window buffer) and **UIF stores** (into a `RtImage`, `src/v3d/backend.rs`),
with the bytes. No code was changed.

**Every figure below is estimated; none is counted.** Read this first.

## What could be counted, and why nothing was

* No counter exists. The back end has no per-store counter or log. The
  frame-1 arena line (`end_frame`, "job arenas high-water") prints arena
  bytes and draws; `v3d_rt_images` counts `direct`, `cpu_writes`, `chains` and
  so on, not stores; `tools/v3d/job_check.c` part 20 builds one synthetic job
  and counts its stores through Mesa's CLE decoder, which says what one job
  does, not what a scene does. Counting stores per scene needs a counter in
  `src/v3d` (a raster/UIF pair bumped where the job's two
  `STORE_TILE_BUFFER_GENERAL`s are emitted, printed with the frame-1 arena
  line), which this item must not add.
* The host harness could not be run in this session: the unattended session
  was refused the command `tools/hostharness/glmark2/run.sh`. `MESA_V3D_TREE`
  is not set and the Mesa tree in `$CCFARM_DEPS_DIR/mesa` has no `build-v3d`
  (a source tree only), so Mesa's CLE decoder was not available either.
* So each figure is **estimated** from the job list the scene's own source
  makes: glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`
  (`src/scene-*.cpp`, `src/scene-terrain/`), 800x600, steady state (frames
  2.., after the first sample of each target has registered its `RtImage`;
  from the second render on the image is stored). The draws of
  `FINDINGS-v3d.md` are consistent with the jobs derived here (desktop blur
  56 draws = 3 x 14 + setup, terrain 25 = 3 x 8 + setup, refract 6 = 3 x 2).

## Rules the estimate uses

* One job per target switch: a different FBO (or the window) opens a new job;
  draws into the same target stay in that job.
* A job stores RT 0 once into the GL storage (raster) for the window or a
  texture target. When the target's texture is sampled by a later job (a
  registered `RtImage`) it stores RT 0 a second time as UIF (`rt_for_render`).
  The window is never sampled: raster only.
* Bytes are width x height x 4, unpadded. The UIF image is padded to UIF
  blocks (refract's 1600x1200 target is 7,684,096 bytes in
  `docs/notes/v3d-sampled-copies.md`), so UIF bytes are a floor by a few KB a
  store. Depth and stencil stores are not counted (colour only).
* The window is 800x600 RGBA8: 1,920,000 bytes a raster store.

## Command

```sh
git rev-parse HEAD      # b9130d64d878f410d12028267122a29d57cc2ff8
GM2_WRITE=0 GM2_PROFILE=1 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2 v3d
```

The run gives the scene list, draws and GPU memory this table is read against
(`FINDINGS-v3d.md`); the store figures themselves come from the glmark2
sources named above, not from the run. To turn the estimate into counts: add
the counter named above, or decode a scene's jobs the way `tools/v3d/job_check.c`
part 20 does (`MESA_V3D_TREE` with `build-v3d`, `tools/v3d/mesa_ref.sh`).

Main commit: the tree is the integration tip
`b9130d64d878f410d12028267122a29d57cc2ff8` (`docs: v3d-sampled-copies, the
host table at the main tip`). `origin/main` could not be resolved in this
workspace, so that tip is the commit the figures name; `src/v3d` is the same
there as on main for this purpose (this file changes nothing else).

## Stores a frame per scene

Columns: jobs a frame, stores (raster + UIF), raster stores, raster bytes,
UIF stores, UIF bytes, sampled later (a later job reads that render's
texture), and whether the figure is counted or estimated. The last three
columns are **after the lazy raster store** (item v3d-lazy-raster, "After
the lazy raster store" below): raster stores and bytes a frame, and the
frame's RT 0 store bytes (raster + UIF) before and after. The UIF stores
are unchanged.

| # | entry | jobs | stores | raster stores | raster bytes | UIF stores | UIF bytes | sampled later | counted / estimated | after: raster stores | after: raster bytes | store bytes before -> after |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | `build:use-vbo=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 1 | `build:use-vbo=true` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 2 | `texture:texture-filter=nearest` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 3 | `texture:texture-filter=linear` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 4 | `texture:texture-filter=mipmap` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 5 | `shading:shading=gouraud` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 6 | `shading:shading=blinn-phong-inf` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 7 | `shading:shading=phong` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 8 | `shading:shading=cel` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 9 | `bump:bump-render=high-poly` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 10 | `bump:bump-render=normals` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 11 | `bump:bump-render=height` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 12 | `effect2d:kernel=0,1,0;1,-4,1;0,1,0;` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 13 | `effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 14 | `pulsar:quads=5:texture=false:light=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 15 | `desktop:windows=4:effect=blur:blur-radius=5:passes=1:separable=true` | 10 | 19 | 10 | 12,225,600 | 9 | 10,305,600 | yes: desktop 800x600 (by 4 window passes and the screen pass), 4 window textures 210x210 (by the vertical pass) | estimated | 1 | 1,920,000 | 22,531,200 -> 12,225,600 |
| 16 | `desktop:windows=4:effect=shadow` | 2 | 3 | 2 | 3,840,000 | 1 | 1,920,000 | yes: desktop 800x600 (by the screen pass) | estimated | 1 | 1,920,000 | 5,760,000 -> 3,840,000 |
| 17 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 18 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=subdata:interleave=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 19 | `buffer:update-fraction=0.5:update-dispersion=0.9:columns=200:update-method=map:interleave=true` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 20 | `ideas:speed=duration` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 21 | `jellyfish` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 22 | `terrain` | 8 | 15 | 8 | 8,728,576 | 7 | 6,808,576 | yes: height, normal, terrain (twice), bloom_h, bloom_v, tilt_h | estimated | 1 | 1,920,000 | 15,537,152 -> 8,728,576 |
| 23 | `shadow` | 2 | 1 | 1 | 1,920,000 | 0 | 0 | colour no; its 1600x1200 depth-only FBO is sampled (depth, not counted here) | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 24 | `refract` | 2 | 3 | 2 | 9,600,000 | 1 | 7,680,000 | yes: 1600x1200 colour+depth, mipmapped, by the screen pass | estimated | 1 | 1,920,000 | 17,280,000 -> 9,600,000 |
| 25 | `conditionals:vertex-steps=0:fragment-steps=0` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 26 | `conditionals:vertex-steps=0:fragment-steps=5` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 27 | `conditionals:vertex-steps=5:fragment-steps=0` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 28 | `function:fragment-steps=5:fragment-complexity=low` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 29 | `function:fragment-steps=5:fragment-complexity=medium` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 30 | `loop:vertex-steps=5:fragment-steps=5:fragment-loop=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 31 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=false` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |
| 32 | `loop:vertex-steps=5:fragment-steps=5:fragment-uniform=true` | 1 | 1 | 1 | 1,920,000 | 0 | 0 | no | estimated | 1 | 1,920,000 | 1,920,000 -> 1,920,000 |

The scenes that do not use a framebuffer object (every row but 15, 16, 22,
23, 24; the only glmark2 sources that create one are `scene-desktop.cpp`,
`scene-terrain/base-renderer.cpp`, `scene-refract.cpp`, `scene-shadow.cpp`)
render into the window with one job a frame and one raster store.

How each textured scene is derived:

* **terrain** (default `bloom=true`, `tilt-shift=true`): per frame the chain
  height 256^2, normal 256^2, terrain 800x600 (depth, mipmapped), bloom_h
  256^2, bloom_v 256^2, overlay (draws into the terrain target again, a second
  terrain store), tilt_h 800x600, tilt_v (window). 8 jobs; the 7 FBO ones are
  sampled later: 8 raster + 7 UIF stores. The specular map (512^2) is rendered
  once at setup. 5 `glGenerateMipmap` a frame (terrain, bloom_h, bloom_v,
  terrain after the overlay, tilt_h), the 5 waits of `v3d-tfu-mipmap-nowait`.
* **desktop blur**: `desktop.clear()` (a job on the 800x600 desktop texture),
  then per window (4) a horizontal-blur job into the window's own 210x210
  texture (samples the desktop) and a vertical-blur plus contents job back into
  the desktop, then the screen pass: 1 + 4 x 2 + 1 = 10 jobs. Raster bytes
  5 x 1,920,000 + 4 x 176,400 + 1,920,000.
* **desktop shadow**: the four windows' shadows and contents all draw into the
  desktop texture (one job), then the screen pass. The shadow pieces' textures
  are rendered once at setup.
* **refract**: the 1600x1200 (2x window) colour+depth FBO (colour mipmapped,
  sampled by the second pass), then the screen pass.
* **shadow**: the 1600x1200 FBO has a depth attachment only, so there is no
  colour store for it (its D24S8 depth store, 7,680,000 bytes, is not counted
  here); the screen pass stores one window raster.

## After the lazy raster store

Since item v3d-lazy-raster (docs/notes/v3d-lazy-raster.md, "As built") a
job rendering into a target with a render image stores the image alone
(v3d_job.h `uif_only`); the GL core's storage is written from the image
only when a CPU reader needs it, and from then on that target's jobs store
both again (`raster_wanted`: one write-back per target, not one a frame).
The window is never sampled and keeps its one raster store.

**The after columns are estimated, by the rules above, not counted.** The
counter the baseline asked for now exists: `ColourStores` in
`src/v3d/backend.rs`, the RT 0 stores of submitted jobs (raster and image,
with bytes) of the last whole frame, logged at shutdown as "yakogl/v3d:
colour stores in the last frame: N raster (B bytes), M render image (B
bytes); level 0 written back for a CPU reader K times", which lands in each
entry's `logs/v3d/<i>.log` of the command above. The command itself could
not be run in this session either: `tools/hostharness/glmark2/run.sh`,
`cmake` and `g++` were each refused to the unattended session. A counted run
replaces these figures with that line, per entry.

The estimate assumes, on top of the rules above: steady state (frames 2..),
every sampled target holds a render image (glmark2's most at once is
terrain's 7, of `RT_IMAGES` 8), and no CPU reader of a target in the frame.
The last holds for the board's setting, `YAKOGL_V3D_TFU=1` (rpi_4b.conf,
which the command takes): terrain's and refract's glGenerateMipmap go to the
TFU, which reads the image; a refused one (the arena line's mipmap
refusal names) takes the GL core's CPU filter, which writes level 0 back
once, and that target then stores both, as in the before columns.

| scene | RT 0 store bytes a frame, before | after | saved |
|---|---|---|---|
| desktop blur (15) | 22,531,200 | 12,225,600 | 10,305,600 (9 raster stores) |
| desktop shadow (16) | 5,760,000 | 3,840,000 | 1,920,000 (1) |
| `terrain` (22) | 15,537,152 | 8,728,576 | 6,808,576 (7) |
| `refract` (24) | 17,280,000 | 9,600,000 | 7,680,000 (1) |
| `shadow` (23) | 1,920,000 | 1,920,000 | 0 |
| the other 28 scenes | 1,920,000 | 1,920,000 | 0 |

`shadow` does not move: its sampled FBO is depth-only (no RT 0 store to
drop); its per-frame cost is the R32F depth storage, the item
v3d-depth-storage-defer. Its colour store is the window's.

## Ranking: where the lazy raster would help most

The lazy raster drops the raster store of a target that only the GPU reads,
which is the raster bytes of the sampled targets (the window's store stays).
Bytes a frame saved, estimated:

| rank | scene | raster stores dropped | bytes a frame | note |
|---|---|---|---|---|
| 1 | desktop blur (15) | 9 | 10,305,600 | most bytes: five 1.92 MB desktop stores a frame |
| 2 | `refract` (24) | 1 | 7,680,000 | one 1600x1200 store; also carries the mipmap wait |
| 3 | `terrain` (22) | 7 | 6,808,576 | most stores of the scenes that matter on the board; 5 mipmaps a frame |
| 4 | desktop shadow (16) | 1 | 1,920,000 | |
| - | the other 28 scenes | 0 | 0 | window only |
| - | `shadow` (23) | 0 | 0 | depth-only FBO: lazy raster does not apply (proposal 2, the depth storage) |

By bytes the order is desktop blur, refract, terrain, desktop shadow; by store
count desktop blur (9) and terrain (7) lead. On the Pi 4 the slow scenes of
these are refract and terrain (`docs/notes/pi4-glmark2-cpu.md`); this file
measures no time, so the board effect needs the board check named in
`docs/notes/v3d-sampled-copies.md`.

## Notes

* Risk of the estimate: it assumes each job stores RT 0 once. A job split by
  the bin pool ("a job with more tiles than the pools hold"), a dropped job,
  or a draw that takes the tiled-copy path changes the real count. A counted
  run should replace these figures.
* Frames 2.. only: the first sample of a target is a tiled copy and the first
  render after it stores the image.
