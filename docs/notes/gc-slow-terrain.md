<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: where glmark2 terrain's 250 ms a frame go (host breakdown)

Plan #328, item bd-terrain (issues #1204, #1106; ROADMAP.md stage 2, "Slow
scenes"). Host only: nothing ran on a board, no code changed. Code is cited
at `964f50f0`; glmark2 at the pinned `22c527cb` (`$GLMARK2_SRC`).
`docs/notes/imx8mp-glmark2-gap.md` is the earlier desk study of the same
question; this note replaces its terrain half with counts taken from the
running code.

## Answer in short

- **Terrain is GPU-bound.** The frame is 52 kicks. At the GPU cost
  calibrated below (about 0.6 ns an instruction a pixel or vertex, 3.9 ns a
  pixel at least), its passes take **about 140 to 205 ms of GPU time**: the
  simplex-noise height map (840 instructions over 256 x 256) about 33 to
  44 ms, the terrain pass (393 216 vertices of 56 instructions, then
  148 instructions a pixel with no early depth) about 50 to 75 ms, the two
  tilt-shift passes about 44 to 58 ms. The other passes, the 8 clear quads
  and the 43 mipmap levels make the rest.
- **What the CPU adds in series is about 15 to 30 ms.** The tiled copies
  of the height and normal maps come with a `finish()` in the middle of the
  frame: about 4.5 ms. The CPU starves the GPU through the 43 one-level
  kicks and wakes late from the 1 ms polls on the 4-slot ring: about 7 to
  10 ms. The rest is recording the passes while the GPU is idle. The
  present's 12.5 ms window copy runs while the GPU draws the last pass, so
  it costs nothing on the critical path.
- **The CPU mipmap fallback is not taken on the board.** The board record
  went from 1000 ms (1 FPS) to 250 ms (4 FPS) at the one commit that offers
  the chains to the GPU (`dc033dc4`), see "The mipmap path on the board". The
  CPU in GL calls is 0.41 ms a frame on the host, a few ms on the A53, and
  most of it overlaps the GPU. No bounded wait on the path is short enough
  to set the frame time.
- A model of the board's timeline with these costs gives **160 to 236 ms a
  frame**, against the board's 200 to 250 ms (4 FPS). The largest
  uncertainty is the GPU's cost per instruction for terrain's own shaders.

## How it was measured

