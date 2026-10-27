<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP glmark2: CPU time of the GL calls on the GC7000 host build

The i.MX8MP glmark2 score is 42 (`tools/cts/RESULTS.md`, "Common set"). Until
now only the frame floor (`docs/notes/imx8mp-frame-floor.md`: a VSYNC wait
under every show) was looked at as a cause. This note is the host profile of
the GL-call time of the 33 scenes, as the Pi 4 got one
(`docs/notes/pi4-glmark2-cpu.md`). The numbers per scene are in
`tools/hostharness/glmark2/FINDINGS-gc.md`. Investigation only: no driver
code changed, nothing ran on a board.

## How it was measured

The glmark2 host harness, vivante build, `GM2_PROFILE=1` and `GM2_PERF=1`,
20 frames, with the glmark2 image's 240 MiB carve-out (commands and
settings: FINDINGS-gc.md, "How it was run"). Host time on an x86-64
desktop: GPU time and waits are not in it, cache maintenance is a no-op, and
the memory is cached. The board's Cortex-A53 reads and writes the GPU
carve-out as Normal non-cacheable memory, so every CPU pass over a texture
costs several times the host's figure. The ranking carries over, the
milliseconds are a lower bound. How many times slower the board is was not
measured here.

There is no per-scene board number to compare with: the i.MX8MP has no
console, and its record carries the score only (42, mean of the scenes'
fps). The comparison below is therefore with the frame floor's model
(17.0 ms a panel period, a scene at 58 or 29 fps or lower), not with a
scene's fps. The i.MX8MP record of the imx8mp-gm-record item would give
that number.

## Which scenes the CPU matters for

| class | scenes | host ms a frame | verdict |
|---|---|---|---|
| light | 23 of 33 (everything not named below) | 0.02 to 0.08 | the frame floor alone explains the CPU side. A draw is built in 2 to 7 microseconds; even at 10 times the host's time the frame's GL calls are under 1 ms against a 17.0 ms period. What decides these scenes on the board is the wait for the latch, the copy of 1.92 MB, and the GPU. |
| mid | `desktop` blur 0.28 and shadow 0.47, `buffer` 0.35 / 0.41 / 0.52, `ideas` 0.60, `build:use-vbo=false` 0.46 | 0.28 to 0.60 | CPU is a small part. For `desktop`, `buffer` and `ideas` most of it is glmark2's own code (mesh updates, matrices: 0.17 to 0.42 ms of it), not the driver. Under the floor at any plausible factor. |
| heavy | `refract` 12.4, `terrain` 9.0, `shadow` 2.2 | 2.2 to 12.4 | CPU in the GL calls is part of the board time. At 4 times the host figure (an assumption, not measured) refract is about 50 ms, terrain about 36 ms and shadow about 9 ms of CPU in GL calls; the first two are already two to three panel periods before any GPU time. Shadow's CPU is on its own under one period, so its board time is the floor plus GPU plus the depth resolve's wait for the GPU (below). |

So the frame floor explains the 23 light scenes (and decides the score of
the 7 mid ones), and CPU time in the GL calls is part of the board time of
three: refract, terrain and, less, shadow. The arithmetic of the score says
how much that is worth: the score is the mean of 33 fps values, so raising
one scene from about 5 fps to 29 adds under 1 point; the three heavy
scenes together are worth about 2 to 3 points of 42 once the floor is gone
(estimate). The floor, not the CPU time, sets the score. The CPU fixes
matter for the three scenes, for the cluster-style sample drawing the same
way, and once the floor is lifted.

## The main CPU costs, with the code

### 1. `glGenerateMipmap` on the CPU: terrain 8.3 ms (92%), refract 10.3 ms (83%)

- Terrain makes 5 calls a frame (glmark2's `BaseRenderer::update_mipmap`:
  the terrain target twice, `bloom_h`, `bloom_v`, `tilt_h`); refract makes
  one, on its 1600x1200 colour target.
- The GC back end has no hook for it: `src/vivante/backend.rs:3039`
  `generate_mipmap: None`. So `src/gl/texture.rs` (`glGenerateMipmap`, the
  `backend::generate_mipmap` branch at line 3828 and the box filter after
  it, lines 3839 to 3951) filters every level on the CPU: per texel, per
  channel, with `box_src` index arithmetic and bounds-checked indexing
  into the texel slice, then `osal::cache_flush` over the whole texture.
