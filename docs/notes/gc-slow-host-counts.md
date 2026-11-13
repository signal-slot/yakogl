<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: host counts of glmark2 terrain, shadow and refract, before the fixes and at origin/main 8bd7cc04

Plan #328, item host-cost-baseline (issues #1204, #1106). Counts only, from
the harness's GC cost mode (`tools/hostharness/glmark2/README.md`, "GC7000
costs"; `FINDINGS-gc-costs.md`): no times, no FPS, no board. No code
changed. Each fix of the plan cites its change against the **baseline**
columns here and fills the **after** column; the X2 numbers
(x2-gm-after) are set beside these. The sections after "Baseline" are
recounts; the latest, "At origin/main 8bd7cc04" (end of the note), has the
figures after mip-one-kick, resolve-own-kick and gc-cache-skip.

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

## After tiled-copy-rs (0a922ae2)

Plan #328, item host-recount. Counts only, no code changed. The tip is
integration 0a922ae2 (2026-11-07), which holds tiled-copy-rs as c707598d
(6e15c9fb on the integration branch before promotion); no other fix of the
"What each fix should move" table is in it. Same glmark2 22c527cb, same two
commands as "Run", 11 frames, frames 2 to 11. Rows from
`<out>/gc-cost-vivante.md`; the lag run unless a row says `done`.

| | terrain base | terrain after | shadow base | shadow after | refract base | refract after |
|---|---:|---:|---:|---:|---:|---:|
| waits in the swap (lag) | 1.8 | 1.8 | 0.9 | 0.9 | 2.3 | 2.3 |
| **waits other (lag)** | 46.2 | **48.2** | 1.8 | **1.8** | 9.4 | **9.4** |
| waits, all (done) | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| **kicks** | 52.0 | **52.0** | 3.0 | **3.0** | 13.0 | **13.0** |
| draws in the streams | 59.0 | 59.0 | 5.0 | 5.0 | 14.0 | 14.0 |
| draw box Mpx | 4.89 | 4.89 | 5.28 | 5.28 | 5.44 | 5.44 |
| RS blits (Mpx), lag | 0.0 (0.00) | 2.0 (0.13) | 0 | 0 | 0 | 0 |
| **invalidated KiB (calls)**, lag | 512.0 (2.0) | **0.0 (0.0)** | 7600.0 (1.0) | **7600.0 (1.0)** | 7600.0 (1.0) | **7600.0 (1.0)** |
| **flushed KiB (calls)**, lag | 604.0 (118.0) | **92.4 (116.0)** | 7506.4 (11.0) | **7506.4 (11.0)** | 7522.2 (29.0) | **7522.2 (29.0)** |
| foreign KiB | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| stream KiB | 74.1 | 74.4 | 5.7 | 5.7 | 17.9 | 17.9 |

