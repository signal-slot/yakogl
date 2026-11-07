<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 terrain: which record line confirms each ranked fix, Pi 4 and AM62P

Plan #329, item terrain-board-readout, for the board sessions
(`pi4-board-before`, `pi4-board-after`, `am62p-board-before`,
`am62p-board-after`, `pvr-mip-tail-e1-board`). Read at main `d254d172`
(2026-11-07); the printing code cited below is unchanged between
`434d2695` and that tip. Nothing here ran on a board; every "before" is the
figure the cited note gives, with the commit that note gives.

How to read a row:

- **Candidate**: the ranked fix and the note that ranks it
  (`pi4-slow-terrain.md` and `am62p-slow-terrain.md`, "Fix candidates,
  ranked", unless another note is named).
- **Record line**: the line or field of the console record, and the
  `file:line` of the code that prints it today. **not printed** means no
  code in the tree prints it (the host harness's `shim.c` prints some
  counters the image does not).
- **Before**: the last record's value and its commit. **Confirms**: the
  value that, read on the board, says the fix did what the note predicts.
- **parse.py** names the field of `tools/hostharness/terrain-records/parse.py`
  (item terrain-records-parse, on origin/main at `f652d6dc`) that carries the
  line, or **none** where it does not.

Records are the console log of a GM run (`tools/board.sh <board> gm`,
`tools/cts/board-stage.sh <board> common`) and, for the AM62P, of
`gl_test` cases 216 and 219 (`tools/cts/pr-session.sh`). Terrain's per-scene
ms is glmark2's own `[terrain] <options>: FPS: N FrameTime: X ms` line
(glmark2's code, not the tree's; read by `tools/cts/glmark2-ms.py:33` and
`parse.py:20`, fields `fps`, `ms`).

## Pi 4

Baseline of every row: main `c4af4917`, `records/pi4/20261022-0308-glmark2-c4af4917/`,
terrain 161.79 ms a frame (`tools/cts/RESULTS.md`, Common set;
`pi4-glmark2-cpu.md:532`).

**Read this before `pi4-board-before`.** `v3d-rt-evict` (`31a4cf8d`) is on
main: `RT_IMAGES` is 16 (`src/v3d/backend.rs:292`), so the render-image table
now holds terrain's 9 targets. `pi4-target-mark-clear` (candidate 1) is not
on main (`free_texture`, `src/gl/texture.rs:5009`, does not clear the entry).
A baseline built from the current tip may therefore already read the
"after" counters of the first two rows (5 chains a frame, `unknown` and
`first-sample` flat) and a frame near 30 ms. The `c4af4917` figures below are
the thrashing frame. If the baseline already reads the confirming values,
candidate 2 is the fix that did it (`v3d-terrain-copies.md`, design 2,
"Done"; its board value was not measured) and candidate 1 is then a
correctness fix (a stale wait for an unrelated kick), not a speed one.

### Lines the GM image prints

| line | printing code | parse.py |
|---|---|---|
| `glmark2-frame: [terrain] ...: N frames; us per frame: gl G, swap S, show H, other O, frame F` | `driver/samples/zephyr/glmark2/src/main.cpp:1208` (`print_cost`) | `frame_line`, `us_gl`, `us_swap`, `us_show`, `us_other`, `us_frame` |
| `; v3d last completed job per frame: prep P, bin B, render R, total T (end waited for in K of N) (jobs J over N frames, ...)` on the same line | `main.cpp:1219-1232` | `end_waited` (K of N) only; the rest is inside `frame_line` verbatim |
| `glmark2-steps: [terrain] ...: us per frame: GlProgram .., ...; sum X` | `main.cpp:1182-1196` | none |
| `glmark2-jobs: [terrain] ...: frame 10: J job(s): total T render R[~] idle I[ tfu K/Tus]; ...` | `main.cpp:1263-1272` (`print_jobs`), once a scene at frame 10 (`JOBS_LINE_FRAME`, `:191`) | none |
| `yakogl/v3d: TFU mipmap at frame N: C chains (Q queued behind a job, W waited), refused: ..., unknown U, ...` | `src/v3d/backend.rs:4918-4925` | `tfu_frame`, `tfu_chains`, `tfu_line` (the line verbatim holds `unknown`); the highest-frame line only |
| `yakogl/v3d: tiled copies for draws at frame N, by reason: plain .., first-sample .., ...` | `src/v3d/backend.rs:4927-4931` | `tiled_frame`, `tiled` (a dict of every reason), `tiled_line` |
| `yakogl/v3d: other copies at frame N: upload .., image-store .., level0-back .., chain-back .., ...` | `src/v3d/backend.rs:4932-4936` | none |

The three `yakogl/v3d:` lines are printed together only when an arena's
high-water mark grew in that frame (`src/v3d/backend.rs:4895`, `if hw !=
g.arenas_high_water`; frame 1 always prints). They are cumulative since boot
and not per scene. A per-terrain figure is the difference of two lines, the
last one before terrain's window and the last one after it, and a scene whose
arenas did not grow prints none. `parse.py` keeps the highest-frame line of
the whole console, so for terrain alone take its `tfu_line` and `tiled_line`
from the console by eye or compare two runs' lines; its `diff` table compares
two runs' cumulative values, not two scenes.

### Counters the image does not print

`v3d_counters` (jobs, waits; `src/v3d/backend.rs:6355`), `v3d_copy_counters`
(`:2269`), `v3d_mip_counters` (`:5034`), `v3d_rt_images` (`:4990`),
`v3d_tex_waits` (`:5080`) and `v3d_tiling_time` (`:5157`) are exported and
called by tests (`tests/v3d_terrain_jobs.rs:944`) and by the host harness
(`tools/hostharness/glmark2/shim.c:475`, `:650`), and by nothing in
`driver/samples/zephyr/glmark2/src/main.cpp` or the board glue. A per-scene
line that prints them is item `glmark2-scene-counters-design`'s design; until
it is built the rows that need them say **not printed**.

### The table

| candidate (note) | record line or field | before | confirms |
|---|---|---|---|
| **1. Clear a texture's target-use entry when the texture is freed** (`pi4-slow-terrain.md` 1; `v3d-terrain-copies.md` design 1; item `pi4-target-mark-clear`, in review) | `glmark2-frame` `frame` and `gl` us (`main.cpp:1208`) | frame 161.79 ms, gl 156.06 ms, swap 5.62 ms (`pi4-glmark2-cpu.md:532`) | frame about 30 to 31 ms (`v3d-terrain-gpu.md`, "Board values"); gl down by 120 ms or more (`v3d-frame-cpu-model.md` 1: less than that rejects the model). The gl and swap split is predicted differently by two notes: `pi4-slow-terrain.md` says gl under about 3 ms, `v3d-terrain-gpu.md` says gl about 20 and swap about 9 (9 waits a frame fall in the calls that end a job, as refract's 15.4 of 17.27 ms in the swap). The frame total agrees; judge by the total and by "gl down 120 ms or more" |
| same | `TFU mipmap ... C chains` (`backend.rs:4918`) and `unknown U` in its `refused:` list | 183 chains and 122 `unknown` over 61 frames: 3 and 2 a frame, between terrain's frame 1 line and shadow's frame 1 line (`pi4-slow-terrain.md`, section 2) | chains grow by 5 a terrain frame (`pi4-slow-terrain.md` 1; `v3d-terrain-gpu.md`: 5 chains, 2.64 ms of TFU a frame); `unknown` stops growing after terrain's frame 1 |
| same | `tiled copies for draws` `first-sample` (`backend.rs:4927`) | 183 over 61 frames, 3 a frame | +0 after terrain's frame 1 |
| same | `other copies` `image-store`, `level0-back`, `chain-back` (`backend.rs:4932`) | `image-store` 245 over 61 frames (4 a frame); `level0-back` and `chain-back` 0 | `image-store` 7 a frame (`v3d-frame-cpu-model.md` 1); `level0-back` and `chain-back` stay 0 (`v3d-frame-cpu-model.md` 5: growth means the board's eviction order differs from the host's) |
| same | `glmark2-jobs` `tfu K/Tus` of the entries (`main.cpp:1263`) | 4 TFU jobs at frame 10 (3 chains and 1 tiling), 1.8 ms; the per-job times are not in the tree (the record is gitignored) | 5 `tfu` entries, `tfu 1/810us` behind an 800x600 chain's job and `tfu 1/100us` behind a bloom chain's; 2.64 ms in all (`v3d-terrain-gpu.md`, per-job table) |
| same | `glmark2-frame` `(jobs J over N frames ...)` and `end waited for in K of N` (`main.cpp:1225-1230`) | 560 jobs over 62 frames (9.0 a frame); `end waited for in 62 of 62` | jobs 9.0 a frame unchanged (`v3d-terrain-gpu.md`); every swap still waits, the frame being GPU-bound |
| same | the GPU time: `glmark2-jobs` `render` of jobs 7 and 8 and of job 3 (`main.cpp:1263`) | 27.3 ms of render at frame 10 (29.1 less 1.8 of TFU); per job: not in the tree | jobs 7 + 8 about 17 ms of about 27, job 3 about 7 ms (`v3d-terrain-gpu.md`, "Board values"); if job 3 is the largest, binning or overdraw is the floor, not the samples |
| **2. Make the render-image table count and survive more than 8 targets** (`pi4-slow-terrain.md` 2; `v3d-terrain-copies.md` design 2; done as `31a4cf8d`, `RT_IMAGES` 16) | the eviction count the note asks for: **not printed** (no counter exists: no match for an eviction counter in `rt_register`, `src/v3d/backend.rs:1862`) | no value. The host showed 3 evictions a frame at 9 targets for 8 slots (`pi4-slow-terrain.md`, section 2) | the same lines as candidate 1: 5 chains a frame, `unknown` and `first-sample` flat after frame 1. Candidate 2 alone gives them if `pi4-target-mark-clear` is not on the image. |
| **3a. Make the CPU mipmap filter cheap on non-cacheable memory** (`pi4-slow-terrain.md` 3; `v3d-frame-cpu-model.md` 2; items `mip-filter-pin`, `mip-filter-wide`) | the filter's time and bytes: **not printed** (the item adds the counters; `texture.rs:4344-4427` has none today) | row A of `v3d-frame-cpu-model.md`: about 19 to 76 ms a frame in the thrashing frame (a model, no measurement) | with candidate 1 not applied, terrain's `gl` falls by that share. With candidate 1 applied the filter does not run in terrain and nothing moves (`pi4-slow-terrain.md` 3: expected 0). Confirm on the counter, not on terrain |
| **3b. `tfu_tile` one job a level; a TFU chain from a raster level 0** (`pi4-slow-terrain.md` 3b; `v3d-tfu-raster-chain.md`) | `v3d_tiling_time` ns (`backend.rs:5157`): **not printed**. `gl_test` case 140's chain part (`driver/tools/pvrlab/gl_test.c:8543`, `[140] = "tfuprobe"`): **not printed**, the part is not written | CPU tiling about 8 to 10 ms a frame in the thrashing frame (model, `pi4-slow-terrain.md`) | for terrain after candidate 1: `first-sample` and `unknown` flat after frame 1, so neither design pays on terrain (`v3d-tfu-raster-chain.md`, "Board record that decides"). Per design: case 140 chain part, every level `ok` or `max |diff| <= 1`, no `E`, no reset |
| **4. Do not drain the GPU before a CPU filter of a target** (`pi4-slow-terrain.md` 4; items `gl-settle-drain-study`, `gl-settle-wait-kick`) | the per-scene count of CPU waits: **not printed** (`v3d_counters` waits, `backend.rs:6355`). Nearest printed: `end waited for in K of N` (`main.cpp:1225`) | waits 9 a frame on the host and board (`v3d-wait-poll.md` 4); 62 of 62 | a few ms (the GPU work overlapped), 0 after candidate 1 (`pi4-slow-terrain.md` 4). Read it on the host first: `gl-settle-order-pin` |
| **5. How the CPU waits for a job** (`v3d-wait-poll.md`) | wait time per wait: **not printed** (item `v3d-wait-time` adds `wait_job` time beside `v3d_counters`, not merged). Printed today: `glmark2-jobs` `idle I` (`main.cpp:1264-1267`) | `idle`: not in the tree (record not kept). Gap bounds: gl + swap less GPU work, 156.06 + 5.62 - 29.1 = 132.6 ms in the thrashing frame | per `v3d-wait-poll.md` 6: `idle` of a job that followed another directly is 10 to 60 us; terrain's `gl` + `swap` less the GPU work (29.1 ms) is under 1 ms after the mark fix; wait time per wait under 10 us once `v3d-wait-time` prints it; above about 2 ms a frame, the estimate is wrong and register reads or the L2T clean (candidates 3, 3b there) are next |
| **6. After the fix: the blur passes sample one level; the dead window clear; TFU bytes** (`v3d-terrain-gpu.md` 1 to 3) | `glmark2-jobs` per job (`main.cpp:1263`); `jobs J over N` (`main.cpp:1230`) | job split: not in the tree (model only) | (3) the dead clear: jobs 9.0 to 8.0 a frame and job 0's `render` gone (0.5 to 0.8 ms); (1) blur passes: jobs 7 and 8 `render` down (up to about 9 ms), with the TFU 0.9 to 1.8 ms more; (2) chains skipped below level 0: `tfu` sum down 0.1 to 1.8 ms |
| *the 130 ms itself* (`pi4-slow-terrain.md`, "The board frame, split five ways") | the same `glmark2-frame` line and the arena lines of a run before any fix | gl 156.06 ms; arena lines: `unknown` +2, `first-sample` +3, `level0-back` 0, `chain-back` 0 a frame (`v3d-frame-cpu-model.md`, "What the board session must read") | the rows above, read from the unfixed image: the model holds if the arena lines show these rates; `v3d_tiling_time` (not printed) would split the CPU tiling from the filter |

