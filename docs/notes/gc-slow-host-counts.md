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