Terrain waits other by call site (lag, a frame): `record_level` (the
mipmap chains' `frame_open`) 40.0 to 43.0; `gc_draw`'s `frame_open` 5.0 to
5.0; `call<gc_draw`, the tiled copy's `finish()`, 1.0 to 0 (gone); swap 1.8
and `glFinish` 0.2 unchanged. Kicks by entry point are unchanged
(`glGenerateMipmap` 48.0, `glDrawArrays` 3.0, swap 0.9, `glFinish` 0.1). The
two RS blits (0.131 Mpx) are in the 800x600 chain starts' kicks
(`glGenerateMipmap`, 3.0 kicks, 7.0 KiB of stream).

### Predictions of "What each fix should move", tiled-copy-rs

| row | expected | after | held |
|---|---:|---:|---|
| terrain invalidated KiB (calls) | 0 (0) | 0.0 (0.0) | yes |
| terrain flushed KiB | about 92 | 92.4 (116.0 calls, from 118.0) | yes |
| terrain waits other, `call<gc_draw` | 0 | 0 | yes |
| terrain waits other, total | 45.2 | **48.2** | **no**: +2.0, not -1.0 |
| terrain RS blits | 2 (0.13 Mpx) | 2.0 (0.13 Mpx), one RS copy per map | yes |
| terrain kicks, draws | 52, 59 | 52.0, 59.0 | yes |
| terrain stream KiB | up, under 1 KiB | 74.1 to 74.4 (+0.3) | yes |
| shadow, refract, all rows | no change | no change, to the digit | yes |

- **The total did not hold.** The `finish()` is gone as predicted, but the
  chains' `record_level` waits rose by 3.0 (40.0 to 43.0), so the lag count
  went from 46.2 to 48.2. Kicks are the same 52, so the same 52 `frame_open`
  checks are made; three more of them now find a pending kick. The likely
  cause is the lag model: the `finish()` used to complete every pending
  kick before the next chain start, and without it the 800x600 chain starts
  (3 a frame) find the kick before them pending. Not traced further. On the
  board it is the blocking waits that count (T: 5 to 7), which this run does
  not give; the lag count is the ceiling, and it rose by 2. Fixes that remove
  `record_level`'s waits (mip-one-kick, gc-frames-ring) should be compared to
  48.2, not 46.2.
- **The `done` run is no longer identical to the lag run.** The baseline
  says every non-wait count is the same in both runs. After tiled-copy-rs the
  terrain row of the `done` run is: RS blits 1.0 (0.07 Mpx), invalidated 256.0
  KiB (1.0 call), flushed 348.2 KiB (117.0), stream 74.2 KiB; kicks, draws and
  box as in the lag run. With every kick done at once, one of the two maps
  takes the CPU copy (256 KiB read, 256 KiB copy flushed) and one the RS
  blit. So the RS path depends on the producer kick being pending when the
  copy is recorded, and the 0 KiB read above holds for the lag model only; on
  the board the count lies between the two runs (0 to 256 KiB read, 92 to 348
  KiB flushed). Not traced further. Shadow and refract are identical in both
  runs.
- The other fixes of the table are not filled in: none of them is in
  0a922ae2.

## At origin/main 8bd7cc04

Plan #328, item tiled-depth (issue #1204). Host model figures, not board
times: counts of what the counted frames would make the i.MX8MP's CPU pay
its GPU. Board ms are in `x2-glmark2-slow-scenes.md`. No code changed. The tip
is origin/main `8bd7cc04` (2026-11-10), which holds every fix of the table
"What each fix should move" except resolve-wide (in review) and the ones
that wait for `backend.rs` (rs-clear-wire, gc-frames-ring) or the board
(depth-sample-gc). Same glmark2 22c527cb, same two commands as "Run" (`done`
run: `GM2_GCCOST_GPU=done`), 11 frames, counted frames 2 to 11, no GL error,
no allocation refused. Rows are from `<out>/gc-cost-vivante.md` of each run;
the lag run unless a row says `done`. "Previous" is the section above, "After
tiled-copy-rs (0a922ae2)".

Merged since that recount, by what each can move in these counts:

- `d3ce1261` mip-one-kick: a GPU mipmap chain is one kick, not one kick a level.
- `e320630d` resolve-own-kick: the depth resolve waits for the kick that wrote the depth.
- `e58be17b` gc-cache-skip: no data-cache maintenance on the carve-out. It is in
  `src/osal/zephyr_gc.c`; the harness puts its own OSAL table in front of
  linux.c's, so **it sees no change from this commit** (the predicted
  "stays" case). The KiB below are the stand-in for the CPU passes, not the
  board's cache maintenance.
- `732bde0f` fb-counters: only counters (`gc_backend_mip_chains`); the harness
  does not print them (see "Mip chains" below).
- `434d2695`, `099b5df6`, `37c90b49` (noise, sr and terrain compiler items):
  "no code change" commits, measured only. A compiler change moves GPU
  instruction counts, not any figure of this host model: **none explains any
  change below.**
- `c707598d` tiled-copy-rs: already in the previous column.

### Terrain

| | previous | now (8bd7cc04) | explained by |
|---|---:|---:|---|
| **kicks a frame** | 52.0 | **14.0** | mip-one-kick: 5 chains are 5 kicks, not 43 level kicks (52 - 43 + 5) |
| waits in the swap (lag) | 1.8 | 2.8 | none named; more pending kicks at the swap in the lag model, not traced |
| **waits other (lag)** | 48.2 | **10.2** | mip-one-kick: `record_level` 43.0 to 5.0 (one `frame_open` a chain); `gc_draw`'s `frame_open` 5.0 to 5.0; `glFinish` 0.2 |
| waits, all (done) | 0.0 | 0.0 | |
| **carve-out CPU KiB a frame**, invalidated (calls), lag | 0.0 (0.0) | **0.0 (0.0)** | tiled-copy-rs holds |
| flushed (calls), lag | 92.4 (116.0) | **68.4 (28.0)** | mip-one-kick: the kick's two flushes a chain, not a level (stream KiB down too). gc-cache-skip: none (not seen) |
| invalidated (calls), `done` | 256.0 (1.0) | 256.0 (1.0) | unchanged: one map's CPU copy |
| flushed (calls), `done` | 348.2 (117.0) | 324.2 (29.0) | as the lag flushed row |
| foreign KiB | 0.0 | 0.0 | |
| stream KiB (lag, `done`) | 74.4, 74.2 | 51.3, 51.1 | mip-one-kick: no per-level prologue and state |
| draws in the streams | 59.0 | 54.0 | probably mip-one-kick (one draw fewer a chain, 5); not traced |
| draw box Mpx | 4.89 | 4.15 | same cause, -0.74 Mpx; not traced |
| RS blits (Mpx), lag | 2.0 (0.13) | **7.0 (0.87)** | none predicted; **not traced** (below) |
| RS blits (Mpx), `done` | 1.0 (0.07) | 6.0 (0.81) | same |