- **Host profile:** `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out>
  vivante` with `GM2_PROFILE=1 GM2_FRAMES=20 GM2_WRITE=0 GM2_ENTRIES='^terrain$'`
  and `YAKOGL_GC_CARVEOUT_BYTES=245642240` (the run's `gc-settings.txt`).
  The sandbox of this session refused command lines that set environment
  variables, so `run.sh` sourced those settings from a file instead (a
  temporary line, removed). x86-64 host, wall time; the host's submits
  complete at once (`tools/hostharness/glmark2/README.md`, "Limits").
- **Kicks, waits and CPU passes:** temporary `glog!` lines, not committed,
  in `src/vivante/backend.rs`, `tiled_copy.rs`, `mipmap.rs` and `ds_cache.rs`:
  - one line per draw: target, viewport, vertex count, the shaders'
    instruction counts (`usc_dw / 4`) and their texture loads (opcodes
    0x18 to 0x1b);
  - one line per `frame_open` (the slot and the fence it would wait for)
    and per `frame_submit` (stream words, arena bytes);
  - one line per `finish()`, `gc_finish_previous`,
    `gc_return_mipmap_chain` and GPU chain;
  - one line per tiled copy (`refresh`: bytes, and whether the target's
    kick was pending);
  - one line per depth copy, seed or resolve.

  Entries `terrain` (5 frames) and, for the calibration, the 27 non-terrain
  entries of the default list up to `loop` (4 frames). The tree was put back
  afterwards; `git status` is clean apart from this note.
- **GPU cost:** no GPU time exists on the host, so each kick's time is
  estimated from its pixels and instructions with the model in "The GPU
  calibration". The board's timeline is modelled from those estimates
  (`build/` scripts of this session, not committed; the rules are in
  "The board timeline model").

## Per frame

### The 52 kicks

Steady-state frame (the third counted one; every counted frame is the same).
The target is the frame the kick renders into. "est" is the GPU estimate at
0.6 ns an instruction (`fsi`: fragment instructions, `vsi`: vertex
instructions, `t`: texture loads) and 3.9 ns a pixel at least, over the
whole target. Clears are full-target quads (`src/vivante/backend.rs:2345-2348`
`emit_clear`, no tile-status fast clear) at 3.9 ns a pixel.

| kicks | pass (glmark2) | target | work | est ms |
|---|---|---|---|---:|
| 1 | `canvas_.clear()` (the window) | 800x600 | clear quad only; tilt_v clears the window again | 1.9 |
| 1 | height map, `SimplexNoiseRenderer` (`terrain-noise.frag`, 4 octaves of 3D simplex noise) | 256x256 | clear + quad, **fsi 840**, 0 t | 33.3 |
| 1 | normal map, `NormalFromHeightRenderer` | 256x256 | clear + quad, fsi 25 / 3 t | 1.2 |
| 1 | terrain, `TerrainRenderer` (`terrain-renderer.cpp:243` `make_grid(256, 256, ...)`: 6 vertices a cell, no index) | 800x600 + depth16 | clear + **393 216 vertices, vsi 56 / 2 t** (the height and normal maps in the vertex stage) + fsi 148 / 5 t, depth test, culled | 57.7 (vertex 13.2, fragment 42.6 at full cover, clear 1.9) |
| 9 | its mipmap chain (`base-renderer.cpp:84-91`) | 400x300 .. 1x1 | one quad a level, fsi 4 / 1 t | 0.6 |
| 1 | bloom_h, `BlurRenderer` | 256x256 | clear + quad, fsi 42 / 5 t | 1.9 |
| 8 | its chain | 128x128 .. 1x1 | fsi 4 | 0.1 |
| 1 | bloom_v | 256x256 | clear + quad, fsi 47 / 5 t | 2.1 |
| 8 | its chain | 128x128 .. 1x1 | fsi 4 | 0.1 |
| 1 | overlay into the terrain target (`overlay-renderer.cpp:72-90`) | 800x600 | quad, fsi 2, blend | 1.9 |
| 9 | the terrain target's chain again (`overlay-renderer.cpp:89`) | 400x300 .. 1x1 | fsi 4 | 0.6 |
| 1 | tilt_h | 800x600 | clear + quad, fsi 66 / 9 t | 20.9 |
| 9 | its chain | 400x300 .. 1x1 | fsi 4 | 0.6 |
| 1 | tilt_v into the window | 800x600 + D24S8 | clear + quad, fsi 75 / 9 t, depth test | 23.5 |
| **52** | | | **8 clear quads, 9 passes, 5 GPU chains (43 levels)** | **146** at full cover |

Every kick is a frame of its own: a new target submits the open one
(`draw_one` -> `frame_open`), and `gc_generate_mipmap` draws each level as
its own kick (`src/vivante/mipmap.rs:270-277`) and then flushes
(`:278`). The terrain mesh does not cover the whole target: the far edge
of the 6000-unit plane is at about 80 % of the screen's height from the
bottom (camera `scene-terrain.cpp:276-278`, 40 degree field of view, far
plane 4000). With that cover and the hills' overdraw put at 0.8 to 1.2,
the terrain pass is 49 to 66 ms and the frame's GPU work 139 to 156 ms at
0.6 ns; at 0.8 ns it is 181 to 204 ms.

No early depth is possible on these passes: every colour target is linear,
and the rasteriser's early depth is off for linear targets
(`src/vivante/backend.rs:1754-1783`). So the terrain fragment shader runs on
every rasterised fragment, hills behind hills included.

### (1) GL-call CPU time, per entry point

Host, 19 counted frames, wall: **0.522 ms a frame, 0.406 ms of it in GL
and EGL calls** (glmark2 itself 0.117 ms).

| entry point | calls a frame | ms a frame |
|---|---:|---:|
| `glDrawArrays` | 8 | 0.241 |
| `glGenerateMipmap` | 5 | 0.136 |
| `glClear` | 8 | 0.004 |
| `glBindTexture` | 18 | 0.003 |
| `glUseProgram` | 18 | 0.003 |
| `glVertexAttribPointer`, `glActiveTexture`, `glBindBuffer`, `glEnable/DisableVertexAttribArray`, `glBindFramebuffer`, `glViewport`, `glDepthMask` | 8 to 13 each | 0.001 to 0.002 each |
| `eglSwapBuffers` | 1 | 0.001 |
| everything else | | under 0.005 together |

Draw steps (`src/draw_steps.rs`): 60 draws a frame, 5.9 us a draw; the
largest step is `GcTiledCopy` (3.2 us a draw on average, all of it in the
terrain draw's two copies), then `GcEmit` (1.0 us). On the A53 this is a few
ms a frame. Almost all of it overlaps the GPU (next sections). Only the part
the GPU waits for counts, and that part is counted in (3).

### (2) CPU passes over the carve-out

The carve-out is Normal non-cacheable on the board
(`driver/src/vivante/gc_render.c:66-92`). Bytes a frame:

| pass | call site | bytes read | bytes written | count a frame |
|---|---|---:|---:|---:|
| tiled copy of the normal map (256x256 RGBA8, 1 level): `cache_invd` of the source, zero fill of the copy, the tile-row copy, `cache_flush` of the copy | `src/vivante/tiled_copy.rs:426-429`, from `prepare` (`:346`) in `draw_one` (`src/vivante/backend.rs:2872`); `wanted_vs` (`tiled_copy.rs:174`) | 262 144 | 524 288 (zero fill + copy) | 1 |
| the same for the height map | the same | 262 144 | 524 288 | 1 |
| command streams (52), written by the back end | `frame_submit`, `src/vivante/backend.rs:2276-2341`; cleaned at `:2316` | 0 | 75 856 (18 964 words) | 52 kicks |
| frame arenas (uniforms, the mipmap quads) | `frame_submit`, `:2317-2322` | 0 | 17 856 | 52 kicks |
| **total** | | **524 288 (512 KiB)** | **1 142 288 (about 1 115 KiB)** | |

- No CPU mipmap filter: all 5 chains a frame go to `gc_generate_mipmap`
  (5 `mipchain` lines a frame, no refusal). The GL core does not call
  `gc_return_mipmap_chain` in a frame (0 lines).
- No depth readback, depth copy or depth seed (`ds_cache.rs`: 0 lines).
  Terrain's depth is a renderbuffer and is never sampled.
- No buffer map or `glBufferData` in a frame. The 393 216-vertex VBO is
  uploaded once, at setup.
- Board cost: about 4 to 5.5 ms a frame for the two copies. This is
  calibrated on the board's window copy (1.92 MB read and written in
  12.3 to 12.9 ms, `imx8mp-frame-floor.md`, "Step 3 result") and on the
  CPU box filter's board cost ("The mipmap path on the board": about 90 ns
  a non-cacheable load). Writing the streams is under 1 ms.

