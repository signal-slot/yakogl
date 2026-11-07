<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: host counts of glmark2 terrain, shadow and refract, before the fixes

Plan #328, item host-cost-baseline (issues #1204, #1106). Counts only, from
the harness's GC cost mode (`tools/hostharness/glmark2/README.md`, "GC7000
costs"; `FINDINGS-gc-costs.md`): no times, no FPS, no board. No code
changed. Each fix of the plan cites its change against the **baseline**
columns here and fills the **after** column; the X2 numbers
(x2-gm-after) are set beside these.

## Run

- Commit: origin/main `f862e8e7` (2026-11-07). `src/` is the same as at
  `009f62a5`, the commit `FINDINGS-gc-costs.md` was taken at: both runs give
  the same counts as that record, to the digit.
- glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5` (the deps cache's
  `glmark2-src`), `--size 800x600`, 11 frames, counted frames 2 to 11 (10),
  GC carve-out modelled at 245642240 bytes, no GL error, no allocation
  refused.
- Commands (from the repository root; `GLMARK2_SRC` is the glmark2 checkout):

```sh
GM2_WRITE=0 GM2_GCCOST=1 GM2_FRAMES=11 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gccost-lag vivante
GM2_GCCOST_GPU=done GM2_WRITE=0 GM2_GCCOST=1 GM2_FRAMES=11 GM2_ENTRIES='^(terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gccost-done vivante
```

  The first run is the default lag model (`GM2_GCCOST_GPU=lag`: a kick
  completes only when the CPU waits for it or a later one, so every fence
  the back end checks while a kick is pending is a wait: the most the board
  pays). The second keeps linux.c's answer, every kick done at once. Rows
  below are from `<out>/gc-cost-vivante.md`.
- **`done` run:** every wait count is 0.0 (swap and elsewhere) for all three
  scenes. Kicks, draws, draw boxes, RS blits, cache bytes and stream KiB are
  identical to the lag run. So the `done` run adds no column of its own: it
  confirms that the waits below are the lag model's, and that the other
  counts do not depend on the model. After a fix, the **waits** are read
  from the lag run; everything else from either.

## Baseline

A frame is a counted frame (mean of 10). "Waits other" are the CPU waits
outside the swap. Cache bytes are the DC range work through the OSAL
(`cache_invd`, `cache_flush`), the stand-in for CPU passes over the
non-cacheable carve-out; it is not the CPU's loads and stores (a pass that
reads N bytes is one `cache_invd` of N; the tiled copy's zero fill is not
counted).

| | terrain | shadow | refract |
|---|---:|---:|---:|
| waits in the swap (lag) | 1.8 | 0.9 | 2.3 |
| **waits other (lag)** | **46.2** | **1.8** | **9.4** |
| waits, all (done) | 0.0 | 0.0 | 0.0 |
| **kicks** | **52.0** | **3.0** | **13.0** |
| draws in the streams | 59.0 | 5.0 | 14.0 |
| draw box Mpx | 4.89 | 5.28 | 5.44 |
| RS blits (Mpx) | 0.0 (0.00) | 0.0 (0.00) | 0.0 (0.00) |
| **invalidated KiB (calls)** | **512.0 (2.0)** | **7600.0 (1.0)** | **7600.0 (1.0)** |
| **flushed KiB (calls)** | **604.0 (118.0)** | **7506.4 (11.0)** | **7522.2 (29.0)** |
| foreign KiB | 0.0 | 0.0 | 0.0 |
| stream KiB | 74.1 | 5.7 | 17.9 |

### Waits other, by call site (lag, a frame)

| call site | terrain | shadow | refract |
|---|---:|---:|---:|
| `frame_open` in `record_level` (`glGenerateMipmap`'s one-level kicks) | 40.0 | | 8.0 |
| `frame_open` in `gc_draw` (`glDrawArrays`, a target change) | 5.0 | | 1.0 |
| `call<gc_draw` (`glDrawArrays`: the tiled copy's `finish()`) | 1.0 | | |
| `resolve_depth` (the depth resolve's `finish()`) | | 1.7 | |
| the swap (`eglSwapBuffers`) | 1.8 | 0.9 | 2.3 |
| `glFinish` | 0.2 | 0.1 | 0.4 |

(The swap and `glFinish` rows are the harness's own path: the host window has
no present, so its swap is a `finish()`; the board's is `finish_previous`.
They are not a target of any fix below.)

### Kicks, by the entry point that submitted them

| entry point | terrain | shadow | refract |
|---|---:|---:|---:|
| `glGenerateMipmap` | 48.0 | | 11.0 |
| `glDrawArrays` | 3.0 | 2.0 | 1.0 |
| `eglSwapBuffers` | 0.9 | 0.9 | 0.9 |
| `glFinish` | 0.1 | 0.1 | 0.1 |

A target change submits the open kick, so `glGenerateMipmap`'s count is the
chain levels plus the pass the chain follows: terrain 43 levels + 5 passes
(three chains from 800x600, two from 256x256: 10 and 9 levels), refract 10
levels + K2 (the 1600x1200 pass). Passes by size, from the report: terrain
draw boxes 800x600 stride 3200, 400x300 and below; 256x256 stride 1024;
shadow's depth pass is the 1600x1200 target with `stride 0` (no colour);
refract's K2 is 1600x1200 stride 6400.

### Pass sizes (draw box Mpx a frame, by target)

| scene | target (stride): Mpx |
|---|---|
| terrain | 800x600: 2.400 + 0.864 + 0.480 + 0.096 = 3.84 (chain starts and the window); 400x300: 0.360; 256x256: 0.262 (`glDrawArrays`) + 0.262 (chain); 200x150: 0.090; 128x128: 0.033; 100x75 and below: 0.040 together. Total 4.89 |
| shadow | 1600x1200 stride 0 (depth pass): 3.840; 800x600: 0.864 + 0.480 + 0.096 = 1.44. Total 5.28 |
| refract | 1600x1200 stride 6400 (K2): 3.840; 800x600 level 1 (stride 6400): 0.480; window 800x600: 0.480 + 0.432 + 0.048; 400x300 and below: 0.120 + 0.030 + 0.009. Total 5.44 |

The box is the viewport and scissor at the draw: an upper bound of the
pixels, not the pixels (README, "GC7000 costs").

## Where a count does not match the earlier notes

Both values given. The notes are `gc-slow-terrain.md` (T),
`gc-slow-shadow-refract.md` (SR) and `gc-slow-stream-diff.md` (SD).

- **Terrain waits.** T predicts, for the board, 52 `frame_open` checks and
  about 5 to 7 blocking waits plus 1 `finish()` (model, GPU timing). The
  harness's lag model counts a wait for every check of a pending kick: **46.2
  other** (40 + 5 + 1 + 0.2). Not a contradiction, a different quantity: the
  lag count is the ceiling; the board's blocking waits are 5 to 7. After a
  fix compare the lag count to the lag count, not to T's 5 to 7.
- **Terrain `finish()` before the tiled copy.** T: 1 a frame. Harness: 1.0
  (`call<gc_draw`). Matches.
- **Terrain CPU bytes.** T: 512 KiB read and about 1 115 KiB written (the
  copies' zero fill 512 KiB + copy 512 KiB + streams and arenas 91 KiB).
  Harness: 512.0 KiB invalidated (matches the read) and **604.0** KiB
  flushed: the two copies' 512 KiB + streams 74.1 + arenas about 17.4; the
  zero fill is a store, not a flush, so T's 1 115 KiB and the harness's 604
  count different things. T's page-4 v3 of about 1 536 KiB (read +
  write incl. fill) is 512 + 1 024; the harness equivalent is 512 + 604.
- **Terrain streams.** T: 18 964 words = 75 856 B; harness 74.1 KiB =
  75 878 B. Matches. T also lists 17 856 B of arenas, which `stream KiB` does
  not hold.
- **Terrain draws.** T's table: 8 clear quads + 43 mipmap levels + 8 glmark2
  draws = 59 in the streams; T's "Draw steps" line says 60 draws a frame.
  Harness: 59.0. The 60 is a count of draw steps, not stream draws (the
  difference is not traced here). SD gives 60 for ours too; Mesa's is 51.
- **Terrain draw box.** T: 2.71 M shaded + 2.18 M clears (+ mipmap levels
  0.52 M, listed on their own). Harness 4.89 M = 2.71 + 2.18: the mipmap
  levels are inside the 2.71, not on top of it.
- **Shadow waits.** SR: 1 a frame outside the swap (`texture.rs:709`
  `finish`, waits for every slot). Harness: **1.7** from the resolve's
  `finish()`, no other. The harness waits per pending slot (K1 and K2), so a
  `finish` that finds two pending kicks counts two. This is the likely cause,
  not traced further.
- **Refract waits.** SR: 1 (resolve `finish`) + up to 7 (`frame_open`, K6 to
  K12). Harness: **9.4 other**: 8.0 in `record_level` (the chain's
  `frame_open`), 1.0 in `draw_one<gc_draw` and **no** wait with
  `resolve_depth` as the caller. The chain's 8 are more than K6 to K12 (7)
  and the resolve's `finish()` finds nothing pending after them.- **Kicks.** Shadow 3.0 (SR 3), refract 13.0 (SR 13), terrain 52.0 (T 52).
  Match. The swap's 0.9 and `glFinish`'s 0.1 kick of K1 are the harness's
  attribution (the canvas clear is submitted by whoever next changes the
  target or at a finish).
- **Shadow and refract cache bytes.** SR: reads 7 782 400 B invalidated (7 600
  KiB) and 7 680 000 B flushed (7 500 KiB). Harness: 7 600.0 invalidated and
  7 506.4 / 7 522.2 flushed (7 500 + the streams and arenas: 6.4 / 22.2 KiB).
  Matches. SR's page-4 v3 of 15 000 KiB is 7 500 + 7 500; the harness's is
  7 600 + 7 506 (shadow).
- **Shadow stream words.** SR/SD: 1 454 words = 5.7 KiB; harness 5.7. Refract
  4 592 words = 17.9 KiB. Match.
- **Shadow draw box.** Harness 5.28 Mpx; SR's GPU estimate counts only the
  clears' 2.40 Mpx exactly and the models' coverage as unknown. The box
  holds 1.92 Mpx for the horse's depth draw too (an upper bound, not the
  pixels), so 5.28 = 2.40 clears + 1.92 horse box + 0.96 ground and lit
  horse boxes.

## What each fix should move

Predicted from the three notes and from the code at the baseline commit.
Rows not listed for a fix should not move; if one does, say so in the fix's
commit. "Lag" means the `GM2_GCCOST_GPU=lag` run. Bounds, not point
predictions, where the notes give none.

| fix (item) | scene | row | baseline | expected | after | basis |
|---|---|---|---:|---:|---:|---|
| tiled-copy-rs | terrain | invalidated KiB (calls) | 512.0 (2.0) | 0 (0) | | T rank 3: the CPU no longer reads the maps; the RS tiles them in the stream |
| | terrain | flushed KiB | 604.0 | about 92 (cmd and arena only) | | the 2 x 256 KiB copy flushes go (604 - 512) |
| | terrain | waits other, `call<gc_draw` | 1.0 | 0 | | the `finish()` before the copy goes |
| | terrain | waits other, total | 46.2 | 45.2 | | one less |
| | terrain | RS blits | 0 | 2 (0.13 Mpx) | | one RS copy per map; if two or four RS ops per map are used, say so |
| | terrain | kicks, draws | 52, 59 | 52, 59 | | unchanged; stream KiB up by the RS blocks (under 1 KiB) |
| mip-one-kick | terrain | kicks | 52.0 | about 9 | | SD finding 2 (a): 52 to 9 |
| | terrain | waits other | 46.2 | at most about 9, plus 1 | | one `frame_open` check a kick at most; `record_level`'s 40.0 goes to 0 to a few |
| | terrain | draws | 59.0 | 59.0 | | levels are still draws; stream KiB falls (the per-kick prologue and state, 328 words a level kick) |
| | terrain | flushed calls | 118.0 | about 30 | | `frame_submit` flushes 2 ranges a kick: 96 calls of `glGenerateMipmap` go to about 2 a chain |
| | refract | kicks | 13.0 | about 3 | | SD: 13 to 3 |
| | refract | waits other | 9.4 | 1 to 2 | | `record_level`'s 8.0 goes to 0; the K13 `frame_open` (1.0) stays |
| | shadow | all | | no change | | no mipmap |
| resolve-own-kick | shadow | waits other | 1.8 (1.7 resolve) | 1.0 or less from `resolve_depth` | | the resolve waits for K2 only, not K1 |
| | refract | waits other | 9.4 | no change from the resolve (0 now); at most -1 | | the resolve's `finish()` is not a counted caller in refract now |
| | all | bytes, kicks, draws | | no change | | |
| resolve-wide | all | any harness row | | no change | | wider loads, the same bytes and calls: the harness counts neither CPU loads nor stores. Proof is `tests/gc_depth_resolve_loads.rs`, then the board (page 0 v0) |
| gc-cache-skip | all | invalidated / flushed KiB (calls) | terrain 512 + 604 (120), shadow 7600 + 7506 (12), refract 7600 + 7522 (30) | 0 (0) for ranges inside a GPU allocation | | **if the skip is in the Rust back end** (the `osal::cache_*` calls). If it is in `z_cache_*` / `gc_render_cache_*` (SR cause 2 names these), the host harness sits in front of the OSAL table and sees no change: the count stays and the fix's test is a stand-in OSAL |
| rs-clear-wire | terrain | draws (RS blits, Mpx) | 59.0 (0, 0.00) | 51 (8 to 16, 2.18 per colour or depth plane) | | SD: 8 clear quads go; Mesa has 51 draws. 4 clears at 800x600 and 4 at 256x256, all full, unscissored and with sizes multiple of 16 x 4 |
| | terrain | draw box Mpx | 4.89 | 2.71 | | the clear quads' 2.18 Mpx are no longer draw boxes |
| | shadow | draws (RS blits) | 5.0 (0) | 3 (3) | | canvas colour + depth fills, depth-only fill in K2: 0.96 + 1.92 Mpx |
| | shadow | draw box Mpx | 5.28 | 2.88 | | minus the 2.40 of the two clears |
| | refract | draws (RS blits) | 14.0 (0) | 12 (4) | | canvas colour + depth, K2 colour + depth |
| | refract | draw box Mpx | 5.44 | 3.04 | | minus 2.40 |
| | all | kicks, waits | | no change | | SD finding 1: GPU time only |
| gc-frames-ring | terrain | waits other | 46.2 | 5.2 or less (the tiled copy's 1.0, the swap's and `glFinish`'s excluded) | | with at least as many slots as a frame's kicks (52) a slot reused is the previous frame's, finished at the swap, so `frame_open` finds nothing pending: 40 + 5 go. With a smaller ring (say 16) the count falls by less; state N |
| | refract | waits other | 9.4 | about 0 from `frame_open` | | 13 slots suffice |
| | shadow | waits other | 1.8 | no change | | no `frame_open` wait now |
| | all | kicks, bytes | | no change | | only slots (the stream memory grows) |
| depth-sample-gc | shadow | invalidated / flushed KiB | 7600.0 / 7506.4 | 0 / about 6 | | no CPU resolve: only the streams are flushed |
| | refract | invalidated / flushed KiB | 7600.0 / 7522.2 | 0 / about 22 | | same |
| | shadow | waits other | 1.8 | 0.1 or less | | the resolve's `finish()` goes; `glFinish` stays |
| | refract | waits other | 9.4 | no change (-0) | | the resolve is not a counted caller there |
| | shadow, refract | kicks | 3.0, 13.0 | 3.0, 13.0 | | the ground's target change still ends K2 until the kick is continued across targets (kick-continue-design); draws unchanged |

Reading the table after a fix: run both commands above on the fix's commit;
fill in the **after** column with the fix's commit hash in the heading of the
rows it touches; list any other row that moved.

## What this does not show

- Times. A wait is at least one 1 ms poll on the board, but the lag model is
  the ceiling: a board whose GPU finishes a kick before the CPU checks it
  pays fewer (T: 5 to 7 blocking waits for terrain against 46.2 here).
- CPU loads and stores. `cache_invd` and `cache_flush` bytes stand for the
  passes; a change that only widens a loop (resolve-wide) moves nothing here.
- Pixels. The draw box is an upper bound; coverage is a board matter.
- The board: the X2's numbers (x2-gm-before, x2-gm-after) set ms a frame
  beside these counts.