- Perf: 91.5% of terrain's frames and 82.8% of refract's are the function's
  own samples. On the board the same loops read and write non-cacheable
  memory; this is the largest board cost of both scenes.
- The same shape as the V3D's (`docs/notes/pi4-glmark2-cpu.md`), with one
  difference: the V3D had a TFU path that did not run; the GC7000 has no
  GPU path at all.

### 2. The depth resolve: shadow 2.1 ms (96.5%), refract 2.1 ms (16%)

- The ground's `glDrawArrays` samples the shadow map, a depth texture.
  `src/gl/draw.rs:3147` `resolve_sampled_depth` -> `src/gl/texture.rs:694`
  `resolve_depth` -> `src/vivante/backend.rs` `gc_depth_resolve` ->
  `src/vivante/ds_cache.rs:477` `resolve_depth` -> `ds_retention::write_depth_texels`.
  It reads the D24S8 surface (row-linear, one word a texel), converts each
  word with `unorm24_depth` and writes an R32F texel, through a closure per
  texel with a checked slice and `try_into` (perf: closure 49%,
  `unorm24_depth` 33%, `write<f32>` 29%, index 16.5%).
- On the board there are two more costs that the host cannot show:
  `resolve_depth` (`texture.rs:711`) first `flush()`es and `finish()`es when
  the kick that rendered the depth is still pending, so the CPU waits for the
  GPU once a frame; and the read is from non-cacheable memory.
- Unlike the V3D, the GC does not tile the R32F texture again: the plain 2D
  depth texture is sampled linear (`wanted` in `tiled_copy.rs` is false for
  it), so the V3D's second pass does not exist here.

### 3. A tiled copy of a vertex-sampled texture: terrain 0.3 to 0.45 ms