### (3) Where the CPU waits for the GPU outside the swap

| wait | call site | a frame (host trace) | board cost |
|---|---|---:|---|
| `finish()` before the tiled copy of the normal map, rendered by the kick still being recorded (`tkick` pending) | `src/vivante/tiled_copy.rs:369-371` -> `src/vivante/backend.rs:3196-3208` (submits, then waits for all 4 slots) | **1** | the CPU waits for the window clear, the height map and the normal map (about 35 ms of GPU work), then copies and records the terrain draw while the GPU idles: about 4.5 ms of idle GPU (model) |
| `frame_open` reusing a slot whose kick has not completed | `src/vivante/backend.rs:2229-2234` (`GC_FRAMES` = 4, `:77`) | **52 checks** (every kick reuses a slot) | blocks behind every long pass: about 5 to 7 blocking waits a frame (model). After each one the 1 ms poll wakes the CPU up to 1.1 ms late, and the chain's one-level kicks then run at the CPU's pace, so the GPU starves: about 7 to 10 ms of idle GPU a frame |
| `gc_return_mipmap_chain` | `src/vivante/mipmap.rs:352-365` | 0 | none |
| depth resolve or copy `finish()` | `src/gl/texture.rs:709`, `src/vivante/ds_cache.rs:457, 509` | 0 | none |