Logging: the note's logging estimate (about 1.2 ms a frame averaged,
75 ms once in frame 1) has no line of its own. The back end's lines are
printed only on arena growth (`backend.rs:4895`); a steady terrain frame
prints none, so the three `yakogl/v3d:` lines of the table are also the
check that nothing prints per frame.

## AM62P

Baseline of every row unless it says otherwise: main `6a7021ec` (built as
`e4819b69`), 2026-11-01, `records/am62p/common-e4819b69/20261101-1935-GM-4bcd9ff3/`,
terrain 83.06 ms (13 FPS), shadow 10.62 ms, refract 35.09 ms
(`tools/cts/RESULTS.md`, Common set). The `gl_test` baseline is main
`61ad38a1` plus the kick count, `records/am62p/mipchains-42/`
(`RESULTS.md:296-306`).

### Lines the GM image and `gl_test` print

| line | printing code | parse.py |
|---|---|---|
| `[terrain] ...: FPS: N FrameTime: X ms` | glmark2 itself; read by `tools/cts/glmark2-ms.py:33` | `fps`, `ms` |
| `glmark2-frame: [terrain] ...: gl G, swap S, show H, other O, frame F` | `driver/samples/zephyr/glmark2/src/main.cpp:1208` | `frame_line`, `us_*` |
| `; pvr last completed kick per frame: submit .. (mmu .., syncs ..), kick->geom .. err .., kick->frag .., frag->frag .., wait W (n) (kicks K over N frames, frames with more than one M, lost L; bound B us)` | `main.cpp:1292-1315` (`print_kicks`); present only in a build with `CONFIG_YAKOGL_DRV_GPU_PVR_KICK_TIMES` (`driver/zephyr/Kconfig:194`; no `tools/cts` script sets it). Otherwise the same place prints `pvr kick times: absent (built without ...)` (`:1298`) | `kick_line`, verbatim, no fields |
| `render: fence N (frame fence) not moved at 20 ms, re-posted R kick(s) (M so far)` | `driver/rust/src/gpu/pvr/render.rs:2373-2380`, only for the first 8 re-posts and every 256th (`:2371`) | `reposts` (lines seen), `reposted_kicks`, `reposts_so_far` (the last "so far") |
| `gl_test: mipchains: 20 frames, X ms a frame (...)` | `driver/tools/pvrlab/gl_test.c:20179` | none (a `gl_test` record, not a GM record) |
| `gl_test:   per frame: TA busy A us, 3D busy B us, TA\|3D C us, wall W us` | `gl_test.c:20184` | none |
| `gl_test:   per frame: K kicks (n over 20 frames)` | `gl_test.c:20190-20193`; printed only when the completed fence moved | none |
| `gl_test: mipchains: reposts R over 20 frames` | `gl_test.c:20196`, PowerVR build only | none |
| `gl_test: kickpair: two\|one: 100 reps, X ms a rep, K kicks a rep, reposts R` | `gl_test.c:20303` | none |
| `gl_test: kickpair: two\|one: last kick, mean us over N reps: submit; post->geom; post->frag; prev frag->frag; wait` (`not built` without kick times) | `gl_test.c:20308-20326` | none |
| `dss stats <when> t=.. ms: vsyncs .. app_frames .. underflows .. sync_lost .. go_timeouts .. go_waits .. safety_flips ..` | `driver/zephyr/lib/board/common/sample_glue.c:221-224`, only in the PowerVR sample built with `CONFIG_YAKOGL_SAMPLE_SHOW_ON_DSS` (`:208`), at the start and end of its hold (`:279`, `:286`); the GM image does not print it | none |
| `eglSwapBuffers: the display held every window buffer; ...` and `... still holds every window buffer; ...` | `src/gl/egl.rs:2305`, `:2316` (each once) | none |
| `pipe fifo high H low L preload P` (debug level) | `driver/rust/src/soc/am62p/dss.rs:227` | none |