Kick pass sizes (lag, a frame, draw box Mpx; kicks a frame): the chain kicks
are 3 at 400x300 (0.480 Mpx, 27 draws) and 2 at 128x128 (0.044, 16 draws), 9
and 8 draws a kick, 7.6 and 6.8 KB of stream each. The passes they follow:
800x600 stride 3200: 3 kicks `glGenerateMipmap` (1.920), 1 `glDrawArrays`
(0.480), 0.9 swap (0.864), 0.1 `glFinish` (0.096); 256x256 stride 1024: 2
`glDrawArrays` and 2 `glGenerateMipmap` (0.131 each). Previous: 43 one-level
kicks (draw box 0.5 Mpx together) and the same 9 other kicks.

Mip chains a frame: **5 made by the GPU (3 from the 800x600 target, 2 from
256x256), 0 refused (CPU-filtered)**; previous: 5 made as 43 level kicks, 0
refused. The count is read off the kicks (the chain kicks are the ones with 8
render targets) and from the library's log: it logs the first GPU chain
(`10-level chain of a 512x512 texture`) and has no line "left to the CPU" in
any phase. The harness does not read `gc_backend_mip_chains`; a printed
per-frame counter is not in the harness.

Tiled copies a frame: lag **0 CPU copies** (0 KiB read), all maps tiled by the
RS in the stream (previous: the same). `done`: **1 CPU copy** (256 KiB read,
256 KiB flushed), 5 maps by RS (previous: 1 and 1).

### Shadow

| | previous | now (8bd7cc04) | explained by |
|---|---:|---:|---|
| **kicks a frame** | 3.0 | **3.0** | unchanged; no mipmap |
| waits in the swap (lag) | 0.9 | 0.9 | |
| **waits other (lag)** | 1.8 | **1.1** | resolve-own-kick: `resolve_depth` 1.7 to 1.0 (K2 only); `glFinish` 0.1 |
| waits, all (done) | 0.0 | 0.0 | |
| **carve-out CPU KiB a frame**, invalidated (calls) | 7600.0 (1.0) | **7600.0 (1.0)** | unchanged: the CPU depth resolve reads the D24S8 surface; depth-sample-gc would remove it |
| flushed (calls) | 7506.4 (11.0) | **7506.1 (8.0)** | KiB: none, 0.3 KiB less. Calls 11 to 8: **none named, not traced** |
| invalidated / flushed, `done` | 7600.0 / 7506.4 | 7600.0 / 7506.1 | same |
| stream KiB | 5.7 | 5.7 | |
| draws, draw box Mpx | 5.0, 5.28 | 5.0, 5.28 | |
| RS blits | 0 | 0 | |

Kick pass sizes (draw box Mpx a frame): 1600x1200 stride 0 (the depth pass)
1 kick, 2 draws, 3.840; 800x600 stride 3200: 1 `glDrawArrays` (0.480), 0.9
swap (0.864), 0.1 `glFinish` (0.096). Same as before. Mip chains: none made,
none refused. Tiled copies: none.

### Refract