Each wait is `wait_fence` (`src/vivante/backend.rs:2414-2420`, timeout -1)
-> `z_wait` (`src/osal/zephyr_gc.c:503-519`) -> `gc_render_wait`
(`driver/src/vivante/gc_render.c:877-970`). That polls `fence_done` and
sleeps `k_sleep(K_MSEC(1))` (`:969`), 1.0 to 1.1 ms at the 10 000 Hz tick
(`imx8mp-frame-floor.md`, candidate 3).

The swap itself: on the board glmark2's window is EGL's back buffer
(`src/gl/egl.rs:1414`, the OSAL has a present), so the swap calls
`finish_previous` (`:2413`, `src/vivante/backend.rs:3220-3233`). That waits
for the previous frame's last kick, long done when terrain's frame ends. The
window copy in `board_show` (12.5 ms) then runs while the GPU draws tilt_v
(about 23.5 ms). The host harness's window has no present, so its swap is
a `finish()` instead (the second `finish` line a frame in the trace). That
is the host's own path, not the board's.

### (4) GPU work

The table "The 52 kicks". The pixel totals a frame, at full-target cover:

- passes: 2.71 M pixels shaded;
- clears: 4 x 480 000 + 4 x 65 536 = 2.18 M pixels;
- mipmap levels: 3 x about 160 000 + 2 x about 21 800 = 0.52 M pixels;
- 204 M fragment instructions;
- 22.0 M vertex instructions (the terrain mesh).

Every target is linear, so there is no early depth. Depth: the terrain
target's depth16 renderbuffer and the window's D24S8, both with plain
quads as clears.

### Bounded waits on the path

None can set a 250 ms frame. `z_wait` bounds a wait without a deadline at
2000 ms (`src/osal/zephyr_gc.c:438`, `:510-511`). A fence that does not
retire for `GC_HANG_MS_DEFAULT`, 5000 ms, ends as a hang
(`driver/src/vivante/gc_render.h:270-271`, `gc_render.c:950-952`). A submit
waits up to 1000 ms for a free event (`gc_render.c:591`,
`driver/drivers/gpu/gpu_vivante_gc_rust.c:339-365`), and with 4 stream
slots at most 4 of the 30 events are ever in flight. Any of these firing
would cost 1 to 5 s and log a line. The poll's 1 ms sleep is the only
timed step, and it is in (3).

## The GPU calibration

`imx8mp-frame-floor.md` gives 3.9 ns a pixel for 1-instruction shaders
(candidate 2). For heavier shaders the board's effect2d scenes are the
cleanest calibration. Each is one full-screen quad (480 000 pixels, 1 kick),
and its instruction counts come from the same trace:

| scene | fsi / texture loads | 1 buffer FPS | 3 buffers FPS (`tools/cts/RESULTS.md:124-125`, main `45dc44a8`) |
|---|---|---:|---:|
| effect2d 3x3 | 63 / 9 | 40 | 48 (20.4 to 20.8 ms) |
| effect2d 5x5 | 105 / 15 | 28 | 28 (34.5 to 35.7 ms) |

Both kernels have 7 instructions per texture load, so these two cannot
split ALU from texture cost.