Two limits: a `gl_test` case 216 baseline at `6a7021ec` or a later main does
not exist (the record is `61ad38a1`'s, and the `reposts` and `kickpair` lines
postdate it), so those rows say "no record" and the board-before session
supplies the baseline. The notes' "reposts" figures of the GM record are
lines seen, rate-limited: for the true count read the "so far" number of the
last printed line (`parse.py` `reposts_so_far`), and treat a count above
8 as printed only every 256th.

### The table

| candidate (note) | record line or field | before | confirms |
|---|---|---|---|
| **1. The once-a-frame 20 ms fence** (`am62p-slow-terrain.md` 1; `pvr-waited-kick.md`, "The proposed fix"; `pvr-rering-knob-recipe`) | `render: fence N (frame fence) not moved at 20 ms, re-posted ...` (`render.rs:2373`); `parse.py` `reposts_so_far`. The call site of the wait is **not printed**: every backend wait is named "frame fence" (item `pvr-repost-trace` adds it) | about one a terrain frame at `ce3354ac` ("every frame now has one frame fence that does not move for 20 ms"); the count at `6a7021ec` is not in the record | the count (M so far) down to 0 over the frames run, and terrain's `FrameTime` below 83.06 ms: up to -19 ms (`am62p-slow-terrain.md`), 77 to 79 ms if the early re-post and the fine poll land (`pvr-waited-kick.md`, "Expected gain"); `pvr_render_wait_rering = true` should cut the stall to about 8 ms if a doorbell is enough |
| same | `gl_test: mipchains: reposts R over 20 frames` (`gl_test.c:20196`) | no record (the line postdates `61ad38a1`) | R about 20 means one 20 ms fence a frame in case 216 (the 46.15 ms is 40 kicks at about 0.65 ms plus one fence); R about 0 means 40 kicks at 1.15 ms each (`am62p-slow-terrain.md`, "What a mip-level kick pays"; `pvr-waited-kick.md`). With path B on, `reposts_early` about 50 and `reposts` 0 (`pvr-waited-kick.md`): `reposts_early` is **not printed**, the counter does not exist |
| **2. Fewer renders a chain: three levels a render** (`am62p-slow-terrain.md` 2; `pvr-mip-fewer-kicks.md`; design `pvr-mip-three-levels`, running) | `gl_test:   per frame: K kicks` (`gl_test.c:20191`) | 40.00 kicks a frame, `61ad38a1` | 21 kicks (4 + 4 + 4 + 4 chains' renders and 5 passes: `am62p-slow-terrain.md` 2) |
| same | `gl_test: mipchains: .. X ms a frame` (`gl_test.c:20179`) | 46.15 ms, 40 kicks, `records/am62p/mipchains-42/` | below 46.15 by the removed kicks; the note gives no ms for 216, only terrain's -25 ms (43 mip kicks to 20, at about 1.1 ms). The four quadrant probes stay `PASS` (`gl_test.c` `mipchains: top left` ... `:20211`) |
| same | terrain `FrameTime` and `pvr last completed kick per frame ... kicks K over N frames` (`main.cpp:1315`) | 83.06 ms; 52 kicks a frame on the host, 52.7 on the board (`ce3354ac`'s kick-times build: 5802 over 110 frames) | about 58 ms (-25), kicks about 29 a frame (52 less 23); picture probes against column `am62p` of `tools/cts/glmark2-pictures.txt` (28 match, 5 mismatch of animated probes at `6a7021ec`), 0 GL errors, `verdict=done`, 0 resets |
| **2'. E1, the tail route** (`pvr-mip-tail-e1.md` "(3) The board procedure"; item `pvr-mip-tail-e1-board`) | the same `gl_test` lines and terrain `FrameTime` (the note's `gl_test.c:20179-20196` and `:20303-20324` still hold them: ms `:20179`, TA/3D `:20184`, kicks `:20190-20193`, re-posts `:20196`, `kickpair` `:20303`) | `per frame: 40.00 kicks`, 46.15 ms (`61ad38a1`); terrain 83.06 ms | `per frame: 24.00 kicks` (3 chains of 9 to 5, 1 of 8 to 4, 5 passes); 216's ms a frame by at least 16 x c - 2 ms, about 27.8 ms plus the tail kicks' shading at c = 1.15 ms or 35.7 ms plus shading at c = 0.65 ms, c being the marginal cost of one queued render from `kickpair` (`two` less `one`, ms a rep, `gl_test.c:20303`); terrain about 60 ms (c 1.15) or about 70 ms (c 0.65); host 52 to 32 kicks a frame (board about 53 to 33). If the kicks fall and the ms do not, the frame is waits, not kicks |
| **3. Path B again, once a waited kick costs what a queued one does** (`am62p-slow-terrain.md` 3; `pvr-waited-kick.md`; `pvr-tail-cacheable`, running) | `gl_test` with the tail on (`Backend::mip_cpu_tail`): `per frame: K kicks`, `X ms a frame` (`gl_test.c:20179`, `:20191`). The per-prefix time (15.39 ms) and the tail's (7.87 ms) are **not printed**: they came from a 2026-10-29 measurement build | with path B on: 15 kicks, 82.95 ms, TA\|3D 7.00 ms (`records/am62p/mipchains-42/`, `RESULTS.md:301-306`); a prefix call 15.39 ms, its tail 7.87 ms (the measurement build); terrain with path B on: about 124 ms (`am62p-slow-terrain.md`: measured +40 ms) | ms a frame under 46.15 in case 216 (15 kicks); per-prefix time near 4.4 ms and the tail under 1 ms (`am62p-slow-terrain.md` 3), readable only in a measurement build; terrain about 35 ms under 83.06 (43 to 11 mip kicks) |
| **4. One submit for a chain (experiment first)** (`am62p-slow-terrain.md` 4; item `pvr-gltest-kick-probes`, merged) | `gl_test: kickpair: two` and `one`: `ms a rep`, `kicks a rep` 2.00 and 1.00 (`gl_test.c:20303`) and `last kick, mean us`: `post->geom`, `post->frag`, `prev frag->frag` (`gl_test.c:20315`) | no record. Reference: post to geometry fence 120 to 140 us and to fragment fence 220 to 230 us for a 64x64 kick on an idle GPU; about 1.09 ms post to geometry done when queued (`pvr-fence-wait.md`, via `am62p-slow-terrain.md`) | the second render of `two` posts its geometry fence much sooner than the first one's 1.09 ms (the take-up is per command, not per render): between 0 (a render's price) and -40 ms (`am62p-slow-terrain.md`). `(two - one)` is c. In the terrain GM record the same fields appear as `kick->geom` and `frag->frag` in `kick_line` |
| **5. The window's dead clear kick** (`am62p-slow-terrain.md` 5; `pvr-extra-kick.md`; item `pvr-dead-clear-kick`) | `pvr last completed kick per frame ... (kicks K over N frames ...)` (`main.cpp:1315`), only with kick times; terrain `FrameTime` | 52.7 kicks a frame (`ce3354ac`'s kick-times build); 83.06 ms | one fewer: 51.7 (host 52 to 51, `kicks.py`); `FrameTime` down by 1.1 to 1.8 ms (about 81.3 to 82.0); picture probes unchanged. No other line moves |
| **6. Each level's own render area** (`am62p-slow-terrain.md` 6; item `pvr-mip-level-area`) | `gl_test:   per frame: TA busy, 3D busy, TA\|3D` (`gl_test.c:20184`) and `ms a frame` | TA\|3D 7.75 ms a frame (path B off), 46.15 ms | TA\|3D down by about 3 ms (11,691 tiles x 0.26 us), then ms a frame down by 0 to 3 ms |
| **7. The 20 us poll** (`am62p-slow-terrain.md` 7; `pvr-fence-wait.md`; item `pvr-fine-wait-default`) | terrain `FrameTime`; `kick_line` field `wait W (n)` (`main.cpp:1309-1312`) | 83.08 to 81.67 ms with the poll (`ce3354ac`, measured); `wait` per kick: no value in the notes | 81.67 ms or lower at the build of the default; `wait` bounded within 20 us of the fence (`am62p-slow-terrain.md` 7) |
| **8. The transfer queue (path A)** (`am62p-slow-terrain.md` 8; `pvr-tq-mip-study.md`) | a transfer job's post to fence time: **not printed**, there is no transfer job in the tree | none | its time against `kick->frag` (`kick_line`); unknown until one blit is timed |
| *stale marks on the PowerVR path* (`pvr-stale-mark-waits.md`, "Board record value that confirms it") | terrain, shadow and refract `FrameTime` in a run from a main with `pi4-target-mark-clear` against one without | terrain 83.06 ms, shadow 10.62 ms, refract 35.09 ms | **no line moves**: all three within the run-to-run spread. A larger move contradicts the note and points at a wait it did not see. No Pi 4 counter exists on this board |

