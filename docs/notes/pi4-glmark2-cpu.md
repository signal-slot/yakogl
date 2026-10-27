<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 glmark2: terrain, shadow and refract CPU time on the V3D host build

On the Pi 4, glmark2 terrain, shadow and refract run at about 6 fps, with
170 to 188 ms of GL-call time a frame (`c854e0ae`; #1209,
#995). This note is the host profile of those three scenes: where the CPU time
of a frame goes, per GL entry point and per function of the GL core and the
V3D back end.

## How it was measured

The glmark2 host harness, V3D build (`tools/hostharness/glmark2/README.md`,
"Profile"), at `c451ca46` (main `499b946e` plus the harness's timing):

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm-prof v3d
GM2_WRITE=0 GM2_PERF=1 GM2_FRAMES=20 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm-perf v3d
```

- 800x600, 20 frames; the counted frames are frames 2 to 20 (19 frames).
  The first frame is in "setup", with the shader compiles.
- The milliseconds come from the `GM2_PROFILE=1` run (per entry point,
  timed around each call). The split inside an entry point comes from the
  perf run (`perf record -e cpu-clock --call-graph dwarf`, 2000 Hz, counted
  frames only), as its share of samples times the profile run's cpu ms a
  frame. perf makes the frames about 15% slower, so its own milliseconds
  are not used.
- Host time, on an x86-64 desktop. The Pi 4's Cortex-A72 is several times
  slower, and it reads the GPU's non-cacheable memory, which the host's
  malloc'd memory is not. The ranking carries over to the board; the
  milliseconds do not.
- GPU waits are not in these numbers: `src/osal/linux.c` completes every
  submit at once. Wall and cpu time agree (within 2%) in all three scenes.

## Per scene, the ranked costs

### shadow: 11.3 ms a frame, 99% of it in `glDrawArrays`

| # | cost | ms a frame | share | call site |
|---|---|---|---|---|
| 1 | depth resolve: the D24S8 depth buffer read back into the R32F depth texture | 6.1 | 54% | the ground's `glDrawArrays` → `src/gl/draw.rs:3147` `resolve_sampled_depth` → `src/gl/texture.rs:694` `resolve_depth` → `src/v3d/backend.rs:3962` `v3d_depth_resolve` → Mesa's `v3d_tex_load_box` (detile) |
| 2 | tiled copy of the resolved depth texture (7,684,096 bytes) for the TMU | 5.1 | 45% | same draw → `src/v3d/backend.rs:3451` `tex_state_for` → `:3495` `copy_image_for` → `:3635` `tiled_copy` → `v3d_tex_store_level` (tile) |
| 3 | everything else (draw building, state, swap) | 0.1 | 1% | |

Depth resolve's share: **all of the frame**. The resolve writes the R32F
texels, which moves the texture's stamp, so the copy made for the last
frame is stale ("a stale tiled copy of 7684096 bytes given back before the
job opened") and is made again: both rows are the depth texture, every frame.

### refract: 29.3 ms a frame, 100% of it in GL calls

| # | cost | ms a frame | share | call site |
|---|---|---|---|---|
| 1 | tiled copies for sampling | 13.0 | 45% | the refracting draw's `glDrawArrays` → `tex_state_for` → `copy_image_for` → `tiled_copy` (`src/v3d/backend.rs:3451, 3495, 3635`) |
| 1a | of which, the colour target's 11-level chain (about 11.0 MB) | ~7.7 | ~26% | estimate: row 1 split by the bytes tiled |
| 1b | of which, the depth texture (7,684,096 bytes) | ~5.3 | ~18% | estimate, as 1a |
| 2 | `glGenerateMipmap` of the 1600x1200 colour target: the GL core's box filter on the CPU | 10.3 | 35% | `src/gl/texture.rs` glGenerateMipmap, after `backend::generate_mipmap` answered 0 (line 3824) |
| 3 | depth resolve | 6.2 | 21% | as shadow's row 1 |
| 4 | GPU memory for the new copies (`l_gpu_alloc` and its memset) | ~1.0 | 4% | inside rows 1 and 3 |

Depth resolve's share: rows 1b and 3, about **11.5 ms, 39%** of the frame.

The tiled-copy high-water is 19,742,720 bytes: the 7.68 MB depth copy, the
11.0 MB colour chain and a 1 MB texture. Splitting row 1 by bytes assumes
the cost per byte is the same for both. It is the same Mesa routine
(`v3d_move_pixels_general_percpp`) and both are 4 bytes a texel.

### terrain: 13.3 ms a frame, 99% of it in GL calls

| # | cost | ms a frame | share | call site |
|---|---|---|---|---|
| 1 | `glGenerateMipmap`, 5 calls a frame, the GL core's box filter on the CPU | 8.1 | 62% | glmark2's `BaseRenderer::update_mipmap` (terrain target twice, the second time after the overlay; `bloom_h`; `bloom_v`; `tilt_h`) → `src/gl/texture.rs` glGenerateMipmap, `backend::generate_mipmap` answered 0 |
| 2 | tiled copies of the mipmapped render targets when a draw samples them | 4.9 | 37% | `glDrawArrays` (8 a frame) → `tex_state_for` → `copy_image_for` → `tiled_copy` |
| 3 | everything else (draws, 18 `glBindTexture` and `glUseProgram` a frame, swap) | 0.2 | 1% | |

Tiled-copy high-water: 12,433,408 bytes. Only one target is sampled from
its render image ("a 256x256 render target is sampled from its render
image (262144 bytes, 1 levels): no tiled copy"): the height/normal map,
which has one level. The four mipmapped targets (800x600 terrain and
tilt-shift, 256x256 bloom) are copied every frame.

Setup (once a scene, not a frame): terrain spends 103 ms in GL calls
before its first swap returns. `glLinkProgram` 29.8 ms (9 programs),
`glGenerateMipmap` 24.8 ms, the first `glDrawArrays` 19.1 ms, `glBufferData`
10.3 ms. Not part of the frame rate.

## Why terrain's two costs are one

The two top costs of terrain, and row 2 of refract, have one cause:

1. `glGenerateMipmap` of an RGBA8 render target asks the back end first
   (`backend::generate_mipmap`, `src/gl/texture.rs:3824`). The V3D makes
   the chain in the render image with one TFU job
   (`src/v3d/backend.rs:1911` `v3d_generate_mipmap`), or answers 0.
2. When it answers 0, the GL core filters the chain on the CPU (cost 1) and
   stamps the texels.
3. The stamp moved, so the render image no longer matches the storage. The
   next draw that samples the target cannot use the image and tiles a copy
   of the whole chain (cost 2) (`docs/online-shaderc/v3d-backend.md`,
   "Render targets sampled from their render image").

On the host, the hook answers 0 at `src/v3d/backend.rs:1930`
(`tfu_min_bytes == 0`): `linux.c` has no TFU. So **on the host both costs
come from the missing TFU**. With the TFU path taken, neither should exist:
no CPU filter, no stamp move, and the sample reads the image. Terrain's
remaining CPU work would be row 3, about 0.2 ms on the host.

Whether the Pi 4 takes the TFU path is **not known**. The glmark2 image
turns the TFU on (`driver/samples/zephyr/glmark2/boards/rpi_4b.conf`,
`CONFIG_YAKOGL_V3D_TFU=y`). But #1209 records that
the `c854e0ae` console did not print refract's "the TFU made the 11-level
chain of a 1600x1200 render image" line. If the board also answers 0 (for
any of the reasons at `src/v3d/backend.rs:1914-1949`), the board does what
the host does: these costs, from non-cacheable memory, on a slower CPU.
That alone could explain the 170 to 188 ms. If the board does take the TFU
path, terrain's board time is not CPU work: each TFU `glGenerateMipmap`
submits the open job and waits for it (`wait_in_flight`, `:2412`), so 5
waits a frame, which the host cannot show.

## Proposed fixes

### Terrain's top two costs

1. **CPU `glGenerateMipmap` (8.1 ms, 62%).** First, find which branch of
   `v3d_generate_mipmap` the board takes. Give each `return 0` a reason
   counter, and print the counts once, at the frame-1 arena line
   (`job arenas high-water at frame N`). Then one Pi 4 glmark2 run shows the
   reason for terrain's four targets and refract's one. Then fix that
   branch. If the reason is "no TFU" (`tfu_min_bytes == 0`), check the boot
   line "tiled copies of N bytes or more by the TFU" and the OSAL's
   `OSAL_QUERY_V3D_TFU_MIN_BYTES` answer. If it is `seen_gen` or `stored`,
   check the order of registration, render and mipmap per frame against the
   design. Terrain's overlay renders into the terrain target a second time,
   and that target is mipmapped twice a frame. For the host, let
   `src/osal/linux.c` answer the TFU queries and run the mipmap job on the
   CPU in the image's layout. Then the harness takes the board's path and
   shows what is left (now not modelled: README "Profile").
2. **Tiled copies of the mipmapped targets (4.9 ms, 37%).** These go away
   with fix 1: the stamp stays and the sample reads the render image. Where
   the CPU fallback must stay (no TFU, a failed TFU job), filter the chain
   into the render image as well as the storage, and keep the stamp the
   image is valid for. Then the copy is not needed. Do not make the fallback
   faster in place: the copy is the larger part on the board, from
   non-cacheable memory.

### Depth resolve (shadow, refract)

It was all of shadow's frame and 39% of refract's. The CPU detiled the
D24S8 buffer into R32F rows (row 1). That write moved the stamp, so the CPU
tiled the rows again for the TMU (row 2): two full passes over 7.68 MB a
frame.

Done: the TMU now samples the depth buffer itself with its DEPTH24_X8 type
(`v3d-backend.md`, "Depth textures sampled from their depth buffer"). The
GL core puts the texture's depth object id in the draw's texture binding
(`GlesTextureBind::depth_id`) after the backend answers that it samples
that depth itself (`backend::samples_rendered_depth`). Both rows are gone
for a draw that samples the depth texture after a render into it:
`tests/depth_resolve_v3d.rs` runs shadow's frame and sees no resolve and
nothing tiled, and the texels the TMU reads equal the old resolve's. The
CPU resolve stays only for the CPU's own reads and writes of the texels.

Measured with the "How it was measured" profile command for shadow and
refract (`GM2_PROFILE=1 GM2_FRAMES=20`, `YAKOGL_V3D_TFU=1`), back to back
on one machine. "before" is main `0fbc81b0`, "after" is the same tree with
this change. The table under "With the host's TFU model" below had 7.29
(shadow) and 6.96 (refract) for "before", on another day.

| scene | before: cpu ms a frame | before: TFU jobs a frame | after: cpu ms a frame | after: TFU jobs a frame |
|---|---|---|---|---|
| shadow | 7.09 | 1 (depth texture tiling, 5.2 ms model) | 0.12 | 0 |
| refract | 7.22 | 2 (11-level mipmap 39.6, depth tiling 5.1 ms model) | 0.23 | 1 (11-level mipmap) |

- Before, `glDrawArrays` was 99% of each frame (6.98 and 7.08 cpu ms):
  the resolve, and "a stale tiled copy of 7684096 bytes given back
  before the job opened" every frame.
- After, both scenes log "a 1600x1200 depth texture is sampled from its
  depth buffer (DEPTH24_X8): no CPU resolve, no tiled copy", and the
  depth texture's TFU job is gone too. The carve-out high-water drops by
  7,684,096 bytes in both scenes (the depth copy).
- No GL error or refused draw in either run. The host renders nothing,
  so the picture line is black in both runs; the picture is the board's
  to check.

Terrain's fix 1 above (the target mipmapped twice a frame) is not part of
this change and is still open.

## With the host's TFU model (linux-tfu-model)

`src/osal/linux.c` now models the TFU (`YAKOGL_V3D_TFU=1`, the glmark2
image's setting, which `run.sh` passes). The model's time is GPU work: it
is not in the cpu column below, and the GPU column gives its host time.
Both runs are the "How it was measured" profile command, back to back on
one machine, at main `f691d33a` plus the model. The "before" run is the
same build with `YAKOGL_V3D_TFU=0` (the CPU path, as in the tables above).
Its milliseconds are higher than in the tables above (another day and
tree), so compare the two runs here with each other, not with the tables
above.

| scene | before: cpu ms a frame | before: top entry points | after: cpu ms a frame | after: TFU jobs a frame | after: model's host ms a frame |
|---|---|---|---|---|---|
| terrain | 15.20 | `glGenerateMipmap` 9.18, `glDrawArrays` 5.89 | 0.46 | 5 (mipmaps) | 28.3 |
| shadow | 14.38 | `glDrawArrays` 14.48 | 7.29 | 1 (depth texture tiling) | 5.6 |
| refract | 48.14 | `glDrawArrays` 32.31, `glGenerateMipmap` 17.70 | 6.96 | 2 (11-level mipmap 38.7, depth tiling 4.4) | 43.1 |

(The top entry points are wall ms a frame.) What the rows say:

- terrain: all its CPU cost was the missing TFU. With the TFU, its 5
  `glGenerateMipmap` calls are 5 jobs ("the TFU made the 10-level chain of
  a 800x600 render image"), and the draws sample the render images: no
  tiled copy is left (`glDrawArrays` 5.89 -> 0.17 ms). What is left is
  0.25 ms in GL calls. On the board, terrain's cost is then the CPU's 5
  waits for the jobs a frame, which the host cannot show.
- shadow and refract: the tiled copy of the depth texture is a TFU job
  now. What was left on the CPU (about 7 ms) was the depth resolve, which
  is gone now ("Depth resolve (shadow, refract)" above). refract's colour
  chain is one TFU job ("the TFU made the 11-level chain of a 1600x1200
  render image").
- The model's host time is larger than the CPU path it replaces (terrain
  28.3 ms against about 15 ms). The model is a plain per-byte loop that
  detiles level 0 and tiles every level, and it is written to match the
  CPU path's bytes, not to be fast. It is not the board's TFU time.

No scene raised a GL error or refused a draw in either run.

## On the board: glmark2 from main `dcbfb30b`, 2026-10-19

The board-side check of the depth-texture change above. The GM row of the
common set (`tools/cts/common-tree.txt`), built by `tools/cts/cts-common.sh
pi4 build` from main `dcbfb30b` (the commit that has the change and this
note; the main tip of the day, `92a17d3f`, differs from it only in Mali
files, which the Pi 4 image does not build), run by `tools/board.sh pi4 run`
(tryboot, the lab's 17-field `BOARD_HDMI_TIMINGS`, 96 MiB carve-out, window
800x600, EGL double buffer). One GL client on the board. Record (gitignored):
`records/pi4/common-dcbfb30b/20261019-2120-GM-0edbe4a2/`, image sha256
`0edbe4a2...`.

```
board.sh pi4 GM rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c1,6184003f,00258320 back_after=470s unsupported=0
glmark2: score 193: 33 scenes ran, 0 failed, 0 unsupported (33 list entries); window 800x600, shown yes, GPU resets 0
glmark2: GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)
```

**Score 193** (188 at `33a0f264` and `499b946e`, 187 at `a19927f3`, 157 at
`c854e0ae` before the EGL double buffer). The console has the two lines the
change adds, once each in shadow and refract: "a 1600x1200 depth texture is
sampled from its depth buffer (DEPTH24_X8): no CPU resolve, no tiled copy".
Terrain still logs "the TFU made the 10-level chain of a 800x600 render
image: glGenerateMipmap filters nothing": the board takes the TFU path, so
terrain's 174 ms of GL-call time is not the CPU mipmap filter; its
`glmark2-steps` line puts 7.8 ms a frame in uniforms and the rest outside
the timed steps (the 5 TFU waits a frame, "What the host does not show").

Per scene, ms/frame from `FrameTime` (`common-tally.py --gm-table`; 33 of 33
from FrameTime), against the 188 run of `33a0f264` (its figures from the
table that run left, `a8fdc9f0:docs/handoff/20261017-pi4-gm-swap.md`; the
old record is not on the node that ran this, so no `--gm-diff`). "gl" and
"swap" are the `glmark2-frame` means, "end waited" its "end waited for in N
of N":

| scene | old ms (`33a0f264`) | new ms | change | fps | gl ms | swap ms | end waited |
|---|---:|---:|---:|---:|---:|---:|---|
| build use-vbo=false | 6.41 | 6.39 | -0.3% | 157 | 2.73 | 3.65 | 2 of 1570 |
| build use-vbo=true | 3.64 | 3.62 | -0.5% | 277 | 0.02 | 3.60 | 1 of 2761 |
| texture nearest | 3.75 | 3.73 | -0.5% | 268 | 0.03 | 3.70 | 1 of 2687 |
| texture linear | 3.72 | 3.71 | -0.3% | 270 | 0.03 | 3.68 | 1 of 2698 |
| texture mipmap | 3.69 | 3.68 | -0.2% | 272 | 0.03 | 3.65 | 1 of 2718 |
| shading gouraud | 3.68 | 3.66 | -0.5% | 274 | 0.02 | 3.63 | 1 of 2732 |
| shading blinn-phong-inf | 4.42 | 4.42 | -0.1% | 227 | 0.05 | 4.36 | 1 of 2266 |
| shading phong | 4.45 | 4.45 | -0.1% | 225 | 0.05 | 4.39 | 1 of 2255 |
| shading cel | 4.42 | 4.43 | +0.2% | 226 | 0.05 | 4.37 | 1 of 2260 |
| bump high-poly | 4.59 | 4.59 | 0.0% | 218 | 0.05 | 4.53 | 1 of 2181 |
| bump normals | 4.42 | 4.42 | 0.0% | 227 | 0.07 | 4.35 | 1 of 2264 |
| bump height | 4.44 | 4.44 | 0.0% | 226 | 0.07 | 4.37 | 1 of 2259 |
| effect2d edge | 4.46 | 4.47 | +0.2% | 224 | 0.05 | 4.41 | 1 of 2241 |
| effect2d convolution 5x5 | 4.47 | 4.47 | 0.0% | 224 | 0.05 | 4.41 | 1 of 2239 |
| pulsar | 4.49 | 4.50 | +0.1% | 223 | 0.16 | 4.33 | 1 of 2231 |
| desktop blur | 13.03 | 13.03 | 0.0% | 77 | 7.11 | 5.91 | 768 of 768 |
| desktop shadow | 6.81 | 6.82 | +0.2% | 147 | 0.83 | 5.98 | 1467 of 1467 |
| buffer not interleaved, map | 10.17 | 10.15 | -0.2% | 99 | 5.62 | 4.52 | 1 of 986 |
| buffer not interleaved, subdata | 10.02 | 10.02 | 0.0% | 100 | 5.49 | 4.51 | 1 of 998 |
| buffer interleaved | 8.77 | 8.76 | -0.1% | 115 | 4.24 | 4.50 | 1 of 1142 |
| ideas | 11.34 | 12.04 | +6.2% | 84 | 7.59 | 4.45 | 1 of 832 |
| jellyfish | 4.50 | 4.50 | -0.1% | 223 | 0.11 | 4.38 | 1 of 2226 |
| **terrain** | 178.86 | 178.94 | 0.0% | 6 | 173.94 | 4.88 | 56 of 56 |
| **shadow** | 175.28 | 7.30 | **-95.8%** | 137 | 0.97 | 6.33 | 1370 of 1371 |
| **refract** | 196.03 | 18.77 | **-90.4%** | 54 | 11.59 | 7.17 | 534 of 534 |
| conditionals 0/0 | 4.36 | 4.37 | +0.1% | 230 | 0.05 | 4.31 | 1 of 2293 |
| conditionals 5/0 | 4.36 | 4.36 | 0.0% | 230 | 0.05 | 4.31 | 1 of 2293 |
| conditionals 0/5 | 4.36 | 4.36 | 0.0% | 230 | 0.05 | 4.31 | 1 of 2293 |
| function low | 4.36 | 4.36 | 0.0% | 230 | 0.05 | 4.31 | 1 of 2292 |
| function medium | 4.36 | 4.37 | +0.2% | 229 | 0.05 | 4.31 | 1 of 2292 |
| loop fragment-loop=false | 4.36 | 4.37 | +0.2% | 229 | 0.05 | 4.31 | 1 of 2291 |
| loop fragment-uniform=false | 4.36 | 4.37 | +0.2% | 229 | 0.05 | 4.31 | 1 of 2291 |
| loop fragment-uniform=true | 4.37 | 4.37 | 0.0% | 229 | 0.05 | 4.31 | 1 of 2291 |

What it says:

- **shadow: 175.28 -> 7.30 ms (6 -> 137 fps), refract: 196.03 -> 18.77 ms
  (6 -> 54 fps).** The GL-call time went from 170 and 188 ms to 0.97 and
  11.59 ms: the resolve and the tiled copy of the depth texture were the
  whole of shadow's frame and most of refract's, as the host profile said.
  Against the 187 run of `a19927f3` (shadow 175.82, refract 194.50) the
  change is the same. The carve-out high-water of the whole run is
  76,772,096 bytes of 100,134,912, reached in refract.
- **terrain: unchanged at 178.9 ms (6 fps).** Fix 1 above (the target
  mipmapped twice a frame, the 5 TFU waits) is still open; nothing in this
  change touches it. refract's remaining 11.6 ms of GL time is outside its
  timed steps (its `glmark2-steps` sum is 0.09 ms), which fits the wait for
  its `glGenerateMipmap` TFU job (the 11-level 1600x1200 chain).
- **Both scenes still read "end waited for in N of N"**: shadow 1370 of
  1371, refract 534 of 534 (`docs/notes/egl-swap-timeline.md`; a V3D frame
  with more than one job waits for the one in flight). Their swap column is
  6.3 and 7.2 ms, so the frame is now GPU-bound, not CPU-bound.
- **Every other scene is within 0.5 percent of the 188 run**, except ideas
  at 12.04 ms (+6.2 percent; `a19927f3` read 11.69, so it has drifted over two
  runs, not with this change). The score moved 188 -> 193 on shadow and
  refract alone.
- No GL error, no refused draw, no incomplete texture sampled, no memory
  refusal, 0 GPU resets. Terrain prints 25 groups of four `fs uniform[0..3]`
  lines at set-up (a debug print of the shader compiler, not a GL error);
  whether earlier runs printed them was not checked here.

### The 33 pictures

Every scene has a `glmark2-picture:` line (33 of 33), every line parses,
none is `absent`, none is `black yes`. Nobody looked at the panel (no camera
on the Pi 4's display in this session), so these are a regression baseline,
as the `imx8mp` column is. Against the Pi 4's own pictures of the 187 run
(`a19927f3`, `4d7c38cb9:tools/cts/glmark2-pictures.txt`, never merged) the
mean of every scene is within 1 level per channel and the probes within 8
levels, except the centre probe of terrain (`98a61d` -> `525815`) and
refract (`8f59a4` -> `5b4fb7`): both scenes animate, so the centre pixel
depends on the frame the readback caught; their means agree (terrain
120 129 44 both times, refract 24 20 31 -> 32). Against the `am62p` column
the tally reads match 28, mismatch 5 (desktop blur, two buffer scenes,
terrain, refract: probes 18 to 153 levels apart, which is a different GPU
and a different frame, not a defect). The crc16 differs from `a19927f3` in
30 of 33 scenes, as expected of an exact checksum of a moving picture.

`common-tally.py --gm-picture-lines <console> pi4`, ready for the `pi4`
column of `tools/cts/glmark2-pictures.txt` (readme-one-commit adds it; this
note does not touch the table):

```
pi4 | [build] use-vbo=false | 800x600; probes c a4a4a4ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 9870, crc16q a224; mean 12 12 12; black no
pi4 | [build] use-vbo=true | 800x600; probes c a4a4a4ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 8962, crc16q e3cd; mean 12 12 12; black no
pi4 | [texture] texture-filter=nearest | 800x600; probes c 2b2d2dff, tl 181a1aff, tr 000000ff, bl 050404ff, br 000000ff; crc16 aeb8, crc16q ca03; mean 14 14 14; black no
pi4 | [texture] texture-filter=linear | 800x600; probes c 2d2e2eff, tl 191b1bff, tr 000000ff, bl 050404ff, br 000000ff; crc16 b8e2, crc16q 6eed; mean 14 14 14; black no
pi4 | [texture] texture-filter=mipmap | 800x600; probes c 2d2e2fff, tl 1b1c1dff, tr 000000ff, bl 050404ff, br 000000ff; crc16 53aa, crc16q d4e5; mean 14 14 14; black no
pi4 | [shading] shading=gouraud | 800x600; probes c 0000c9ff, tl 000000ff, tr 000000ff, bl 000012ff, br 00008eff; crc16 5023, crc16q 83c1; mean 0 0 17; black no
pi4 | [shading] shading=blinn-phong-inf | 800x600; probes c 1919baff, tl 000000ff, tr 000000ff, bl 191924ff, br 19198cff; crc16 435a, crc16q ff01; mean 5 5 18; black no
pi4 | [shading] shading=phong | 800x600; probes c 0505a2ff, tl 000000ff, tr 000000ff, bl 050586ff, br 0505bfff; crc16 4118, crc16q b7f8; mean 1 1 28; black no
pi4 | [shading] shading=cel | 800x600; probes c 009900ff, tl 000000ff, tr 000000ff, bl 009900ff, br 009900ff; crc16 f6b2, crc16q 7354; mean 0 25 0; black no
pi4 | [bump] bump-render=high-poly | 800x600; probes c a1a1a1ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 573d, crc16q 0059; mean 15 15 15; black no
pi4 | [bump] bump-render=normals | 800x600; probes c abababff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 880e, crc16q ee39; mean 15 15 15; black no
pi4 | [bump] bump-render=height | 800x600; probes c 989898ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 fe03, crc16q 6ed6; mean 15 15 15; black no
pi4 | [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | 800x600; probes c 000000ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 1c4e, crc16q 2919; mean 0 0 1; black no
pi4 | [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | 800x600; probes c ffffffff, tl ffffffff, tr ffffffff, bl ffffffff, br ffffffff; crc16 4857, crc16q c3f1; mean 231 233 221; black no
pi4 | [pulsar] light=false:quads=5:texture=false | 800x600; probes c 70016fff, tl 5a0313ff, tr 000000ff, bl 000000ff, br 811b4dff; crc16 1806, crc16q 6cb3; mean 47 32 45; black no
pi4 | [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | 800x600; probes c ffffffff, tl 919191ff, tr c0c0c0ff, bl ffffffff, br ffffffff; crc16 ae81, crc16q 60aa; mean 205 210 191; black no
pi4 | [desktop] effect=shadow:windows=4 | 800x600; probes c ffffffff, tl 919191ff, tr c0c0c0ff, bl ffffffff, br ffffffff; crc16 5402, crc16q ca35; mean 197 202 183; black no
pi4 | [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | 800x600; probes c b4daf0f0, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 f070, crc16q 1559; mean 42 57 66; black no
pi4 | [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | 800x600; probes c c1e0f3f3, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 8f57, crc16q 71b5; mean 41 56 65; black no
pi4 | [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | 800x600; probes c b6dbf0f0, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 dff7, crc16q fe37; mean 41 57 66; black no
pi4 | [ideas] speed=duration | 800x600; probes c 2b223cff, tl 010101ff, tr 010101ff, bl 000000ff, br 010101ff; crc16 76e7, crc16q e1db; mean 5 4 7; black no
pi4 | [jellyfish] <default> | 800x600; probes c 59261097, tl 578ef9ff, tr 578ef9ff, bl 3358c8ff, br 3358c8ff; crc16 901e, crc16q 280a; mean 70 104 195; black no
pi4 | [terrain] <default> | 800x600; probes c 525815ff, tl 555413ff, tr 6d841fff, bl 6c751cff, br 637b11ff; crc16 32fe, crc16q c189; mean 120 129 44; black no
pi4 | [shadow] <default> | 800x600; probes c a0a0a0ff, tl 000000ff, tr 000000ff, bl 4d4d4dff, br 4d4d4dff; crc16 f3a7, crc16q c98d; mean 34 34 34; black no
pi4 | [refract] <default> | 800x600; probes c 5b4fb7ff, tl 000000ff, tr 000000ff, bl 000000ff, br 000000ff; crc16 33b5, crc16q 06cd; mean 24 20 32; black no
pi4 | [conditionals] fragment-steps=0:vertex-steps=0 | 800x600; probes c 000000ff, tl 000000ff, tr fdfdfdff, bl fefefeff, br f9f9f9ff; crc16 2224, crc16q 89a3; mean 57 57 57; black no
pi4 | [conditionals] fragment-steps=5:vertex-steps=0 | 800x600; probes c 000000ff, tl 000000ff, tr c2c2c2ff, bl eaeaeaff, br 474747ff; crc16 7307, crc16q c47c; mean 50 50 50; black no
pi4 | [conditionals] fragment-steps=0:vertex-steps=5 | 800x600; probes c 000000ff, tl 030303ff, tr fdfdfdff, bl fefefeff, br f9f9f9ff; crc16 659e, crc16q 7a1d; mean 55 55 55; black no
pi4 | [function] fragment-complexity=low:fragment-steps=5 | 800x600; probes c 000000ff, tl 000000ff, tr 2c2c2cff, bl 636363ff, br 868686ff; crc16 2790, crc16q 9cf2; mean 54 54 54; black no
pi4 | [function] fragment-complexity=medium:fragment-steps=5 | 800x600; probes c 000000ff, tl 000000ff, tr 2c2c2cff, bl 636363ff, br 868686ff; crc16 ec86, crc16q 4511; mean 54 54 54; black no
pi4 | [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | 800x600; probes c 000000ff, tl 000000ff, tr 2c2c2cff, bl 636363ff, br 868686ff; crc16 c2a9, crc16q 808e; mean 53 53 53; black no
pi4 | [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | 800x600; probes c 000000ff, tl 000000ff, tr 2c2c2cff, bl 636363ff, br 868686ff; crc16 3139, crc16q 952c; mean 54 54 54; black no
pi4 | [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | 800x600; probes c 000000ff, tl 000000ff, tr 2c2c2cff, bl 636363ff, br 868686ff; crc16 b58e, crc16q 3a4a; mean 54 54 54; black no
```

How the console was captured: the session's console lease held the Pi 4's
serial bridge, so `board.sh` read the feed bridge of
`tools/am62p_netboot_uart.py feed` (`tools/cts/README.md`, "The console
under a farm console lease"), fed by hand from the lease's console in
pieces of up to 30 s. The record's `console.ts` times are the relay's, not
the board's (no timing claim rests on them; the per-frame figures are the
image's own). The relay passed every result, `glmark2-frame`, `-steps`,
`-picture` and `-mem` line as printed; it left out the Linux shutdown
lines before the boot, part of the firmware's boot chatter and 24 of the 25
identical `fs uniform[0..3]` groups of terrain's set-up, so the record's
console.txt is 359 lines, not the board's full output.

## glGenerateMipmap without the CPU wait (v3d-tfu-mipmap-nowait)

Terrain's 178.9 ms and refract's remaining 11.6 ms of GL time on the board
(above) fit the wait every delegated `glGenerateMipmap` made: it submitted
the open render job, waited for it on the CPU (`wait_in_flight`) and only
then ran the TFU job. Mesa's v3d does not wait: its TFU ioctl carries an
in-sync on the render's out-sync and the kernel orders the two.

Now `v3d_generate_mipmap` submits the render and queues the TFU job behind
the job in flight (`OSAL_QUERY_V3D_TFU_QUEUE`, `src/osal/osal_internal.h`):

- The order on the board is the Zephyr OSAL's own queue. `zephyr_v3d.c`
  keeps one job in flight; a TFU job queued behind it is run by
  `finish_inflight` after the render's FRDONE and the cache clean, before
  the job's fence completes, and every submit, `wait()` and `completed()`
  goes through `finish_inflight` first. So the render that stores level 0,
  the TFU job that reads it, and the next job (the draw sampling the chain)
  run in that order with no CPU wait inside the call. The TFU and a render
  never overlap, as before (`osal_v3d_tfu`, "the TFU must not run beside a
  render"). The OSAL keeps 4 queued jobs; a fifth waits for the job in
  flight (counted, "waited" below).
- The CPU still finishes the render, at the next submit or wait, but by then
  it has recorded the next job: the wait moved out of `glGenerateMipmap`
  to where Mesa's CPU would block too. The TFU job itself runs inside that
  later `finish_inflight` (polled, as before).
- A CPU read of the chain (`rt_chain_back`) waits for the fence the TFU job
  ran behind first (`RtImage::tfu_fence`).
- Every refusal branch has a counter, printed beside the frame-1 arena
  line and at shutdown: `TFU mipmap at frame 1: N chains (Q queued behind a
  job, W waited), refused: uninit, levels, not-rt, unknown, stamp,
  relayout, no-tfu, flush, unstored, wait, layout, pack, no-query, tfu`.
  A board run then names the branch terrain takes instead of guessing it.

`tests/v3d_tfu_mipmap_nowait.rs` pins the order on an OSAL of the board's
shape (one job in flight, finished only by a wait or the next submit): the
render submitted and the TFU job queued behind its fence with no `wait()`
inside the call, the TFU run before the sampling job's submit, and the
sampling draw reading the render image through all 7 levels; with the
render already submitted the TFU job is queued behind the later job in
flight, and with every job completed it runs within the call. Against
main's `v3d_generate_mipmap` both tests fail on the wait (`[Submit(11),
Wait(11), TfuRan]`).

Host before/after, `YAKOGL_V3D_TFU=1`: the Linux OSAL completes every
submit inside `submit()`, so no wait costs host time and the milliseconds a
frame are not expected to move; the primary host metric is the CPU waits.
The harness counts them: with `GM2_PROFILE=1` on a V3D build, `shim.c`
prints the back end's jobs and waits over the counted frames
(`gm2: v3d-frames counted=<n> kicks=<k> waits=<w>`, from `v3d_counters`)
and the waits made inside each entry point (`gm2: v3d-waits
glGenerateMipmap setup=.. frames=.. teardown=..`). The run, on the commit
before this change (with only `v3d_counters` exported, which the shim needs)
and on the change:

```
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=20 GM2_ENTRIES='^(terrain|refract)$' YAKOGL_V3D_TFU=1 \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out> v3d
```

Before is main at `fef4f270` with this change's `shim.c` and `report.py`
and `#[no_mangle]` on `v3d_counters`, nothing else; after is the change.
Both ran one after the other on one x86-64 host (AMD Ryzen 9 5900HX).
ms a frame is `<out>/profile-v3d.md`'s counted frames (frames 2 to 20,
wall); the waits are the logs' `gm2: v3d-waits` and `gm2: v3d-frames`
counts divided by `counted=19`. Both scenes ran with no GL error and no
refused draw, and the TFU made every chain (5 a frame in terrain, 1 in
refract) in both runs.

| scene | ms a frame, before | after | `glGenerateMipmap` ms a frame, before | after | waits in `glGenerateMipmap` a frame, before | after | waits in `eglSwapBuffers` a frame, before | after | waits a frame overall (= jobs a frame), before | after |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| terrain | 1.231 | 1.324 | 0.105 | 0.125 | 6.0 | 5.0 | 0.95 | 1.89 | 9.0 | 9.0 |
| refract | 0.470 | 0.495 | 0.032 | 0.034 | 2.0 | 1.0 | 0.95 | 1.89 | 3.0 | 3.0 |

What it shows:

- The wait for the render that stores the mipmapped image is gone from
  `glGenerateMipmap`: one wait a frame fewer in each scene. The waits that
  stay in the call are `flush_job`'s for the job submitted before the
  render (the back end keeps one job in flight, `flush_job` "the job
  before has run before this one is submitted"): in terrain that is the
  previous call's render, which main had already waited for inside that
  call. So on the board the CPU now records the next render while the
  previous one runs, instead of waiting for each render as soon as it is
  submitted.
- The overall count does not fall (9 and 3 a frame, one per job): with one
  job in flight every job is still waited for once, later. The wait main
  made at the end of `glGenerateMipmap` now falls at the next submit, here
  the swap's (`eglSwapBuffers` 0.95 to 1.89 a frame).
- The milliseconds do not move beyond the host's noise: the Linux OSAL
  completes every submit inside `submit()`, so a wait costs the host
  nothing and the wait count is the host's metric. The setup phases of the
  same runs differ by more (terrain 104.9 ms before, 156.0 ms after, in GL
  calls) than the frames do.
- The frame-1 counters of the after run: terrain `1 chains (0 queued
  behind a job, 0 waited), refused: ... unknown 13` (no render image for the
  texture, `rt_find`; all in the setup's calls, frame 2's line has the
  same 13 beside 6 chains); refract `0 chains ... unknown 1`. No job was
  queued here: the Linux OSAL has completed the job before the TFU job is
  asked for, so it runs at once and returns no fence.

What the change is for is the board: there each of those waits is a real
render's time. The Pi 4 session measures terrain's and refract's ms a
frame.

## On the board: glmark2 from main `c4af4917`, 2026-10-22

The board check of the TFU no-wait above and of the V3D and GL changes that
followed it on main: the FS uniform dump off (`8d7553a3`), the V3D reading
uniforms from the host arrays only (`c4af4917`), lazy raster (`361f4cbf`),
the deferred depth storage (`897ead62`), the copy reason counters
(`cb33efd5`), the UIF walker (`8e05a17e`) and the buffer retired reuse
(`55b7486c`). The GM image of `cts-common.sh pi4 build` from main
`c4af4917` (`CONFIG_YAKOGL_V3D_TFU=y`, 96 MiB carve-out; the main tip of
the day, `71c73b0f`, differs only in `tools/check.sh` and its selftest),
image sha256 `2e41f5e9...`, one tryboot with the lab's 17-field
`hdmi_timings`, window 800x600, EGL double buffer, one GL client on the
board. Record (gitignored): `records/pi4/20261022-0308-glmark2-c4af4917/`.
The image was staged and the console read by hand with the same steps as
`board.sh pi4 run`. The console lines are the image's own; the firmware and
Linux lines around them are left out. `board.sh pi4 tally` gives the
verdict from the run's files:

```
board.sh pi4 GM rc=0 verdict=done boot=tryboot row boot_verdict=complete result=record:GLMARK2 resets=0 record=594b1201,fc3f00c3,6184003f,00258320 back_after=?s unsupported=0
glmark2: score 195: 33 scenes ran, 0 failed, 0 unsupported (33 list entries); window 800x600, shown yes, GPU resets 0
glmark2: GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)
```

Before is the 193 run of `445c5653` (RESULTS.md; terrain 179.26 there,
178.86 and 178.94 in the two runs before it) and, for the gl and swap
columns, the `dcbfb30b` run above. After is `common-tally.py --gm-table`
over the console. Only scenes that moved by more than 1.1 percent are
listed:

| scene | before ms | after ms | change | gl ms before -> after | swap ms before -> after | jobs a frame | end waited |
|---|---:|---:|---:|---|---|---:|---|
| **terrain** | 179.26 | 161.79 | -9.7% | 173.94 -> 156.06 | 4.88 -> 5.62 | 9.0 | 62 of 62 |
| **shadow** | 7.32 | 7.34 | +0.3% | 0.97 -> 0.97 | 6.33 -> 6.36 | 3.0 | 1364 of 1365 |
| **refract** | 18.72 | 17.27 | -7.7% | 11.59 -> 1.81 | 7.17 -> 15.42 | 3.0 | 581 of 581 |
| build use-vbo=false | 6.42 | 4.75 | -26.0% | 2.73 -> 1.02 | 3.65 -> 3.72 | 1.0 | 2 of 2123 |
| desktop blur | 13.06 | 11.18 | -14.4% | 7.11 -> 5.55 | 5.91 -> 5.61 | 11.0 | 896 of 896 |
| desktop shadow | 6.87 | 6.51 | -5.2% | 0.83 -> 0.85 | 5.98 -> 5.65 | 3.0 | 1538 of 1538 |
| buffer subdata | 10.03 | 8.97 | -10.6% | 5.49 -> 4.43 | 4.51 -> 4.52 | 1.0 | 1 of 1116 |
| ideas | 11.85 | 12.40 | +4.6% | 7.59 -> 7.71 | 4.45 -> 4.69 | 1.0 | 1 of 813 |

Score 193 -> 195; the 33 scenes sum 370.50 -> 347.45 ms a frame.

- **terrain: 179.3 -> 161.8 ms.** The uniforms step of its `glmark2-steps`
  line went from 7.8 ms a frame to 9 us and no `fs uniform[` line was
  printed (the FS dump is off): that is 7.8 ms of the 17.5. The rest of
  the GL time, 156 ms, is still outside the timed steps (their sum is
  0.17 ms). The counters are cumulative since boot and printed only when
  an arena grows, so a scene's share is the difference of two printed
  lines. From the line before terrain to shadow's frame-1 line (terrain's
  62 frames plus shadow's set-up and first frame): 184 TFU chains, every
  one queued behind a job and 0 waited (1 by terrain's frame 1, then 3 a
  frame), 135 `unknown` refusals (13 by terrain's frame 1); tiled copies
  24 `plain` (all by terrain's frame 1) and 201 `first-sample` (18 by
  frame 1, then 3 a frame), 245 `image-store` copies, 1 `upload`. The
  CPU waits are one per job: 9 jobs a frame (560 over 62), and the swap
  waits in every frame. The GPU work of frame 10 is 29.1 ms (8 jobs; 4 TFU
  jobs 1.8 ms of it). So about 130 ms of the 161.8 ms is CPU time that no
  step lap times, not GPU time and not the TFU wait (#1209).
- **refract: 18.7 -> 17.3 ms.** GL time fell from 11.6 to 1.8 ms and the
  swap rose from 7.2 to 15.4 ms: the wait for the render that the TFU
  chain reads left `glGenerateMipmap` and now falls at the swap, as the
  host predicted. The frame is now its 3 jobs' GPU time (16.8 ms at frame
  10, the third with a 3.2 ms TFU job). From shadow's frame-1 line to
  refract's (shadow's other frames, refract's set-up and first frame) the
  counts grow by no TFU chain, 1 `unknown`, 1 `first-sample` copy and 1
  `upload`; no line is printed after refract's frame 1, so its later
  frames are not in the counts.
- **shadow: unchanged at 7.3 ms**, at the GPU floor of its 3 jobs (7.2 ms
  at frame 10). Its depth texture is sampled from the depth buffer (the
  line "no CPU resolve, no tiled copy" again).
- **The carve-out high-water** of the whole run is 69,092,096 of
  100,134,912 bytes, reached in refract (76,772,096 at `dcbfb30b`): the
  7,680,000 bytes of the R32F depth storage that is no longer allocated.
- **Other scenes.** build use-vbo=false 6.42 -> 4.75 ms (GL time 2.73 ->
  1.02 ms, the client-array copy), desktop blur 13.06 -> 11.18 ms, buffer
  subdata 10.03 -> 8.97 ms, desktop shadow 6.87 -> 6.51 ms. Buffer map
  (10.12) and interleaved (8.80) did not move. ideas rose to 12.40 ms
  (+4.6 percent, one run; it read 11.34 to 12.04 in the four runs before).
  Every other scene is within 1.1 percent of `445c5653`.
- Pictures: none black, none absent; 31 of 33 match the `pi4` column of
  `tools/cts/glmark2-pictures.txt`. The other two differ in one probe each
  (desktop blur tr, jellyfish c), both animated scenes.

## What the host does not show

- GPU waits: the job waits before a depth resolve and before a TFU mipmap,
  and the swap's wait for its frame (#925). Only the board shows them.
- The board's memory: storage, copies and images are non-cacheable on the
  Pi 4, malloc'd here. The CPU tiling and filtering rows are relatively
  more expensive on the board.
- The TFU's own time: the host runs a model of it (above), whose time is
  not the board's.