- Read as GPU-bound with the window copy overlapped (the 3-buffer present
  runs beside the GPU), the frames are the GPU time. That gives 20.6 ms
  (3.8 ms of clear and raster, 16.8 ms of shading) and 35.1 ms: 0.56 and
  0.62 ns an instruction a pixel. The ratio 1.70 matches 105 / 63 = 1.67.
- Read with the present in series instead (12.5 ms subtracted), the two
  give 8 and 22.6 ms, and no proportional model fits them (0.72 ns from
  the difference predicts 21.8 ms for the 3x3).

So the note uses **0.6 ns an instruction, per pixel or per vertex**, which
is one instruction a clock on each of the two shader cores at 800 MHz
(`docs/notes/imx8mp-gc-bringup.md:41`). It also uses 0.8 ns as the upper
case for terrain's own mix:

- the noise is mostly `floor`, `mod`, `step` and `dot`;
- terrain.frag has `normalize`, `pow` and fog;
- a vertex may cost more than a pixel.

The vertex cost is the least known term: no board scene isolates it.

## The board timeline model

A step model of one frame:

- **CPU.** Records each kick (0.1 to 0.3 ms for a mipmap level, 0.3 to
  1 ms for a pass). For the terrain draw it first `finish()`es and makes
  the two tiled copies (2.2 to 3 ms each).
- **Stream slots.** Before each kick it waits for the slot of the kick 4
  earlier. The wait polls every 1.05 ms.
- **GPU.** Runs the kicks in order, with the estimates above plus 0.03 to
  0.08 ms a kick.
- **Swap.** Waits for the previous frame's last kick, copies the window
  (12.5 ms) and waits for the next latch (T = 17.0 ms, 1 buffer).

30 frames, the mean of frames 10 to 29:

| terrain cover x overdraw | ns an instruction | GPU ms a frame | frame ms | blocking waits a frame | 1 ms sleeps a frame |
|---|---:|---:|---:|---:|---:|
| 0.8 | 0.6 | 139 to 142 | 161 to 166 | 6 to 7 | 107 to 108 |
| 1.2 | 0.6 | 156 to 159 | 179 to 188 | 7 to 8 | 122 to 125 |
| 0.8 | 0.8 | 181 to 184 | 199 to 211 | 6 to 7 | 144 to 148 |
| 1.2 | 0.8 | 204 to 207 | 224 to 236 | 6 to 7 | 166 to 174 |

The board's 4 FPS is 200 to 250 ms (`unsigned(frames / elapsed)`). The
model's upper half reaches it, its lower half falls short by up to 40 ms.

What each serial cost is worth: from a central case (0.8 cover, 0.7 ns,
183.4 ms), one cost removed at a time:

| removed | frame ms | saved |
|---|---:|---:|
| the mid-frame `finish()` (the copies then overlap the GPU) | 178.9 | 4.5 |
| the 1 ms poll (a 20 us poll instead) | 180.7 | 2.7 |
| one kick per mipmap chain (52 -> 9 kicks) | 176.3 | 7.1 |
| 64 stream slots instead of 4 | 176.3 | 7.1 |
| the present's copy and latch | 183.8 | 0 (they overlap tilt_v) |

## The mipmap path on the board

`tools/cts/RESULTS.md:170-190`: the per-scene record at main `445c5653`
moved terrain from 1000 ms (1 FPS, `c5f10ea9`) to 250 ms (4 FPS), and
refract from 1000 to 500 ms. The one GL or GC7000 change between those two
commits is `dc033dc4`, the GL core offering the rendered-into chains to the
back end (`git merge-base --is-ancestor dc033dc4 445c5653`: yes). The
3-buffer table of 2026-10-18 (`RESULTS.md:134`, before the offer) also reads
terrain 1 FPS.