### AM62P display (`am62p-dss-readahead.md`, "The board check that confirms it")

The question is a torn bottom strip of a light scene, read through the camera
and the `dss stats` line. The GM image does not print `dss stats`; the
sample with a hold does (`sample_glue.c:221`), and a console from the GM run
has no such line. The comparison values below are for a run of the sample or a
build that prints it.

| check | record line | before | confirms "safe" |
|---|---|---|---|
| FIFO ran dry, sync lost | `dss stats` `underflows`, `sync_lost` (`sample_glue.c:221`) | 0 and 0 (`vsyncs 20008 app_frames 152650 underflows 0 sync_lost 0 go_timeouts 0`, `am62p-flip-latch.md:427`) | both 0; any other value is a finding of its own |
| GO never rode along | `go_timeouts`, `go_waits` | `go_timeouts` 0; `go_waits` 134955 against `app_frames - vsyncs` 132642, +1.7 percent (`am62p-flip-latch.md:427-429`) | `go_timeouts` 0 and `go_waits` within a few percent of `app_frames - vsyncs` |
| frame rate | `vsyncs` over the hold | 58.85 per second, the 16.99 ms frame (`am62p-dss-readahead.md:75`, `:146`) | 59 within 1 percent |
| no second writer | `safety_flips` | 0 | 0 |
| ring of window buffers | `eglSwapBuffers: the display held every window buffer` (`egl.rs:2305`), `still holds every window buffer` (`egl.rs:2316`) | not in the notes | the first at most once, the second 0 times |
| read-ahead depth | `DSS_VID_BUF_SIZE_STATUS` (`regs.rs` `vid::BUF_SIZE_STATUS`, offset 0x38) and `pipe fifo high/low/preload` (`dss.rs:227`, debug level; needs `CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP` or a debug read): **the register read is not printed**; the debug line is | computed 10 lines (BUFSIZE 0xA00, `high 2559 low 2552 preload 2552`) | the same values; a different BUFSIZE scales the window: lines = BUFSIZE x 16 / 4096 |
| torn frame | camera: `take_screenshot` (`reuse_detection=true`), 10 or more shots of glmark2 `[build] use-vbo=true` and `[texture] texture-filter=nearest`, the bottom 10 to 13 lines first | no record | no discontinuity in the bottom strip with the counters at 0. A discontinuity there with the counters at 0 is "hazard": the defect "AM62P: DSS reads the old buffer after the latch, torn bottom lines in light scenes" (`am62p-dss-readahead.md`) |