| | previous | now (8bd7cc04) | explained by |
|---|---:|---:|---|
| **kicks a frame** | 13.0 | **4.0** | mip-one-kick: the 10 level kicks of the one chain are 1 kick (13 - 10 + 1) |
| waits in the swap (lag) | 2.3 | 0.9 | mip-one-kick (fewer pending kicks at the swap); not traced further |
| **waits other (lag)** | 9.4 | **1.1** | mip-one-kick: `record_level` 8.0 to 0; `resolve_depth` 0 to 1.0 (the chain's waits no longer finish the depth kick first, so the resolve finds it pending; with resolve-own-kick it waits for that kick only); `glFinish` 0.4 to 0.1. The `gc_draw` `frame_open` 1.0 is gone |
| waits, all (done) | 0.0 | 0.0 | |
| **carve-out CPU KiB a frame**, invalidated (calls) | 7600.0 (1.0) | **7600.0 (1.0)** | unchanged (CPU depth resolve) |
| flushed (calls) | 7522.2 (29.0) | **7517.3 (9.0)** | mip-one-kick: 20 level flushes less; the KiB are the stream (17.9 to 13.3) |
| invalidated / flushed, `done` | 7600.0 / 7522.2 | 7600.0 / 7517.3 | same |
| stream KiB | 17.9 | 13.3 | mip-one-kick |
| draws, draw box Mpx | 14.0, 5.44 | 14.0, 5.44 | unchanged |
| RS blits | 0 | 0 | |

Kick pass sizes (draw box Mpx a frame): 1600x1200 stride 6400 (K2) 1 kick, 2
draws, 3.840, a `glGenerateMipmap`-attributed kick; the chain 800x600 stride
6400, 1 kick, 10 draws, 0.640; 800x600 stride 3200: 1 `glDrawArrays` (0.480),
0.9 swap (0.432), 0.1 `glFinish` (0.048). Previous: 10 one-level kicks for the
same chain. Mip chains a frame: **1 made (logged as the `11-level chain of a 1600x1200
texture`), 0 refused**; previous 1 and 0. Tiled copies: none.

### What did not hold, and what is open

- The previous section's prediction "refract: the resolve is not a counted
  caller" is off by 1.0 now (see above); it follows from mip-one-kick, which
  that prediction did not account for.
- Terrain's RS blits rose from 2.0 to 7.0 (0.13 to 0.87 Mpx): six 256x256-sized
  blits (two in `glDrawArrays` kicks, four in `glGenerateMipmap` kicks) and
  one 800x600 blit in a chain-start kick, instead of two. None of the merged
  commits was predicted to change it. **Not traced**; the likeliest reading is
  that with fewer waits between kicks the producer is still pending when
  the tiled copy is refreshed, so each refresh takes the RS path, as the
  `done`-run difference of the previous section already showed.
- Terrain's `waits other` is 10.2 in the lag model: 5.0 `record_level` and 5.0
  `gc_draw` `frame_open` checks, one a kick that finds a kick pending.
  Removing them is gc-frames-ring's (a ring of at least 14 slots now
  covers a frame's kicks, not 52).
- Terrain stays the heaviest of the three on kicks and waits; shadow and
  refract stay the heaviest on CPU KiB (7.4 MiB read and 7.3 MiB flushed a
  frame, the depth resolve), which only depth-sample-gc removes. gc-cache-skip
  is not in these figures by construction.

## Predicted effect of gc_tiled_rt (host-derived, plan #355)

Plan #355, item session-a-predict (issues #1204, #1106, #15). **Every number
in this section is host-derived: counts from the host harness and
milliseconds computed from board figures cited here. None is a board
measurement.** No X2 ms a frame exists yet (`x2-glmark2-slow-scenes.md`);
session A (`gc-tiled-wire.md`, "Session A: the run sheet") measures them with
the knob off and on, and this section is what its numbers are read against.
No code or test is committed with it.

### How the counts were taken

- Tip `fac26ee1`, which holds `gc_tiled_rt` (W1 to W5 of `gc-tiled-wire.md`,
  "Status"). The harness is the shared one of `tests/gc_scene_kicks.rs`
  (`tests/common/gc_scenes.rs`), **not** the glmark2 host harness of the
  sections above: it is a model (small meshes and short shaders, glmark2's
  targets, clears, draws and `glGenerateMipmap` in glmark2's order), so its
  counts differ from the glmark2 runs above. `gc_scene_kicks` reports 11 kicks
  for terrain, 2 for shadow and 3 for refract at this tip (the ceilings in the
  file are 14, 3 and 4; 8bd7cc04 counted 14, 3 and 4 in the glmark2 harness).
  Compare off with on **inside this section**, not with the tables above.
- Frames 3 to 6 (4 counted), a mean a frame; both GPU models (`Lag`, `Done`),
  which agree except where a row says so.
- Knob on: a local, uncommitted `gc_debug_set_tiled_rt(1)` after `gc_init`
  (`test-knobs`). Stream words, draws, RS jobs and draws with
  `PE_DEPTH_CONFIG` `EARLY_Z` set were read off each submitted kick's command
  stream by a local, uncommitted decoder in the harness's `submit` stand-in
  (LOAD_STATE writes tracked across kicks; an RS job is a write to
  `RS_KICKER`, classified by `RS_CONFIG` `SOURCE_TILED` / `DEST_TILED`, the
  stride `TILING` bits and `RS_CLEAR_CONTROL`). Command, from the repository
  root:

```sh
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2,test-knobs --test <local test> -- --nocapture
```

  The first scene run of a process counted fewer draws (terrain 51, not 71)
  and was discarded; the figures are of a warm run, the same in `Lag` and
  `Done`. "Draws" are every draw op of the stream, clear quads included
  (`gc_scene_kicks` has no draw count).

### What the counts say before any millisecond

The knob adds work only where a draw qualifies for a tiled shadow
(`gc-tiled-wire.md`, W4: one 4-byte colour target, `w % 16 == 0`, `h % 4 == 0`,
depth test on with a depth **renderbuffer**). In this harness that is terrain's
800x600 pass with the depth16 renderbuffer; shadow's and refract's 1600x1200
passes have a depth **texture** and are not taken. Why each other draw is
refused is item tiled-qualify-census's, not traced here.

### Terrain

| a frame (mean, frames 3 to 6) | knob off | knob on | change |
|---|---:|---:|---:|
| kicks (`Lag`, `Done`) | 11 | 11 | 0 |
| waits outside the swap (`Lag`; `Done` 0) | 11 | 11 | 0 |
| RS jobs, fill (colour and depth clears) | 11.2 | 11.2 | 0 |
| RS jobs, linear to tiled (off: the tiled copies of the two vertex-stage maps; on: and the tile-in of a shadow) | 2 | 3 | **+1** |
| RS jobs, tiled to linear (the copy back) | 0 | 2 | **+2** |
| RS jobs, all | 13.2 | 16.2 | **+3** |
| draws in the streams | 71 | 71 | 0 |
| draws taking early depth | 0 | 2 | **+2** |
| stream words | 16378 | 16510 | +132 (0.5 KB) |
| CPU tiled copies, CPU depth resolves | 0, 0 | 0, 0 | 0 |

The extra jobs are 800x600x4 = 1.92 MB each (the terrain target; the 256x256
maps are 0.26 MB). The two early-depth draws are the harness's draws into the
tiled shadow; the real number is glmark2's terrain draw count and cannot be
read off this model.

Predicted change, a frame, **host-derived**:

- **Cost: +0.7 to +3 ms.** 3 RS jobs of 1.92 MB, each 0.24 to 0.96 ms at the RS
  rate of 2 to 8 GB/s that `gc-slow-stream-diff.md` (finding 1, "Full-target
  clears drawn as quads", its cost column) assumes, with the bytes counted
  once; up to 1.9 ms each if the read and the write both count. That rate is
  itself unmeasured (`gc-tiled-targets.md`, "0. Board probe: layouts and RS
  rates"). For scale, the CPU copy of the same 1.92 MB took 12.3 to 12.9 ms on
  the board (`gc-slow-terrain.md`, "Board cost" of the tiled copies, from
  `imx8mp-frame-floor.md`, "Step 3 result"); the RS is expected well under it.
  No kick is added, so the 0.03 to 0.08 ms of GPU overhead a kick
  (`gc-slow-stream-diff.md`, finding 3) does not move; the +132 words are noise
  against the 6 to 76 KB a frame the FE already fetches (the same note,
  finding 6).
- **Gain: 0 to -14 ms** (to -19 at 0.8 ns an instruction): early depth rejects
  hidden fragments of the 148-instruction terrain pass, 42.6 ms at full cover
  (`gc-tiled-targets.md`, "5. Early depth", Gain, and "The part"). The note puts
  the range's width on `make_grid`'s draw order; the model of that order is
  item terrain-overdraw's, in review and not in this tree, so the range is not
  narrowed here.
- **Net: from about -14 ms to about +3 ms**, the sign depending on the draw
  order. Of a board frame of about 250 ms (`gc-slow-terrain.md`, before plan
  #328's fixes) this is at most a twentieth: the knob cannot take terrain off
  its GPU-shading bound (140 to 205 ms, the same note, rank 1).

Reading rule for session A, terrain (D = ms a frame with the knob on minus off,
same image otherwise; the knob switches W4 and W5 together, so D cannot
separate them):

- **Helps:** D <= -3 ms. The RS cost cannot exceed about 3 ms on the model, so
  the rest is early depth.
- **Hurts:** D >= +0.7 ms, up to +3 ms: "terrain got slower by about the cost of
  three RS copies a frame: one tile-in, two copy-backs", with no early-depth
  gain (the pass draws far to near, or the hidden share is small). D > +3 ms is
  more than the model: an RS rate under 2 GB/s, or an RS job that stalls the
  pipe (look first at whether the copy-back waits for the kick's draws).
- **Does nothing:** -3 ms < D < +0.7 ms, inside the spread of repeated runs.
  The two effects may cancel; the host cannot say, and the pair of runs does not
  either. Say "no net effect", not "no early depth".
- Kicks, waits and CPU tiled copies must not move (the first rows above); a
  PERF counter showing one more means the knob did something the host does not.

### Shadow

| a frame (mean, frames 3 to 6) | knob off | knob on | change |
|---|---:|---:|---:|
| kicks (`Lag`, `Done`) | 2 | 2 | 0 |
| waits outside the swap (`Lag`; `Done` 0) | 1 | 1 | 0 |
| RS jobs, fill | 3 | 3 | 0 |
| RS jobs, linear to tiled / tiled to linear | 0 / 0 | 0 / 0 | 0 |
| draws in the streams | 3 | 3 | 0 |
| draws taking early depth | 0 | 0 | 0 |
| stream words | 834 | 834 | 0 |
| CPU depth resolves (1600x1200) | 1 | 1 | 0 |

The streams are the same size: no draw is taken (the depth pass renders to a
depth texture). Predicted change, **host-derived**: **0 ms**. The part of the
frame that dominates is not touched: the CPU depth resolve is 350 to 1000 ms on
the board by its own estimate (`gc-slow-shadow-refract.md`, cause 1, from the
AM62P's 349 ms for 1.92 M texels), which nothing in W4 or W5 removes
(depth-sample-gc does, `gc-depth-sample.md`). Even if early depth took the depth
pass, the cited range is 0 to 1 ms (`gc-tiled-targets.md`, "5. Early depth",
shadow; `gc-slow-stream-diff.md`, finding 4).

Reading rule for session A, shadow:

- **Does nothing (the prediction):** |D| within the spread of repeated runs.
  Shadow stays at its resolve-bound frame with the knob on.
- **Helps:** D < 0 beyond the spread can only be early depth on the depth pass
  or a draw the host harness does not show taking a shadow; at most 1 ms by the
  cited range. A larger D is not this knob: look for another difference between
  the two runs.
- **Hurts:** "shadow got slower by about the cost of one more RS copy a stream:
  the tile-in cost" means a stream with a tiled shadow that the host harness
  does not have. The size says which target: 0.2 to 1 ms for an 800x600 one
  (1.92 MB), 1 to 4 ms for the 1600x1200 one (7.68 MB at 2 to 8 GB/s), a job.
  The record's `gc_tiled_rt` log lines and item tiled-qualify-census say which
  draw.

### Refract

| a frame (mean, frames 3 to 6) | knob off | knob on | change |
|---|---:|---:|---:|
| kicks (`Lag`, `Done`) | 3 | 3 | 0 |
| waits outside the swap (`Lag`; `Done` 0) | 1 | 1 | 0 |
| RS jobs, fill | 4 | 4 | 0 |
| RS jobs, linear to tiled / tiled to linear | 0 / 0 | 0 / 0 | 0 |
| draws in the streams | 12 | 12 | 0 |
| draws taking early depth | 0 | 0 | 0 |
| stream words | 2924 | 2924 | 0 |
| CPU depth resolves (1600x1200) | 1 | 1 | 0 |

Predicted change, **host-derived**: **0 ms**, for shadow's reason: the
1600x1200 colour and depth pass has a depth texture and is not taken. The frame
is bound by the same CPU depth resolve (`gc-slow-shadow-refract.md`, cause 1).
The early-depth range for refract is 0 to 2 ms (`gc-tiled-targets.md`, "5. Early
depth", refract).

Reading rule for session A, refract: as shadow's. A knob-on frame within the
spread is the prediction; a gain of up to 2 ms would be early depth on a draw
the host does not show; a slowdown of 0.2 to 1 ms (800x600) or 1 to 4 ms
(1600x1200) a job is an RS tile-in or copy back the host harness does not have.

### Across the three scenes

- Only terrain's D can be large: from about -14 to +3 ms of about 250. Shadow's
  and refract's frames are for depth-sample-gc, not this knob.
- A D outside its stated bound is a finding about the model (a draw the host
  does not take, an RS rate, a stall), not about the knob: record the numbers
  and the knob's log lines, and open the issue for it.
- The RS rate (2 to 8 GB/s) and the hidden-fragment share of terrain are the two
  inputs no board has measured; session A replaces them.

## Predicted effect of W7 to W9, tile status and fast clear (host-derived, plan #355)

Plan #355, item w5-early-z (issues #1204, #1106, #15). Docs only. **Every
figure is arithmetic on counts the host harness reports and on bandwidths the
GC7000UL notes already cite. None is a board measurement, and the RS rate is
itself unmeasured.** This is what the case 243 probe's answer and a later X2
run are read against.

### What it replaces in `gc-tiled-wire.md`

The order table of `gc-tiled-wire.md` gives W8 "terrain 1 to 5 ms, refract 2.5
to 7.5 ms". Those are `gc-tiled-targets.md` change 4's figures, and they price
each clear as a **quad** at 3.9 ns a pixel (terrain's "four FBO clear quads at
800x600 and the four 256x256 ones", 4.8 ms; refract's one fused 1600x1200
quad, 7.5 ms). That is no longer the stream: plan #328 landed the RS fill
clears (`rs_clear.rs`), and the "RS jobs, fill" rows above (terrain 11.2,
shadow 3, refract 4 a frame) are RS jobs, not draws. What W7 to W9 can still
remove is the **bytes** a fill writes, not a job: a TS fill is also one RS job.
So every RS job count below is unchanged; the fill shrinks to the tile status
(`gc-tiled-targets.md`, "Fast clear": 30 KB for a 1600x1200 surface instead of
7.68 MB). The quad-priced figures overstate the gain at this tip.

### Inputs, and the commit read

- Commit read: `b2673e3c` (the workspace's tip: W1 to W5 are in it, W7 to W9
  are not).
- Counts: `cargo test --no-default-features --features
  host-test,gpu-vivante,shaderc-online,report-es2 --test gc_scene_kicks --
  --nocapture` (3 pass; kicks 11, 2, 3 for terrain, shadow, refract, as above),
  and the clear call sites of `tests/common/gc_scenes.rs` (`canvas_clear`,
  `Renderer::render_texture`, `terrain()`, `shadow()`, `refract()`). Sizes:
  `CANVAS_W` x `CANVAS_H` = 800x600; shadow and refract's targets are twice
  that, 1600x1200.
- Surface bytes: a colour target is A8R8G8B8, 4 bytes a pixel. A depth surface
  is the backend's D24S8, 4 bytes a pixel in the padded allocation
  (`src/vivante/ds_cache.rs:3`, `:334`; `gc-slow-stream-diff.md` finding 1:
  "one fill for colour and one for depth over the padded D24S8 allocation"),
  so terrain's 16-bit renderbuffer costs 4 bytes a pixel as well.
- TS size: one 4-bit entry a 128-byte tile (`gc-tiled-targets.md`, 7.68 MB /
  128 B = 60 000 tiles = 30 KB), aligned to `0x100` (`gc-tiled-wire.md` W7).
  800x600x4 = 1.92 MB is 15 000 tiles, 7 500 bytes, 7 680 aligned;
  1600x1200x4 is 30 000 bytes; 256x256x4 = 0.262 MB is 1 024 bytes.
- Bandwidth: the RS fill rate of **2 to 8 GB/s, bytes counted once**, the
  assumption of `gc-slow-stream-diff.md` finding 1 and `:225-226` ("at the low
  end the RS is no faster per byte than the quad's 1.9 GB/s, at the high end it
  is half the LPDDR4"), unmeasured until step 0 (iv) of `gc-tiled-targets.md`.
  The only measured figure is the quad's 1.9 GB/s written, 1 981 us for 0.48
  Mpx (`imx8mp-frame-floor.md`, "Step 3 result", as quoted there). A TS fill of
  7.7 KB to 30 KB costs 0.001 to 0.015 ms at the same rates, below 1 % of the
  bytes it replaces; it is not subtracted below.
- What W7 to W9 reach: W8 and W9 apply to a **W4 shadow** (one 4-byte colour
  target, `w % 16 == 0`, `h % 4 == 0`, depth test on with a depth
  **renderbuffer**) and to a PE-private `ds_cache` depth surface
  (`gc-tiled-wire.md` W8, W9). In the harness that is terrain's 800x600 pass
  only (the tables above: shadow's and refract's 1600x1200 passes have a depth
  **texture**). A depth texture needs W12 (a TS-aware CPU depth resolve), a
  mipmapped or sampled colour texture needs W10 and W11.

### The clears of one frame, from the harness

| scene | clear (call site) | colour | depth | fill jobs | bytes |
|---|---|---:|---:|---:|---:|
| terrain | canvas clear (window, linear) | 800x600 | 800x600 | 2 | 3.84 MB |
| terrain | height_map, normal_map (no depth) | 2 x 256x256 | none | 2 | 0.52 MB |
| terrain | **terrain pass** (depth16 renderbuffer, fused) | 800x600 | 800x600 | 2 | **3.84 MB** |
| terrain | bloom_h, bloom_v (no depth) | 2 x 256x256 | none | 2 | 0.52 MB |
| terrain | tilt_h (no depth) | 800x600 | none | 1 | 1.92 MB |
| terrain | tilt_v (window, `onscreen`, fused) | 800x600 | 800x600 | 2 | 3.84 MB |
| terrain | total | | | 11 (11.2 measured) | 14.5 MB |
| shadow | depth pass (depth texture, depth only) | none | 1600x1200 | 1 | 7.68 MB |
| shadow | canvas clear | 800x600 | 800x600 | 2 | 3.84 MB |
| shadow | total | | | 3 (3 measured) | 11.5 MB |
| refract | distance target (textures, fused) | 1600x1200 | 1600x1200 | 2 | 15.36 MB |
| refract | canvas clear | 800x600 | 800x600 | 2 | 3.84 MB |
| refract | total | | | 4 (4 measured) | 19.2 MB |

The job totals agree with the "RS jobs, fill" rows above (terrain's 0.2 is a
mean over frames 3 to 6 that this note does not attribute). The byte totals
agree with `gc-slow-stream-diff.md` finding 1 for shadow and refract (11.5 and
19.2 MB); for terrain that note has 13.5 MB in the glmark2 harness against 14.5
here (two different models; the 1 MB is not traced). The window's clears are
linear and not a shadow: only `gc-tiled-wire.md` W13 would reach them.

### Terrain

W7 to W9 as wired reach the terrain pass only.

- Removed: its colour fill (1.92 MB) and its depth fill (1.92 MB), 3.84 MB in 2
  jobs. The jobs stay, each writing 7 680 bytes of TS.
- 3.84 MB at 2 to 8 GB/s is **1.92 to 0.48 ms** (W8 half, W9 half: the two
  surfaces have the same size).
- The resolve that reads through TS (W8's TS-sourced copy of the same 1.92 MB)
  does not fetch cleared tiles: **0 to 0.24-0.96 ms**, 0 if the pass draws
  every tile, the whole 1.92 MB read (8 to 2 GB/s) if it draws none. The
  pass's coverage is an open input (terrain-overdraw models it, not in this
  tree), so this is an upper bound, not a middle.
- **Prediction, W7 to W9 after W4/W5: terrain 0.5 to 2.9 ms a frame faster**
  (0.48 + 0 to 1.92 + 0.96). Of a board frame of about 250 ms
  (`gc-slow-terrain.md`, before plan #328's fixes) that is 0.2 to 1.2 %. Rank 1
  of that note, the GPU shading of the 9 passes (140 to 205 ms), is untouched.
- **If W10 and W11 then widen it to every offscreen colour target** (change 4
  of `gc-tiled-targets.md`, not W8 as wired): tilt_h's 1.92 MB and the four
  256x256 maps' 1.05 MB (0.26 each) join, 3.84 + 2.97 = 6.81 MB, **0.85 to 3.4
  ms** removed. The chains (terrain, bloom_h, bloom_v, tilt_h) have several
  levels, cannot use the sampler's TS, and a TILED source has no in-place
  resolve, so each needs a copy of its surface: 1.92 + 1.92 + 2 x 0.262 = 4.37
  MB, **0 to 0.55-2.2 ms** (0 where W4's copy-back already resolved them).
  Net **-1.3 to +3.4 ms**. That is the replacement for the table's "1 to 5":
  its top, 4.8 ms, is quads.
- Not counted, no figure in the notes: the PE's depth reads of cleared tiles,
  and tiled sampling's cache locality (`gc-tiled-targets.md` change 4).

### Shadow

- As wired W7 to W9 remove **nothing**: its only non-window clear is the depth
  texture (7.68 MB, 1 job), neither a W4 shadow nor a PE-private surface.
  Prediction **0 ms**, 0 jobs, 0 bytes.
- With W12 (the depth texture gets a TS, the CPU resolve reads through it) W9's
  fill reaches it: 7.68 MB becomes a 30 KB TS fill, **0.96 to 3.84 ms**
  removed. That is the table's "fast clear of depth: shadow 7.5 ms" at the
  quad's 3.9 ns a pixel. Against a shadow frame of 350 to 1000 ms of CPU depth
  resolve (`gc-slow-shadow-refract.md`, cause 1) it is 0.1 to 1 %; the
  TS-aware read of W12b (30 to 150 ms, "if the CPU resolve stays") is the gain
  that matters, and is not W7 to W9's.

### Refract

- As wired **0 ms**: the fused 1600x1200 colour and depth clear (15.36 MB, 2
  jobs) goes to textures, which W4 refuses and W9 does not reach.
- With W10, W11 and W12 (a tiled colour texture, a depth texture with TS):
  15.36 MB of fills, **1.9 to 7.7 ms** removed. The colour's level 0 is sampled
  by the `glGenerateMipmap` chain and the refraction pass, so it needs a resolve
  (TILED: a copy of all 7.68 MB), **0 to 0.96-3.8 ms**. Net **-1.9 to +7.7 ms**.
  The table's "2.5 to 7.5 ms" is the same sum priced as a quad (7.5 ms for
  1.92 Mpx of colour and depth); at RS rates the loss side is wider.
- Of a refract frame of about 500 ms (`gc-tiled-targets.md`) the gain is at
  most 1.5 %; its dominant cost is the CPU depth resolve of shadow's cause.

### Across the three scenes

| scene | bytes removed (jobs unchanged) | as wired, after W4/W5 | with W10 to W12 |
|---|---:|---:|---:|
| terrain | 3.84 MB (colour 1.92, depth 1.92) | **0.5 to 2.9 ms** | -1.3 to +3.4 ms |
| shadow | 0 | **0 ms** | 0.96 to 3.84 ms (W12 only) |
| refract | 0 | **0 ms** | -1.9 to +7.7 ms |

- W7 to W9 remove **no RS job** in any scene. A PERF page that counts RS jobs
  does not move; the bytes-written page (page 3 v3, KiB written,
  `gc-slow-stream-diff.md`) falls by the bytes above (terrain as wired: 3.84
  MB, 3 750 KiB a frame).
- Every gain is a small share of a frame of 250 to 1 000 ms. W8 is a step that
  W10 builds on, not a speed item by itself (as the order table says).
- The RS rate and terrain's drawn-tile share are the inputs no board has
  measured. A fill of one 1600x1200 surface timed by RS (step 0 (iv) of
  `gc-tiled-targets.md`) narrows every range above by up to a factor of four.

### What case 243 must answer before W7 is wired

`gc-ts-probe.md`, "Part (iii): case 243 `tsprobe`" (K0 control resolve, K1 TS
fill, K2 TS-sourced resolve, K3 pattern resolve). The terrain range above
stands only if:

1. **The TS fill lands with the MMU off** (not `FILL_MISS` or `NO_WRITE`): the
   RS writes `T` at the address the back end gives every register.
2. **The TS-sourced resolve honours the clear code** (arm 0 `YES`, not
   `IGNORED`): a cleared tile reads as `TS_COLOR_CLEAR_VALUE`. Arm 0 is TILED,
   128B mode, no compression, 64x64 A8R8G8B8: W7 and W8's configuration, so the
   answer covers the terrain pass's 800x600 target.
3. **Mode and compression** (arms 1 and 2: the `AUTO_DISABLE` words; the 256B
   mode with Mesa's `0x382`): they change the TS size (256B granules halve it,
   to 3.8 KB for a 1.92 MB surface) and not the bytes the fills save.
4. **Not answered by 243**: the depth TS (W9's board check, "likely, not
   proven" in `gc-ts-probe.md`), the sampler TS (W10's), and the TS-aware depth
   read (W12, with case 244's layout answer). The terrain range assumes colour
   and depth both work; with colour only, take **0.24 to 1.9 ms** (the colour
   fill 1.92 MB, 0.24 to 0.96 ms, plus the resolve read, 0 to 0.96 ms).

If arm 0 is `IGNORED`, `NO_WRITE`, `FILL_MISS` or hangs, W7 to W9 stay off and
every range above is 0; a hang also stops the session and releases the board
(`gc-ts-probe.md`).