The "moved no scene" of `imx8mp-glmark2-cpu.md` ("On the board, after the GC
mipmap offer") compared `a917d123` with `445c5653`, and both already hold
the offer. So the GPU chain is taken on the board, and it took about 750 ms
off terrain's frame. That is the CPU box filter of 5 chains on
non-cacheable memory, against 40 ms on the host
(`gc-mipmap-cpu-cases.md`, "before"): about 19 times the host. The filter
reads a byte at a time, about 8.6 MB a frame, so this is about 90 ns a
non-cacheable load. That agrees with `imx8mp-glmark2-gap.md`'s 100 ns from
the window copy. This answers #1106's open question by reasoning from the
record. Page 4 of fb-counters confirms it by a count (below).

## Candidates, ranked, with the fix and the record that confirms each

Page values are what the X2 would read with the cause as described. The X2
keeps w0..w2 only, so v0..v3 of a page. Pages 0 to 3 are
`driver/samples/zephyr/glmark2/src/gm_perf_page.h` at `964f50f0`: every
value saturates at 0xffff, and a scene is 10 s
(`driver/samples/zephyr/glmark2/Kconfig`, `GLMARK2_ARGS`). Page 4 (0x1004)
is the fb-counters item's: v0 GPU mipmap chains, v1 CPU mipmap fallbacks,
v2 CPU waits on the GPU outside the swap, v3 KiB of CPU passes over the
carve-out. The values below are a frame's; scale them to the page's unit
as fb-counters defines it (for example per 100 frames).

Common to all: **page 0 v0 = 40 to 50 frames** (10 s / 200 to 250 ms);
v1 (frame us) = 0xffff (saturated).

| rank | cause | est ms a frame | fix (owner) | X2 record that confirms it |
|---|---|---:|---|---|
| 1 | **GPU shading of the 9 passes**: noise height map 33 to 44, terrain 50 to 75 (vertex 13 to 18 of it), tilt-shift 44 to 58, the rest about 10 | 140 to 205 | Largest parts first. (a) Terrain's fragment shader runs on every fragment because linear targets have no early depth (`backend.rs:1754-1783`): a tiled colour target with early depth and tile-status fast clear (`imx8mp-frame-floor.md` F4; also turns the 8 clear quads, about 9 ms, into fast clears). (b) The noise shader's 840 instructions: compare with Mesa etnaviv's count for the same text (`tools/vivante/etna_shc.c`, `glsl2vivante.py`). If ours is much longer, the online compiler's vec4 packing of `permute` and `mod` is the fix. Not done here (needs a built Mesa tree). (c) The 393 216 non-indexed vertices are glmark2's, not ours. | page 1 v2 (`gc_render_wait` sleeps per 100 frames) **10 000 to 17 000**: the CPU sleeps through 110 to 180 ms of each frame waiting on the GPU. Page 1 v1 (waits per 100 frames) 600 to 800. Page 1 v0 and v3 = 0xffff. Page 3 v1 (HI idle kcycles an interval) **16 000 to 40 000** (the GPU idle 20 to 50 ms a frame) while v0 = 0xffff. Page 0 v2 (gl) = 0xffff: the CPU waits inside GL calls. Page 0 v3 (swap) 12 500 to 30 000 us. If instead page 3 v1 reads over 80 000 (the GPU idle 100 ms or more), rank 1 is wrong and the time is on the CPU. |
| 2 | **52 kicks through a 4-slot ring with 1 ms polls**: the CPU blocks behind each long pass, wakes late, then feeds the one-level mipmap kicks slower than the GPU runs them | 7 to 10 | One kick per chain: all levels of a chain in one stream (`mipmap.rs:270-278`, `backend.rs:2222-2260`; `imx8mp-glmark2-gap.md` item 3). With it or instead, more stream slots (`GC_FRAMES`, `backend.rs:77`). | Page 1 v1 600 to 800, and page 4 v2 **6 to 8 a frame**: 5 to 7 in `frame_open`, 1 from rank 3. After the fix: kicks 52 -> 9 a frame (page 1 v4, 5 200 -> 900 per 100 frames, readable only on boards that store w3). The blocking waits stay about as many, because the CPU still waits behind the long passes; what goes is the GPU's idle time between them. So page 3 v1 falls by about 5 000 kcycles (about 7 ms) and page 0 v0 rises by 1 to 2 frames. |
| 3 | **Mid-frame `finish()` and the CPU tiled copies of the height and normal maps** (the vertex stage cannot read linear storage, `tiled_copy.rs:170-180`) | 4 to 6 | Tile the two maps on the GPU instead of the CPU: a resolve-engine (RS) copy into the tiled copy, ordered in the stream after the render that wrote the map. That removes the `finish()` and the 1.5 MB of non-cacheable CPU traffic. Or render the two maps into tiled storage directly (`gc-terrain-tiled-copy.md`, "Not done"). | Page 4 v2 includes **1 a frame** from `tiled_copy.rs:371`. Page 4 v3 about **1 536 KiB a frame**: 512 read + 1 024 written, or 1 024 KiB if the zero fill is not counted; the streams and arenas add 91 KiB if counted. After the fix: v3 about 0 to 91, v2 one less, page 0 v0 up about 1 frame. |
| 4 | **The 1 ms sleep poll** in `gc_render_wait` (`gc_render.c:969`) | 2 to 4 | Wake the wait from the interrupt (plan #328 item fence-irq). | Page 1 v2 / v1 = 15 to 25 sleeps a blocking wait. After the fix: page 1 v2 near 0 with the same v1, page 0 v0 up 0 to 1 frame. |
| 5 | **The present**: the window copy of 1.92 MB and the latch at 1 buffer | 0 on the critical path (12.5 to 29.5 ms of CPU, beside tilt_v) | None for terrain; it matters only once the GPU frame is under about 30 ms. | Page 0 v3 (swap) **12 500 to 30 000 us**. A swap of 0xffff would mean the swap waits for the GPU (not expected: `finish_previous` waits for the previous frame only). |
| 6 | **CPU mipmap fallback** | 0 (ruled out by the 1000 -> 250 ms move) | none | Page 4 v0 **5 a frame**, v1 **0**. v1 = 5 with a frame near 1000 ms would mean the board left the chains to the CPU after all. |
| 7 | **GL-call CPU time** | 2 to 4, overlapped | none now | Page 0 v2 cannot show it (the waits are inside the GL calls). Not separable on the X2 with v0..v3. |
| 8 | **A bounded wait or timeout** | 0 | none | A 1 to 5 s frame would put page 0 v0 at 10 or fewer and log a stall. Not expected. |

Order of the work by gain: rank 1 (only the early-depth or tiled-target
change and the compiler comparison move the frame by tens of ms), then
ranks 2 and 3 (about 12 to 16 ms together, both host-testable), then
rank 4 (fence-irq, already an item). Even with ranks 2 to 4 all done the
frame stays at about 160 to 220 ms. Terrain reaches the panel's rate only
when the GPU work itself is cut, by early depth, fast clears and a
shorter noise shader.

## What this does not show

- No board number. The GPU costs are a two-scene calibration and a
  geometric guess of the terrain's cover. The model's 160 to 236 ms
  brackets the board's 200 to 250 but does not prove the split.
- The vertex stage's cost per instruction and the cost of `pow`,
  `normalize` and `mod` sequences were not measured on this GPU.
- Etnaviv's instruction count for `terrain-noise.frag` was not taken. The
  Mesa 26.2.1 tree in the dependency cache could not be copied for
  `etna_shc` in this session, and patching the shared copy in place would
  have changed another item's dependency.
- The page 3 KiB values (GPU traffic) are not predicted closely here.
  About 17 MB a frame is the terrain mesh alone (393 216 x 44 bytes), so
  expect page 3 v2 of about 25 000 to 50 000 KiB and v3 of about 15 000 to
  25 000 KiB. They do not separate the candidates.
