<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP glmark2: why terrain reads 250 ms and shadow/refract 500 ms a frame on the board against 1 to 2 ms of host CPU

A desk study (2026-10-21, main `41e95169`; line numbers are that commit's).
Nothing ran on a board and no code changed. Every millisecond below is an
**estimate from the repository's own numbers**, not a board reading: the
i.MX8MP has no console and its per-scene record has no GPU or CPU split for
these three scenes (section "What the record can and cannot show").

## The facts

| scene | board, main `a917d123` (`tools/cts/RESULTS.md`, i.MX8MP row; `docs/notes/imx8mp-glmark2-cpu.md`, "On the board, after the GC mipmap offer") | what that FPS can mean | host CPU in GL calls (`tools/hostharness/glmark2/FINDINGS-gc-allscenes.md`) |
|---|---|---|---|
| terrain | 250 ms, 4 FPS | 200 to 250 ms | 0.9 ms (8 draws 0.22, 5 mipmaps 0.05) |
| shadow | 500 ms, 2 FPS | 333 to 500 ms | 2.6 ms (`glDrawArrays` 2.35, 3 calls) |
| refract | 500 ms, 2 FPS | 333 to 500 ms | 2.5 ms (`glDrawArrays` 2.2, 2 calls) |

"ms a frame" there is 1000 / the integer FPS, so the true figure is up to one
FPS step *faster* (`unsigned(frames / elapsed)`, `imx8mp-frame-floor.md`
"What is known"). The GC mipmap offer moved no scene (sum of all 33 scenes
1932.65 -> 1934.56 ms).

## The short answer

The gap is not 4 to 10 times; it is about 100 to 250 times for the one loop
that matters, and that is what `imx8mp-glmark2-cpu.md` assumed away.

1. **The "4 times the host" factor does not hold for memory-bound loops.**
   The carve-out is mapped Normal non-cacheable (`driver/src/vivante/gc_render.c:68-92`,
   `K_MEM_ARM_NORMAL_NC`). An A53 load from it costs one memory round trip
   whatever its width. The board has one calibration: the 1.92 MB window copy
   in `board_show` takes 12.3 to 12.9 ms (`imx8mp-frame-floor.md`, "Step 3
   result": frame minus GPU busy is 12.9 ms for `clear` and 12.3 ms for
   `build`), where the host does the same copy in about 0.1 to 0.2 ms: 60 to
   100 times. With 16-byte loads that is about 120 000 loads in about 12 ms,
   roughly 100 ns a load.
2. **shadow and refract each resolve a 1600 x 1200 depth texture on the CPU
   every frame**: 1.92 M texels, 7.68 MB read from non-cacheable memory and
   7.68 MB written (FINDINGS-gc-allscenes.md, "Where shadow's and refract's
   draw time goes"). At 100 ns per 16-byte load that is 50 ms; at one load per
   texel it is about 190 ms. That is the largest single cost on the two
   scenes and it is invisible on the host (0.7 to 2.1 ms, cached).
3. **terrain has no such loop.** Its board time is GPU pixel work plus the CPU
   waiting on the GPU in 1 ms sleeps, about 45 kicks a frame since the GPU
   mipmap offer, through a ring of 4 stream slots.
4. **Even these estimates reach only 250 to 300 ms for shadow and refract**
   against 333 to 500. The remainder is not explained by anything this
   repository holds; it is the board measurement's job.

## Ranked explanations

Estimates are per frame, board, from repository facts. "Basis" says which.
None is measured on a board.

### terrain (200 to 250 ms)

| rank | hypothesis | estimate | basis, with code | verdict |
|---|---|---|---|---|
| 1 | **GPU-bound: pixels x passes** at 800 x 600, linear targets, no early depth | 50 to 120 ms | Passes a frame: height map and normal map (256 x 256), terrain target 800 x 600, bloom_h and bloom_v 256 x 256, tilt_h 800 x 600, the window; about 2 to 3 M shaded pixels, plus the 43 mip levels. Every covered pixel is shaded before the depth test (`imx8mp-frame-floor.md` candidate 2: linear colour target, `src/vivante/backend.rs:1584-1590` in that note's tree). Measured on this GPU: 3.9 ns a pixel with 1-instruction shaders, about 22 ns a pixel for gc_slint's real shaders (47.4 ms for about 2.15 M pixels, `tools/cts/RESULTS.md:15802-15816`). Terrain's shaders (bump-mapped lighting, 9-tap blur, tilt-shift) are heavier: 20 to 40 ns a pixel assumed. | likely the biggest term; unmeasured |
| 2 | **The CPU waits for the GPU once per mipmap kick, in 1 ms sleeps** | 25 to 55 ms | `record_level` draws every level into its own target, so every level above 0 is a kick (`src/vivante/mipmap.rs:120-215`, pinned by `tests/gc_mipmap_gpu.rs`: "one kick per level"): terrain 3 x 9 + 2 x 8 = 43 kicks a frame plus about 8 of its own. The stream ring has `GC_FRAMES = 4` slots (`src/vivante/backend.rs:74`); `frame_open` waits for the slot's fence before reusing it (`:1979-1989`), the wait is `wait_fence` -> `z_wait` -> `gc_render_wait`, which polls and does `k_sleep(K_MSEC(1))` (`driver/src/vivante/gc_render.c:968-969`; 1.0 to 1.1 ms with the 10 000 Hz tick, `imx8mp-frame-floor.md` candidate 3). A level's GPU time is under 1 ms, so a wait that is not done at its first poll costs a whole millisecond: 25 to 50 waits. `gc_generate_mipmap` also ends with `gc_flush()` per chain (`mipmap.rs:274`). The host has neither the ring wait nor the sleep. | real if the GPU is the slower side; sets a floor of about 1 ms a kick |
| 3 | **Cache maintenance and non-cacheable CPU passes** | 15 to 30 ms | The tiled copies of the 256 x 256 height and normal maps are rebuilt every frame and are legitimate (`docs/notes/gc-terrain-tiled-copy.md`): 2 x 256 KB read and written at the window copy's rate (about 150 MB/s) is 3 to 4 ms, plus the zero fill and `cache_invd` / `cache_flush` of each (`refresh`); `frame_submit` cleans stream and arena (`backend.rs:2060-2066`); the window copy in `board_show` is 12.5 ms (measured, any scene). | smaller than 1 and 2; the window copy is certain |
| 4 | **The CPU mipmap fallback is still taken on the board** | 0 if the GPU path runs; about 120 ms if it did | The host proves the offer is reached (28 of 28 calls, `gc-mipmap-cpu-cases.md`, `gc_backend_mip_chains` counted no refusal), but the board image's page record carries no `gc_backend_mip_chains` and no refusal counter (`imx8mp-glmark2-cpu.md`, "On the board", last paragraph). The CPU box filter on the board: terrain's three 800 x 600 chains read and write about 2.6 MB each from non-cacheable memory (17 ms each at the copy's rate) plus the arithmetic. Pointer against it taking the CPU path: the offer would then change nothing only if the GPU path costs the same as the CPU path (rank 2 plus rank 1's extra levels), which is possible but is the coincidence the record cannot rule out. | low probability, high impact: the swing factor, settled by one counter |
| 5 | **Integer-FPS quantisation** | at most x1.25 (terrain) | 4 FPS is 200 to 250 ms, so 250 ms is the top of its bin. | explains a quarter at most |

The sum of ranks 1 to 3 is 90 to 205 ms against 200 to 250: the repository's
numbers just reach the observation at their upper ends.

### shadow and refract (333 to 500 ms)

| rank | hypothesis | shadow | refract | basis, with code | verdict |
|---|---|---|---|---|---|
| 1 | **The CPU depth resolve reads non-cacheable memory** | 60 to 200 ms | same | `settle` -> `resolve_depth` (`src/gl/texture.rs:695-732`) -> `gc_depth_resolve` (`src/vivante/backend.rs:2719`) -> `ds_cache::resolve_depth` (`src/vivante/ds_cache.rs:477-531`): `cache_invd` of the whole 7.8 MB surface (`:513`), then one `unorm24_depth` per texel (`:526`, `src/ds_retention.rs:135`: a `u32` load, a shift, an f64 divide, an `f32` store), then `osal::cache_flush` of the 7.68 MB result (`texture.rs:724`). Read side 1.92 M words; at 16-byte loads (if LLVM vectorises the loop) 120 k loads x 100 ns = 12 ms for the loads plus the f64 divides (about 15 ns each on an A53, 29 ms) and the NC writes; at one 4-byte load per texel 190 ms. This is where "4 times the host" breaks: the host does the whole loop in 0.7 to 2.1 ms because the surface is in cache. `docs/notes/gc-depth-resolve.md` made the loop 2x cheaper on the host, which cannot show here. | the largest term; the width of its range is the main uncertainty |
| 2 | **GPU-bound: depth pass at twice the canvas** | 20 to 50 ms | 40 to 90 ms | Both scenes render depth into a 1600 x 1200 target (FINDINGS-gc-allscenes.md; `width_ = canvas_.width() * 2`): the clear is a full-screen quad of 1.92 M pixels (`backend.rs` `emit_clear`, no tile-status fast clear), at 3.9 ns a pixel 7.5 ms, plus the model, then the 800 x 600 pass sampling the result. Refract adds a 1600 x 1200 colour target and its 11-level GPU chain (10 kicks), and a refraction pass. | second term |
| 3 | **CPU and GPU in series, not overlapped** | included in 1 and 2 | included | `resolve_depth` does `flush()` then `finish()` (`texture.rs:706`) and `gc_finish` waits for all 4 slots (`backend.rs:2905-2920`): the CPU idles while the depth pass runs, then the GPU idles while the CPU resolves, then the main pass runs. The frame is the sum, not the maximum (`imx8mp-glmark2-cpu.md` item 2 names the wait). Waits poll at 1 ms steps (`gc_render.c:968`). | removes the overlap that rank 2's time could otherwise hide behind; 1 to 3 ms of polling |
| 4 | **Cache maintenance on non-cacheable memory** | 5 to 25 ms | same | Two range operations of 7.7 MB each per frame (`ds_cache.rs:513`, `texture.rs:724`), about 245 000 64-byte lines by virtual address; A53 `dc ivac` / `dc civac` of a line not in the cache is tens of ns. On a non-cacheable mapping both are pure cost: there is nothing to invalidate or clean (`gc_render.c:68-79` says so for the alias problem). | small, easy to remove |
| 5 | **Present copy and one-off costs** | 12.5 ms + 5 ms | 12.5 + 10 ms | The window copy (measured, above). First-frame costs inside the timed 20 to 40 frames: refract's NPOT chain growth took 25 ms on the host (`gc-mipmap-cpu-cases.md`, last section) and is a non-cacheable copy of 7.7 MB on the board; at 2 FPS a one-second one-off is 25 to 50 ms a frame. | small |
| 6 | **CPU mipmap fallback still taken** (refract only: 1 chain, 1600 x 1200) | 0 | 0, or about 300 ms if taken | The box filter reads 7.7 MB and writes 2.6 MB non-cacheable, 67 ms at the copy's rate, plus about 45 to 52 ms of arithmetic on the host scaled for an A53. Refract's board time before the offer is the same 500 ms, so a fallback of that size would have to be replaced by an equal cost (10 kicks, rank 2). Settled by the same counter as terrain's rank 4. | low probability, high impact |
| 7 | **Integer-FPS quantisation** | x1.5 at most | x1.5 | 2 FPS is 333 to 500 ms. | the true figure may be a third lower than 500 |

Sum of ranks 1 to 5: shadow about 100 to 290 ms, refract about 130 to 330 ms,
against 333 to 500. The upper ends touch the lower bound of the observation;
the lower ends are far from it. The most probable reading is the upper end of
rank 1 (one load per texel at 100 to 150 ns) plus rank 2, with quantisation
putting the real figure in the lower half of the bin. That is a reading, not
a result, and the board record below decides it.

### What is ruled out

- **Frame floor (VSYNC).** It rounds a frame up to a multiple of 17.0 ms; at
  200 to 500 ms it is a 3 to 8 % effect (`imx8mp-frame-floor.md`).
- **The 1 ms poll as the main cost.** It matters only per wait (rank 2 of
  terrain) and sets a floor; shadow has a handful of waits a frame.
- **A bad host figure.** The host numbers are right for what they measure
  (cached CPU time); they are not a model of the board.

## What the record can and cannot show

The i.MX8MP's per-scene record is the GM F page (fps per scene, integer) and
the PERF pages (`imx8mp-frame-floor.md`, "The frame-breakdown record"; code
`driver/samples/zephyr/glmark2/src/main.cpp`, `gm_perf_page.h`). **The PERF
pages saturate every value at 0xffff** (`gm_perf_page.h:14-15, 73`). Times are
microseconds, so 65.5 ms is the ceiling. At 250 to 500 ms a frame these
saturate and read `ffff`:

- page 0 v1 frame, v2 gl, v3 swap, v4 show (a show is about 0 here), v5
  frame to frame;
- page 1 v0 wait us, v3 GPU busy us;
- page 3 v0 and v1 (kcycles: 800 MHz x 0.25 s = 200 000 kcycles per
  interval).

What survives (counts, per-100 values, KiB):

| value | what it gives for these scenes |
|---|---|
| page 0 v0, frames | ms a frame = 10 000 / frames (a scene is 10 s, `driver/samples/zephyr/glmark2/Kconfig:23`; 20 to 40 frames, so 2.5 to 5 % resolution, better than integer FPS) |
| page 1 v2, `gc_render_wait` sleeps per 100 frames | CPU asleep waiting for the GPU: ms a frame = v2 x 1.05 / 100 (each sleep is 1.0 to 1.1 ms) |
| page 1 v1, waits per 100 frames; v4, kicks per 100 frames | **which mipmap path ran**: GPU chains give about 5 000 kicks per 100 frames for terrain (43 mipmap + about 8 own), about 1 300 for refract (10 + about 3), a few hundred for shadow; the CPU fallback gives about 800 for terrain, about 300 for refract. Derived from the host's call counts, not measured. This is the missing `gc_backend_mip_chains`. |
| page 2 v2, intervals at 2.5 T or more | confirms every frame is long (v2 = frames - 1) |
| page 3 v2 and v3, KiB read and written per interval | the GPU's memory traffic per frame (a lower bound on its time at the DDR rate); it does not include the CPU's non-cacheable accesses |

So with the pages as built, the final session can say: the frame in ms
(page 0 v0), how much of it the CPU spent asleep in GPU waits (page 1 v2), and
which mipmap path ran (page 1 v4). It cannot yet say how much of the rest is
GPU work and how much is CPU work on non-cacheable memory, which is exactly
the split asked for: **GPU busy us (page 1 v3) and the HI profile (page 3 v0,
v1) saturate.** `tools/cts/glmark2-ms.py` reads a console log
(`FPS: n FrameTime: ms`), which the i.MX8MP does not have: its ms a frame is
1000 / the F-page FPS (`tools/board.sh imx8mp gm IMG OUT fps`, then
`glmark2-pages.txt`) or 10 000 / the PERF frames above. The tool is for the
Pi 4 and AM62P sessions.

### The record the final i.MX8MP session should take

Existing tools only, one scene per image, the glmark2 sample's copy path (no
view) and the default one screen buffer, as in "Step 3" of
`imx8mp-frame-floor.md` (its recipe, `export $(tools/cts/deps-env.sh imx8mp)`,
`west build ... '-DCONFIG_GLMARK2_ARGS="-b terrain"'`, then four boots):

```sh
for scene in terrain shadow refract; do          # one image per scene
  for p in 0 1 2 3; do
    SEL=$((0x1000 | p)) BOARD_HOST=<from ask_farm> BOARD_BOOT=uboot-mmc \
      tools/board.sh imx8mp run "$IMG/gm-$scene/zephyr/zephyr.bin" "$REC" "$scene-p$p"
  done
done
yakogl-tools record <w0> <w1> <w2> <w3>          # each snvs.txt: "experiment 0 page p: v0 .. v5"
```

12 boots, about 100 s each. Plus the F pages for the same main, so the
integer FPS comes from the same image (`tools/board.sh imx8mp gm IMG OUT fps`,
`GM_ROWS` to cut it to scenes 22 to 24). From the surviving values, per scene:

1. ms a frame = 10 000 / page 0 v0.
2. asleep = page 1 v2 x 1.05 / 100; awake = frame - asleep. A frame that is
   mostly asleep is GPU-bound (or waiting on 1 ms polls); a frame that is
   mostly awake is CPU work (the resolve, the copies).
3. page 1 v4 against the table above: which mipmap path ran, and how many
   kicks a frame cost.
4. page 3 v2 + v3: GPU traffic per frame.

To separate the **GPU busy time from the CPU's non-cacheable work**, the
pages must stop saturating (fix item 1 below). Until then a second, existing
way to split them is the canvas: run each scene a second time with
`-s 400x300` in `CONFIG_GLMARK2_ARGS`. GPU pixel work falls to a quarter, the
mipmap kick count stays the same (levels, not pixels, set it) and the
resolve falls to a quarter too (it follows the canvas), so this separates
the per-kick cost (rank 2 of terrain) from the per-pixel cost (rank 1), not
the resolve from the GPU. It is a fallback, not the record.

## Ranked host-first fix items

Host first: a minimal host reproduction, then a before and after with
`tools/hostharness/glmark2` and `tools/hostharness/glmark2/compare.py`. Costs
that exist only on non-cacheable memory cannot be timed on the host; for
those the host proves the output is unchanged and counts accesses, and the
board session measures.

1. **The PERF pages stop saturating** (makes the next board session able to
   tell CPU from GPU; no effect on a frame). `driver/samples/zephyr/glmark2/src/gm_perf_page.h`
   (`sat16`, page 0, 1, 3 units: ms when a mean exceeds 65 535 us, or a
   16-us unit), `main.cpp` (the writer), `host/perf_page_test.c` and
   `host/run.sh` (the host packing and decode check),
   `driver/rust/src/board/record.rs:1190` (the decoder's unit). Add the
   mipmap counters (`gc_backend_mip_chains`, refusals) to a page, so
   rank 4 and rank 6 need no inference from kick counts. Host: the packing
   and decode check. Do first: it is cheap and every other item is judged by
   it.
2. **A depth resolve that does not read non-cacheable memory a word at a
   time** (shadow and refract, ranks 1 and 4). `src/vivante/ds_cache.rs`
   `resolve_depth` (`:477-531`) and `src/gl/texture.rs:695-732`: copy each row
   of the surface into a cached row buffer with one wide `memcpy` (or 64-byte
   loads), convert from the buffer, write the destination row from a buffer
   with one wide store; drop the whole-surface `cache_invd` (`:513`) and the
   whole-result `cache_flush` (`texture.rs:724`) when the mapping is
   non-cacheable. Host proof: `tests/gc_depth_resolve.rs` (bit-exact against
   the f64 formula; extend it with the row-buffer path) and an access-width
   count in a stand-in of the OSAL (loads of 16 bytes or more, not one per
   texel). Board: measures the gain; expected 100 to 150 ms a frame on each
   of shadow and refract if rank 1 is as estimated. Complements
   `gc-unorm24-fix` (the arithmetic, in review) and does not depend on it.
3. **The mipmap chain in one kick** (terrain rank 2, refract rank 6's
   replacement cost). `src/vivante/mipmap.rs` `record_level` / `gc_generate_mipmap`
   (`:120-290`), `src/vivante/backend.rs` `draw_one` / `frame_open`
   (`:1979`): draw all levels of a chain into one stream (a target change is
   a state group between draws, with the colour cache flush between levels),
   so a chain is one kick, not 9 or 10, and raise or remove the ring wait.
   Host: the existing stand-in OSAL that executes each stream
   (`tests/gc_mipmap_gpu.rs`); the pinned "one kick per level" becomes "one
   kick per chain", with the levels equal within 1 LSB; kicks a frame in the
   harness (`GM2_PERF=1`) go from about 50 to about 12 for terrain.
4. **A finer fence poll** (every wait in ranks 2 and 3). `driver/src/vivante/gc_render.c:968-969`:
   sleep 100 to 200 us instead of `K_MSEC(1)`, as the AM62P did
   (`docs/notes/pvr-fence-wait.md`, 303 -> 316 with a 20 us poll). Host: the
   Zephyr host model has no GPU; check the wait loop with a mock fence in the
   existing driver tests; board: page 1 v2 and the frame.
5. **A resolve that waits for its own kick only** (shadow, refract rank 3).
   `src/gl/texture.rs:706` calls `flush()` and `finish()`; `finish` waits for
   all four slots (`backend.rs:2905`). Wait for the fence of the kick that
   rendered the depth (`framebuffer::texture_target_use`), through
   `wait_fence`, and leave the later kicks running. Host: `tests/gl_*` over
   the stand-in with a counting `wait`. Small alone; it matters once item 2
   makes the CPU work short.
6. **No resolve at all: the texture unit samples the surface**
   (`docs/notes/gc-depth-sample.md`, route 1: an ARGB8888 view, the depth
   rebuilt in the shader). Removes ranks 1, 3 and 4 of shadow and refract and
   the 7.68 MB of reads. It needs the one board experiment of that note
   first; do not write code before it.
7. **Terrain's and refract's GPU time** (rank 1 of each): a tiled colour
   target with tile status, fast clear and early depth (`imx8mp-frame-floor.md`
   F4, `src/vivante/**`). The largest item and not host-first: the host has no
   GPU time. Only after the record in this note says GPU busy time is the
   larger part.

Order by expected gain a frame: item 2 (shadow and refract, up to about
150 ms each if rank 1 is right), item 3 (terrain and refract, 20 to 40 ms),
item 7 (terrain, refract), items 4 and 5 (a few ms each). Item 1 first in
time, for the measurement.

## What this study does not show

- No number here is a board reading. The 100 ns a non-cacheable load is the
  window copy's calibration (12.5 ms for about 120 000 16-byte loads), not a
  measurement of the resolve loop; whether the loop's loads are 4 or 16
  bytes wide is not known (not read from the compiled object).
- Terrain's GPU time is an estimate from 22 ns a pixel (gc_slint) and a guess
  of its shaders' weight; the Mesa etnaviv numbers were not read.
- Whether the board image reaches the GPU mipmap path is not shown by any
  record to date; the kick count of page 1 v4 settles it.
- The glmark2 scene structure (passes, canvas multiples) is taken from the
  repository's notes (`FINDINGS-gc-allscenes.md`, `gc-terrain-tiled-copy.md`,
  `gc-mipmap-cpu-cases.md`), not from the glmark2 source.
