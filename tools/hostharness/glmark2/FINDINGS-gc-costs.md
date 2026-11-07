<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain, shadow and refract: what the GC7000 makes the CPU pay

Issue #1204 (plan #328, item harness-gc-costs). The host profile
(`GM2_PROFILE=1`) gives the GL calls' CPU time and nothing else: linux.c
completes every kick at once. This record is `GM2_GCCOST=1` (README.md, "GC7000
costs"): per frame, the CPU's waits for the GPU, the bytes of cache
maintenance over the carve-out, and the kicks with their targets, counted
through the OSAL table `shim.c` puts in front of linux.c's. Counts, not times.

## Run

Library: `src/` at `009f62a5` (unchanged at main `efbb727c`: no commit
since touches `src/`, `include/` or the build), the harness of this change. glmark2
`22c527cb0556f3a1ac4445aaa52cc532760928d5`, `--size 800x600`, 11 frames,
counted frames 2 to 11 (10). GC carve-out modelled at 245642240 bytes (the
i.MX8MP glmark2 image's, less the board's window and screen); no allocation
refused, no GL error.

```sh
GM2_WRITE=0 GM2_GCCOST=1 GM2_FRAMES=11 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-gccost-lag vivante
GM2_WRITE=0 GM2_GCCOST=1 GM2_GCCOST_GPU=done GM2_FRAMES=11 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-gccost-done vivante
```

The first run's `GPU model` is `lag` (a kick completes only when the CPU waits
for it or a later one: each fence the back end checks is a wait, the most the
board pays when its GPU is the slower side, as in these three scenes). The
`done` run gave the same kicks, draws, boxes and cache bytes, to the digit,
with no wait: the lag model moves the waits only. A third run without the
switch printed no `gm2: gc-` line and wrote no `gc-cost-vivante.md`, and
`report.py`'s findings text of all three runs is the same, line for line.

## A frame

From `build/gm2-gccost-lag/gc-cost-vivante.md`, per counted frame:

| scene | waits: in the swap, elsewhere | kicks | draws in the streams | draw box Mpx | RS blits | invalidated KiB (calls) | flushed KiB (calls) | stream KiB |
|---|---|---|---|---|---|---|---|---|
| `terrain` | 1.8, 46.2 | 52.0 | 59.0 | 4.89 | 0 | 512.0 (2.0) | 604.0 (118.0) | 74.1 |
| `shadow` | 0.9, 1.8 | 3.0 | 5.0 | 5.28 | 0 | 7600.0 (1.0) | 7506.4 (11.0) | 5.7 |
| `refract` | 2.3, 9.4 | 13.0 | 14.0 | 5.44 | 0 | 7600.0 (1.0) | 7522.2 (29.0) | 17.9 |

glmark2 itself draws 8 (terrain), 3 (shadow) and 2 (refract) times a frame
(scene-end `draws=` over 11 frames); the rest of the streams' draws are the
back end's own: the GPU mipmap chain (one draw a level), the clears.
No kick uses the resolve engine (RS blits 0). Each wait is at least one 1 ms
poll on the board (`driver/src/vivante/gc_render.c`), so the waits alone are
a floor of about 48 ms (terrain), 3 ms (shadow) and 12 ms (refract) a frame,
before the GPU's time in them.

### terrain

* `glGenerateMipmap`: 48 of the 52 kicks and 40 of the 48 waits a frame. The
  caller of all 40: `wait_fence<draw_one<record_level<generate_mipmap<glGenerateMipmap`.
  Three chains from 800x600 (stride 3200) and two from 256x256 (stride
  1024) a frame, one kick a target size down to 1x1 (10 and 9 kicks a chain,
  3 x 10 + 2 x 9 = 48). `draw_one` opens a frame in the next of the
  back end's four command slots (`GC_FRAMES`), which waits for the kick last
  recorded in it: a run of kicks waits on the kick four before it, whatever
  its size. Most of these kicks are tiny: 400 of the 480 counted ones have a
  draw box under 0.1 Mpx (levels 100x75 and below). Flushes: 96 calls, 83 KiB
  a frame (their command streams and arenas).
* `glDrawArrays`: 3 kicks and 6 waits a frame (5 `draw_one<gc_draw`, 1
  `call<gc_draw`), two 256x256 targets (the noise and normal maps, 4 draws)
  and one 800x600. The CPU invalidates 512 KiB in 2 calls (two 256x256 RGBA
  maps read back, the vertex-stage tiled copies) and flushes 519 KiB.
* The swap: 1.8 waits, 0.9 kicks.

### shadow

* `glDrawArrays`: 2 kicks a frame, one into a 1600x1200 target with no colour
  surface (`stride` 0: the depth pass, 2 draws, 3.84 Mpx of boxes) and one
  800x600. 1.7 waits a frame, every one from
  `wait_fence<{closure#1}<resolve_depth<depth_for_draw<draw_arrays`: the CPU
  depth resolve waits for the depth pass. The CPU then invalidates 7600 KiB
  in one call (1600x1216 at 4 bytes: the depth surface) and flushes 7504 KiB
  (1600x1200 at 4 bytes: the texture it writes): about 15 MB of CPU passes a
  frame over memory that is Normal non-cacheable on the board.
* The swap: 0.9 waits, 0.9 kicks.

### refract

* `glGenerateMipmap`: 11 kicks and 8 waits a frame, a 1600x1200 RGBA chain
  (stride 6400) of 11 levels, a kick a level, 2 draws in the first (3.84 Mpx)
  and one in each after; caller
  `wait_fence<draw_one<record_level<generate_mipmap<glGenerateMipmap`.
* `glDrawArrays`: 1 kick and 1 wait (`draw_one<gc_draw`) a frame, and the
  same 7600 KiB invalidated in one call and 7502 KiB flushed as shadow's
  depth resolve.
* The swap: 2.3 waits, 0.9 kicks; `glFinish` 0.4 waits.

## What it points at

These are host counts; the board items measure the milliseconds.

1. terrain's and refract's GPU mipmap chains, a kick a level, each kick
   waiting for its command slot's previous one: 40 and 8 waits a frame (item
   mip-one-kick).
2. shadow's and refract's CPU depth resolve: 7.4 MiB read and 7.3 MiB
   written a frame over the carve-out, behind a wait for the depth pass
   (items resolve-wide, resolve-own-kick, depth-sample-gc).
3. terrain's 512 KiB of vertex-stage maps read back a frame (item
   tiled-copy-rs).
4. The DC range maintenance itself: 118 flush calls a frame in terrain, 29 in
   refract, every one over non-cacheable memory on the board (item
   gc-cache-skip).

## Limits

* The draw box is the viewport and scissor box at the draw (in samples), an
  upper bound of what a draw covers; the stream does not give the covered
  pixels. A target's size is the kick's largest box extent.
* The lag model is the worst case for waits: a board whose GPU finishes a kick
  before the CPU checks it pays fewer. It also changes what
  `gc_kick_pending` answers, as a busy GPU does.
* The back end's own counters (`gc_counters`, `src/vivante/backend.rs`) are
  not exported (no `#[no_mangle]`), so the waits are counted at the OSAL's
  `wait`, not cross-checked against the back end's count.