## What parse.py covers, and what it does not

Covers (all of terrain's console): commit, verdict line, records path,
`FrameTime`/FPS, `glmark2-frame` us fields and `end waited for`, the highest
Pi 4 `TFU mipmap` chain count and `tiled copies` reasons, the AM62P re-post
line count and "so far", and the `pvr last completed kick` line verbatim.

Does not cover: `unknown` and the other refusals (only inside `tfu_line`),
`other copies` (`image-store`, `level0-back`, `chain-back`), `glmark2-jobs`,
`glmark2-steps`, the kick line's numbers (`kick->geom`, `kick->frag`, `wait`,
`kicks K over N`), every `gl_test` line, `dss stats` and the `egl.rs`
lines. A session that needs those reads the console by eye, or extends
`parse.py` (outside this note).

## Discrepancies found while collecting the rows

- Candidate 1's `gl` after the fix: `pi4-slow-terrain.md` says under about
  3 ms, `v3d-terrain-gpu.md` says about 20 (swap about 9). The two agree on
  the frame total (30 to 31 ms). The first ignores the 9 waits a frame that
  a GPU-bound frame makes inside the GL calls; the second counts them. Read
  the total and the 120 ms drop, not the split.
- `pi4-slow-terrain.md` says "All of these lines are already printed". The
  three `yakogl/v3d:` lines are printed only on arena growth, so a scene
  that grew no arena has no line of its own (see above); `v3d-frame-cpu-model.md`
  and `v3d-terrain-copies.md` take their rates from lines at terrain's frame 1
  and shadow's.
- `pvr-mip-tail-e1.md` lists `reposts` in `gl_test` case 216 as "(new)". The
  line is in the tree now (`gl_test.c:20196`), but no board record has it.
- `pvr-mip-tail-e1.md` quotes `gl_test.c:20179-20196` and `:20303-20324`:
  they are still the lines of the kick count, the ms, the TA/3D counters,
  the re-posts and the `kickpair` rows.
- The Pi 4 baseline at the current tip may already show candidate 2's
  effect (above).