`yakogl::vivante::tiled_copy::tile` (`src/vivante/tiled_copy.rs`, `prepare`
-> `refresh`, called from `draw_one`): every plain 2D texture the vertex
stage samples is tiled (`wanted_vs`, line 140). Terrain samples its
height/normal map in the vertex stage, and the copy is rebuilt whenever the
texture's `content_gen` or `target_kick` changes (`key_of`). That it is made
again every frame (3% of the frame's samples) says the texture is rendered
into each frame; which texture it is was not checked.

### 4. Smaller

- `build:use-vbo=false`, 0.44 ms: client arrays are converted and copied into
  the frame arena at every draw (`yakogl::gl::draw::attrib_row`,
  `convert_attrib`, `memmove`). That is what the scene asks for; not a bug.
- `buffer` with `update-method=map` and interleave, 0.34 ms:
  `glMapBufferOES` -> `map_range` (`src/gl/buffer.rs:840`) makes a fresh GPU
  allocation and copies the buffer (`cache_invd`, `copy_nonoverlapping`,
  `cache_flush`) at every map. The host figure includes the page faults of
  a new malloc; the board's allocation and copy are different. Unchecked.

## Not in the host profile

- The frame floor itself, the copy in `board_show` and the GPU time of
  every scene (`imx8mp-frame-floor.md`).
- GPU waits: the depth resolve's `finish()`, and `gc_finish_previous`
  at every swap.
- The cost per CPU pass on the board (several times the host's).
- A picture check: the host renders nothing.

## Fix items, in order

0. **The frame floor (#737), not a CPU item.** It decides 23 of the 33
   scenes and most of the score. Ordered here so the CPU items are read in their
   place: expected gain is the largest of all (the score is capped near 58
   by the wait), and it is owned by `imx8mp-floor-gm-arms` and
   `imx8mp-wait-vblank`.
1. **A GPU `glGenerateMipmap` for the GC7000** (set `generate_mipmap` at
   `src/vivante/backend.rs:3039`; the GL core offers the chain at
   `src/gl/texture.rs:3828` and records the result as `chain_in_backend`).
   The target is linear storage in the GC, so the chain can be drawn level by
   level with a quad, as the PowerVR's does (`src/pvr/backend.rs`
   `pvr_generate_mipmap`). Gain on the host: terrain 9.0 -> about 0.7 ms,
   refract 12.4 -> about 2.1 ms (the CPU box filter is 92% and 83%). On the
   board more, since the filter reads non-cacheable memory. Most of the
   three scenes' gain. Needs a board check of the picture (a wrong level shows
   as shimmer). **Done on the host** (2026-10-18, `src/vivante/mipmap.rs`;
   2026-10-20, the GL core's offer of glmark2's rendered-into chains, #1106;
   the sections "After: the GPU chain" and "After: the GL core's offer"
   below). The board check is what is left.
2. **The depth resolve** (`ds_cache.rs:resolve_depth`). Cheap step first:
   convert row by row with plain slices (no closure and `try_into` per
   texel, no checked index): the loop is 2 ms for a texture of which only
   the conversion is needed. Estimate 2 to 4 times faster; not tried.
   Real fix: do not read the depth on the CPU, sample the D24S8 surface or
   convert it with a draw; that also removes the CPU's wait for the GPU.
   Gain on the host: shadow 2.2 -> about 0.1 ms, refract -2.1 ms. Needs the
   texture's depth format to be one the GC's texture unit reads: this was
   not known: `src/gl/texture.rs:680` says the TPU's reading of the depth
   surface was "not tried on either board".
3. **Terrain's tiled copy** (`tiled_copy.rs`): find which texture is
   copied each frame, and keep the copy while its content is unchanged.
   Gain: up to 0.45 ms of terrain's 9.0, and it matters only after item 1.
4. **Optional, scenes that ask for it:** the arena copy of client arrays
   (`build:use-vbo=false`) and the per-map allocation of `glMapBufferOES`.
   Gain under 0.5 ms each; do after the floor.

Order by gain: item 1 (about 18 ms of the 23 ms of host time of terrain and
refract), item 2 (about 4 ms), item 3 and 4 (under 1 ms each).

## After: the GPU chain (item 1, 2026-10-18)

`src/vivante/mipmap.rs` answers `generate_mipmap` for a 2D RGBA8 chain in
either channel order: each level above 0 is one draw through `draw_one`, a
frame of its own into the level (a linear RGBA8888 target at the level's
address and the chain's pitch), one full-screen quad with the quad vertex
program and the new internal fragment program `mipgen_ps`
(`tools/shaders/internal/es/mipgen.frag`, row `GC_INTERNAL_MIPGEN_PS` of
`src/vivante/internal.rs`), the level above bound alone as a one-level
bilinear CLAMP_TO_EDGE texture and sampled at the centre of each texel. The
levels land where the texture unit walks them (`texture.rs`
`build_descriptor`: one LINEAR_STRIDE, each level on a 64-byte boundary),
so `return_mipmap_chain` is a wait for the chain's last kick. The level
above is always bound B,G,R,A, the order the pixel engine writes: a
rendered-into texture (`pe_order`) is sampled right, an uploaded R,G,B,A one
is read and written back swapped twice, each byte where it was read from.
Left to the CPU filter, counted by `gc_backend_mip_chains`: other formats,
cube and array stacks, a layout the back end does not walk, a chain past
the 32-bit address space, a refused draw. A GPU-made chain also stales the
tiled copies of its storage and a later copy waits for its kicks
(`tiled_copy::storage_rewritten`, `mipmap::chain_kick_pending`): the GL
core's stamps, which the copies are keyed by, do not move for it.

**What the host measured** (`tests/gc_mipmap_gpu.rs`, the real GL core on
a stand-in of the back end's shape for the CPU reference and on the real
back end over a stand-in OSAL whose `submit` executes each stream as the
GPU would; `cargo test ... --test gc_mipmap_gpu -- --nocapture`, main
`3adf39e3` plus this change, 2026-10-18):

| chain | levels | GPU levels against the quad's filter of the level above | level 1 against the CPU box filter | CPU box filter, whole chain | CPU to record the GPU chain's draws |
|---|---:|---:|---:|---:|---:|
| 800x600 (terrain target) | 10 | within 1 LSB | within 1 LSB | 11.1 ms | 0.011 ms |
| 1600x1200 (refract target) | 11 | within 1 LSB | within 1 LSB | 44.5 ms | 0.011 ms |
| 64x64, 32x64, 64x1 | 7 | 0 LSB | 0 LSB | 0.1 ms or less | 0.01 ms |

Host time of this test's build (the test profile, one cold call each; the
NPOT chains' storage growth, `grow_npot_chain`, is inside the CPU figure),
so the milliseconds are not the harness's; the ratio is the point: the CPU
side of a chain goes from the box filter of every texel to the recording of
nine or ten draws. The GPU's time to draw the levels is not on the host; the
board measures it. Over the exactly halving levels the GPU chain is within
2 LSB of the CPU chain as a whole (each path rounds its own .5 cases; dEQP
compares a level with the result's level above, which is the 1 LSB column).

That change alone did not reach glmark2: the GL core offered a chain only
at `chain_pitch` 0 and not `pe_order`, which no GC7000 chain is (next
section).

## After: the GL core's offer (item 1, #1106, 2026-10-20)

`src/gl/texture.rs` `glGenerateMipmap` offered a chain to the back end only
when `t.chain_pitch == 0` and `!t.pe_order` (from the V3D's TFU change
`c6f17516`). On the GC7000 every chain is at level 0's pitch
(`mip_level_align` 64, `chain_pitch` never 0) and glmark2's targets are
`pe_order` (rendered into every frame), so the back end's entry was never
reached from GL. The two conditions are gone; the bind already carries the
pitch (`stride_texels`) and the order (`bgra`), which
`gc_generate_mipmap` handles. The PowerVR and the V3D never have either
(neither sets `mip_level_align` nor `rt_rgba8888_is_bgra`), so their offer
is the one they had; an uploaded `GL_BGRA_EXT` / XRGB image (`t.bgra`)
still keeps the CPU filter. `tests/gc_mipmap_gpu.rs`
`glmark2s_rendered_into_chains_are_offered_to_the_gpu` pins it: an RGBA8
texture rendered into through a framebuffer, then glGenerateMipmap, two
frames, at 64x64, 800x600 and 1600x1200: the chain is the back end's
(`chain_in_backend`), counted, one kick per level above 0, every level
within 1 LSB of the quad's filter of the level above and level 1 within
1 LSB of the CPU box filter.

**Host harness, before and after** (the GC7000 build, `GM2_PROFILE=1
GM2_FRAMES=20`, entries `terrain` and `refract`, 800x600, the carve-out
`YAKOGL_GC_CARVEOUT_BYTES=245642240` run.sh derives; ms a frame over the
counted frames 2 to 20, wall, the median of three runs, before and after
interleaved; an x86-64 host, not the one of FINDINGS-gc.md, so the
before column is not that file's):

| entry | frame before | frame after | glGenerateMipmap before | after | glDrawArrays before | after |
|---|---:|---:|---:|---:|---:|---:|
| terrain (5 calls a frame) | 11.39 | 0.87 | 10.32 | 0.054 | 0.40 | 0.20 |
| refract (1 call a frame) | 13.79 | 2.20 | 10.95 | 0.013 | 2.28 | 1.96 |

Runs (frame ms): terrain before 11.39, 10.79, 11.44, after 0.87, 0.97,
0.85; refract before 15.92, 13.79, 13.28, after 2.20, 3.11, 1.76.
Before: main `b4e5f46e` (its library sources are `4b057224`'s). After: the
same plus the `src/gl/texture.rs` change of #1106. Every after run logs
"the GPU made the ... chain" and no chain left to the CPU; 0 GL errors,
0 refused draws, 0 allocations refused in all twelve runs. What is left in
glGenerateMipmap is the recording of the chain's draws. Refract's
remaining frame is glDrawArrays, the depth resolve (item 2), which varies
from run to run (1.5 to 2.8 ms after).

How it was run: `tools/hostharness/glmark2/run.sh` could not be started
in this session (the permission layer refused the script and cmake), so
its steps were done by hand with the same sources and flags: glmark2
`22c527cb` compiled as run.sh compiles it (shared-library.cpp's `dl*`
renamed by `-D` instead of objcopy), `host_main.cpp`, `shim.c` with
`gen_timed.py`'s wrappers, `src/osal/linux.c`, and the `yakogl` crate with
`osal-linux,gpu-vivante,shaderc-online,report-es2` linked as an rlib into
the program (release, opt-level 2, no LTO) instead of the assembled
`libyakogl.a`. The milliseconds therefore compare with each other, not
with FINDINGS-gc.md's.

**Board check needed** (#1206): terrain and refract pictures, dEQP-GLES2
`texture.mipmap.*`, and the per-scene ms a frame on the i.MX8MP.

## After: buffer map reuse (item 4, map half, 2026-10-18)

`5ed9e1c2`: a write map of a buffer that a pending kick reads no longer allocates at every map. The buffer keeps the storages it moves off (two per buffer, 16 in all, `src/gl/buffer.rs` "Retired storages") and takes one back once its kicks are done; with the window's one frame in flight it cycles through three storages, so after the first two maps no map allocates. A busy `glMapBufferRange` with `GL_MAP_INVALIDATE_RANGE_BIT` copies only the bytes outside the mapped range. A busy `glMapBufferOES` still copies the whole buffer (OES_mapbuffer keeps every byte the caller does not write). `tests/gl_map_buffer_reuse.rs` pins both cases.

Host harness, GC7000 build (`GM2_PROFILE=1 GM2_FRAMES=200 GM2_ENTRIES='^buffer'`, two runs each, ms a frame, wall):

| entry | frame before | frame after | glMapBufferOES before | after |
|---|---|---|---|---|
| map, interleave=true | 0.516, 0.532 | 0.235, 0.194 | 0.341, 0.351 | 0.030, 0.023 |
| map, interleave=false | 0.369, 0.341 | 0.351, 0.352 | 0.040, 0.038 | 0.027, 0.028 |
| subdata (path not changed by `5ed9e1c2`) | 0.419, 0.383 | 0.423, 0.384 | - | - |

So the 0.34 ms of FINDINGS-gc.md was the new allocation at every map, not the copy; what is left in glMapBufferOES is the copy. Not run on a board.

### Subdata after `55b7486c` (busy glBufferSubData takes a retired storage)

`detach_from_pending_kick` takes back a retired storage and copies only the ranges written since (`tests/gl_buffer_retired_reuse.rs`: no allocation after the first `lag + 1` frames). Same harness command (`GM2_PROFILE=1 GM2_FRAMES=200 GM2_ENTRIES='^buffer'`, GC7000 build), run on two trees that differ only by this commit (`55b7486c~1` and `55b7486c`), before and after alternating on one machine, two runs each; `update-method=subdata`, interleave=false, ms a frame, wall:

| entry | frame before | frame after | glBufferSubData before (40 calls a frame) | after |
|---|---|---|---|---|
| subdata, interleave=false | 0.438, 0.434 | 0.393, 0.470 | 0.060, 0.064 | 0.037, 0.044 |

The update calls drop by about a third (0.062 to 0.040 ms a frame). The frame does not show it: glmark2's own time is 80% of the frame and moves with the host's load by more than the change (the map rows, which this commit does not touch, read 0.356 to 0.455 ms across the same four runs). The host's allocation is cheap (the OSAL reuses freed memory), so this harness understates what the board saves. The ignored timing test in `tests/gl_buffer_retired_reuse.rs` (`docs/notes/gl-buffer-update.md`, "Done") isolates the calls: 0.055 to 0.024 ms a frame, 800 to 0 allocations in 200 frames. Not run on a board.

## On the board, after the GC mipmap offer (main `a917d123`, 2026-10-20)

The per-scene record (`tools/board.sh imx8mp gm ... pictures`, 66 boots, all
`verdict=done`; `records/imx8mp/20261020-2100-glmark2-a917d123/`) of an
image that holds `dc033dc44` (the GL core offers glmark2's rendered-into
RGBA8 chains to the GC7000 path, #1106). The score run of the same image
reads 42, 33 ran, 0 failed, 0 GL errors.

| scene | ms a frame | FPS (integer) |
|---|---:|---:|
| terrain | 250 | 4 |
| shadow | 500 | 2 |
| refract | 500 | 2 |

ms a frame is 1000 / the integer FPS, so a scene is up to one FPS step
faster than it reads. All 33 scenes sum to 1934.56 ms (1932.65 at
`445c5653`): at this resolution the offer moved no scene, and terrain,
shadow and refract are as before the offer. The page record carries no
`gc_backend_mip_chains` count and no CPU-refusal counters (no console), so
whether the chains now reach the GPU path on the board is not shown by this
run; a run that prints the frame-1 arena line would show it. The pictures
match the earlier column in all 33 scenes by value (terrain mean 119 129 45,
refract 22 18 29), not looked at by eye.

## What would settle the open points

- A per-scene board number (fps, or ms a frame) for terrain, refract and
  shadow, from the `imx8mp-gm-record` item or the frame-breakdown record
  in `imx8mp-frame-floor.md`: it gives the real board/host factor and checks
  the "at 4 times" estimates above.
- Which texture `tiled_copy::tile` copies in terrain.
