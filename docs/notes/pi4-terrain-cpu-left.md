<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: where the ~9.6 ms that is not GPU work goes (host study)

Plan #329, item `pi4-terrain-cpu-left` (#1209). Host only, no board, no code
change. Measured at main `975a6792` (the integration tip the item started
from; `git fetch origin` is not possible here). glmark2 is `22c527cb`
(`$CONNECTED_DEPS_DIR/glmark2-src`). Line numbers are that tree's.

## Answer in short

1. **The host cannot show 9.6 ms of CPU work, because there is about 1 ms of
   it.** On the V3D stand-in one terrain frame is 1.06 ms wall, 0.84 ms of it
   in GL/EGL calls, 8 jobs, 8 waits that cost 0.05 to 0.2 us each (the
   stand-in's jobs are done at submit). Mapped to the board (below) that is
   **about 1 to 2.5 ms of CPU work**, plus up to 1.8 ms of non-cacheable
   tile-state zeroing. The board's frame is 35.7 ms, GPU work 26.1 ms. So the
   CPU is not the main part of the 9.6 ms; **the CPU is blocked in waits for
   about 32 ms of the 35.7 ms**.
2. **The last job's `prep` of 2.56 ms is queue time, not CPU work.** The
   OSAL stamps `prep` from the job's *submit* to its bin start, and a job
   submitted behind a running one keeps that stamp (`src/osal/zephyr_v3d.c:1411`
   `s.pend_c0 = c0`, `:1477` `start_job(..., s.pend_c0)`, `:1433-1434`
   `last_job_cyc.prep = c_bin - c0`). What is CPU work in `prep` is one
   `memset` of the job's tile state (`:1431`): 130 tiles x 256 B = 33,280 B
   for an 800x600 job, 12 us at the best rate in the tree (R1, 0.36 ns/B),
   0.33 ms at the pessimistic one (R5, 10 ns/B), `v3d-frame-cpu-model.md`.
   So at least 2.2 ms of the 2.56 is the previous job still running. It is
   the same before and after the render-image fix for that reason.
3. **Where the 9.6 ms most likely is (a budget, not a measurement):** GPU
   work that the 26.1 ms leaves out (binning, cache clean, start-up of each
   job), GPU idle between jobs, and the CPU's recording time that is exposed
   at the jobs' edges. The 26.1 ms is `render` (23.49) plus TFU (2.64). The
   OSAL runs one job at a time, **bin first and then render**
   (`zephyr_v3d.c:1391-1414`: "the binner and the tile memory belong to the
   job running"), so binning is serial with rendering and is in no printed
   figure but `total`. Terrain's job 3 bins 393,216 vertices (131,072
   triangles) through the coordinate shader (`v3d-terrain-gpu.md:194`, `:232`).
4. **Ranked fixes** and the board value that confirms each are at the end.
   The first board read is one number: `wait.ns` a frame in
   `glmark2-counters`.

## Commands run

All outputs under `$TMPDIR`; nothing in the tree.

```sh
env PI4T_RUNS=prof-tfu1 bash tools/hostharness/pi4-terrain/run.sh \
    "$CONNECTED_DEPS_DIR/glmark2-src" "$TMPDIR/p4"
env PI4T_RUNS=perf-tfu1 bash tools/hostharness/pi4-terrain/run.sh \
    "$CONNECTED_DEPS_DIR/glmark2-src" "$TMPDIR/p4"
```

`run.sh` is terrain alone, 20 frames, `GM2_PROFILE=1` (and `GM2_PERF=1` for the
second), `YAKOGL_V3D_TFU=1`; `summarize.py` reads frames 2 to 20. **`GM2_PERF`
gave no result**: this host's `perf_event_paranoid` refuses it (the same as
`v3d-draw-fixed-cost.md`, "Method"), `summarize.py` reports "no gm2: gl-frames
line", so the split below is from `GM2_PROFILE`. The terrain-after-two-desktop
-scenes case is `multiscene.sh`, which `pi4-terrain-host-after.md` ran; it
gives the same 8 jobs, 5 chains, 0 refusals, so it was not repeated.

The waits by call site needed a scratch tree: a detached worktree at the same
commit under `$TMPDIR/wt`, with `#[track_caller]` on `wait_in_flight`,
`wait_queued` and `wait_job` and one `glog!("WAITSITE {line} {kick} {ns}")`
in `wait_job`. The same `run.sh` was run from it
(`$TMPDIR/p4w`). That run's own timings are inflated by the log lines (1.45 ms
a frame instead of 1.04) and are not used; only the call sites and the
nanoseconds per wait are. It was not committed.

## One frame on the host, by step

Frames 2 to 20, `prof-tfu1`. The frame is 8 jobs, one draw each, 5 TFU chains
(modelled on the CPU: 51.3 ms a frame of model time, which is GPU time and not
counted here).

| step | host us a frame | where (`file:line`) |
|---|---:|---|
| frame, wall | 1062 | harness |
| in GL/EGL calls | 845 | |
| `glDrawArrays` (8) | 518 | below |
| `glGenerateMipmap` (5) | 189 | below |
| `eglSwapBuffers` (1) | 35 | below |
| `glBindFramebuffer` (8), `glClear` (8) | 25, 19 | state, the pending-clear park |
| the other 13 entry points | 59 | `glBindTexture` 10, `glUseProgram` 10, `glBindBuffer` 9, ... |
| outside the GL calls (glmark2's own matrices and loop) | 217 | not in the board's `gl` |

**Draws, 518 us (65 us a draw).** The timed draw steps are 96 us of it
(`GlProgram` 31.5, `GlUniforms` 28.2, `GlTextures` 16.1, `GlAttribs` 13.3,
`GlState` 7.2); the board's `glmark2-steps` sum is 111 us at `17e91019`, so
that code runs at 1.15x the host's speed on the board, **including its stores
into the non-cacheable arenas**. The other ~420 us is the job's open, first
draw and submit: `v3d_job_draw` 12.6 us (`emit_state` 6.5, the shader and
attribute records 3.2, three `write_uniforms` 2.1), `texture_units` for the
fragment stage 5.3 us, the checks and `begin_job` 8.4, `check_program` 1.6,
and `flush_job` 15.9 us when a draw closes a job
(`v3d-draw-fixed-cost.md`, terrain column, 32.3 us a draw plus the submit; 8 x
48 = 0.39 ms, which agrees with 0.42).

**Job building, uniform and CL writes.** BCL 1,014 B, RCL 3,069 B, indirect
5,357 B, texture states 584 B, data 0, so 10,024 B a frame in 9 jobs
(`v3d-frame-cpu-model.md` row E), flushed at `submit_job`
(`src/v3d/backend.rs:3347-3353`), which on the board does nothing: the
carve-out is non-cacheable (`src/osal/zephyr_v3d.c:551-555`, `in_nc`). At R1
that is 4 us, at R5 0.1 ms. Not a candidate.

**`glGenerateMipmap`, 189 us (38 us a chain).** The TFU chain's setup
(`rt_chain` queueing, `backend.rs:2488-2496`: queued behind the job in flight
with `after: g.in_flight.fence`), the job's flush (`flush_job`,
`backend.rs:2935`, which `wait_queued`s the older job at `:3018` and submits at
`:3019`), and the model's TFU bookkeeping. No tiled copy, no CPU filter (0
`unknown`, 0 `first-sample` after frame 1), so none of the loops of
`v3d-frame-cpu-model.md` rows A and B run. The host's TFU model finishes the
chain on the CPU at the wait, which is why the stand-in counts it as 5 waits
in this entry point.

**Waits, with call sites** (the scratch run, 19 steady frames, 8 a frame):

| call site | waits a frame | host ns a wait | board meaning |
|---|---:|---:|---|
| `flush_job` -> `wait_queued`, `backend.rs:3018` (job N waits for job N-2 before it is submitted) | 5.9 (5 in `glGenerateMipmap`, ~1 at the swap) | 169 | **blocks for the GPU on the board** |
| `v3d_finish`, `backend.rs:6990` (from `eglSwapBuffers`, `src/gl/egl.rs:2413`, and `glFinish`) | 2.1 | 45 | **does not exist on the board's frame** (below) |

The stand-in's windowed surface is not double-buffered, so the swap takes the
`complete` path (`backend::finish`, `egl.rs:2413`) and waits for the last
job. The board's is EGL double buffered (`pi4-slow-terrain.md`, the record's
`swap` column), where the swap takes `finish_previous`
(`backend.rs:7032`, `wait_queued` only: the older of the two jobs). **The
host's "2.84 waits in `eglSwapBuffers`" is two waits more than the board's
swap makes.** That does not change the CPU work (45 ns each) but it shows
that the host's swap time is not a model of the board's `swap` 8.3 ms, which
is a wait for jobs 6 or 7 (GPU time).

No wait in `glDrawArrays`: the `copy_image_for` wait (`backend.rs:5297`) and
`settle_texture`'s drains never fire in this frame; the render-image table
(16 slots) holds every target. `choose_arenas` (`backend.rs:3109-3110`) did
not wait either: the second arena set was free at every job.

**Logging.** None in the steady frame: the log of the run has no line between
frame 1's lines and the end, apart from the scratch counter. The board's
record agrees (terrain printed nothing between its two arena lines,
`pi4-slow-terrain.md:455-459`). `dump_small_fs_block` is not reached.

**Swap path.** `eglSwapBuffers` is 35 us on the host: `backend::flush`
(submits job 8), `finish_previous`/`finish`, the buffer hand-over. On the
board `show` is a copy only when EGL does not show the window
(`board_show`, `driver/zephyr/lib/board/rpi4/board_rpi4.c:317-320`); the
record's `glmark2-frame` has gl 27374 + swap 8283 = 35657 of 35659 us, so
`other` is 2 us and `show` is inside `swap` or zero.

**TFU setup.** Per chain: queue behind the job (`backend.rs:2488`), then on
the board the OSAL runs it in `finish_inflight` after the job's render and
cache clean (`zephyr_v3d.c:1311`, `tfu_queue_run`), serially, polling the TFU.
That is the 2.64 ms of TFU (806, 114, 114, 804, 804 us) in the 26.1 ms. The
CPU's own part is the 38 us a chain above. On the host the model's 51 ms a
frame is a CPU loop and is not comparable.

## Mapping host time to board time

The earlier notes' ratio method: shadow's two CPU passes took 11.2 ms on the
host and about 162 ms on the board, 15x, for loops of 4-byte non-cacheable
loads (`pi4-slow-terrain.md:280-302`). That ratio is for **reads** of
non-cacheable memory. It does not apply to this frame:

- terrain's steady frame reads no non-cacheable memory on the CPU: no
  tiled copy, no CPU filter, no `rt_*_back` (rows A, B, F of
  `v3d-frame-cpu-model.md` are 0). It only **writes** the lists, uniforms
  and tile state, and a write-combined store is cheap.
- the board's own `glmark2-steps` sum, 111 us against the host's 96 us, is the
  check: the one place where this frame's CPU code is timed on the board is 1.15x
  the host, not 15x.

So the ratio used here is **1.2x (the steps' own) to 3x (an A72 at 1.5 GHz
against an x86 core, the generic bound)** on the host's 0.845 ms:

| part | board ms a frame | basis |
|---|---:|---|
| GL/EGL call CPU time | 1.0 to 2.5 | 0.845 x 1.2 to 3 |
| tile-state `memset`, 8 jobs, 182,784 B (at the OSAL, **with the GPU idle**) | 0.07 to 1.8 | R1 to R5, row D |
| list and record writes, 10,024 B | 0.004 to 0.1 | row E |
| waits' own cost (poll, clean, start of the waiting job) | 0.1 to 0.6 | `v3d-wait-poll.md:160`, 9 waits x 10 to 60 us |
| **CPU work and wait overhead, total** | **1.2 to 5** | |

What does not carry over, and the direction of the error:

- **Cacheable host memory.** The arenas are `malloc`ed on the host and
  non-cacheable on the board. Stores are write-combined; the 1.15x of the
  steps shows it is small for the draw path. Loads from the arenas (none in
  this frame) would be the 15x case. The `memset` is the one NC-write loop
  large enough to matter (up to 1.8 ms at R5), and it is a **store** loop, so
  the likely figure is the low end.
- **Jobs complete at submit on the host** (`src/osal/linux.c`), so the host
  has no queueing, no bin, no clean, no TFU time, no `idle`, no `prep` wait.
  Everything that is GPU or OSAL time on the board is zero here.
- **The host's swap path** is `finish`, the board's `finish_previous` (above).
- **Host noise.** `v3d-draw-fixed-cost.md` saw 1.7 to 2.2x between loaded and
  quiet runs of the same step. This run was one run; the 0.845 ms is a
  single figure, not a minimum of several.

## What the 35.7 ms is, as a budget

Board record `17e91019` (`pi4-slow-terrain.md:426-449`): frame 35.659 ms
(gl 27.374, swap 8.283), 9.0 jobs a frame at the record (8 after `4255adf1`,
the dead window clear; the GPU part below is frame 10 of the same record),
render 23.49, TFU 2.64.

| part | ms | state |
|---|---:|---|
| `render`, 8 jobs | 23.49 | measured (`glmark2-jobs`, frame 10) |
| TFU, 5 chains | 2.64 | measured |
| **GPU work counted so far** | **26.13** | |
| CPU work and wait overhead (above) | 1.2 to 5 | host + ratio |
| binning of 8 jobs, serial before each render | 1 to 5, of which job 3 most | estimate; **not measured**, no record has it |
| GPU idle: a job starts only when the CPU polls | up to ~3 | estimate; `idle` is in the record but not read |
| remainder | whatever of 9.6 is left | |

Why a job waits for the CPU. FRDONE has no interrupt (`v3d-wait-poll.md:71`):
the OSAL learns that job N-1 is over only when the CPU is in a wait,
`completed()`, or a submit (`zephyr_v3d.c:1482-1488`, `z_completed`). Job N is
the one pending behind it, and `start_pending` (`:1466`) starts it from there.
In terrain every job is one draw, so job N+1 is submitted after the CPU has
recorded it (0.1 to 0.4 ms a job on the board); job N-1's end is noticed at
the `wait_queued` of that flush (`backend.rs:3018`). For a long job (3, 6, 7,
8: 130 tiles, 7 to 9 ms) that costs nothing, the CPU is waiting before the
GPU is done. For a short one (1, 2, 4, 5: 16 tiles, well under 1 ms of
render) the GPU is done before the CPU has recorded the next job, and it
sits idle until the CPU gets to a wait. The 8 short-or-boundary exposures
times the recording time is the "up to ~3 ms".

The bound on the unexplained part: the CPU is in waits for
`35.66 - (1.2 to 5) = 31 to 34 ms` of the frame, and the GPU is busy for
26.1 ms plus its binning. A wait that is not the GPU's work is the OSAL's
own: the cache clean, the TFU runs, the start of the next job (`:1302-1316`).
All of the 9.6 ms is in those three: GPU work the counters omit, the
OSAL's per-job edge, and the exposed recording. None of them is a loop over
non-cacheable memory.

## Fix candidates, ranked

Each with an expected gain and the board record value that confirms it. All
are from the budget above, so the first board read decides which are real.

0. **Read before changing anything: `wait.ns` a frame.** In
   `glmark2-counters: terrain` (`main.cpp:277-278`, `wait.ns` over `frames`,
   items `v3d-wait-time` and `glmark2-counters-print`). Prediction: **31 to
   34 ms**. Then CPU work is 1.7 to 4.7 ms and the 9.6 ms is GPU or OSAL
   time. **If it is under 28 ms**, the CPU work is over 7 ms and this note's
   budget is wrong: go to rows D (`memset`, 1.8 ms at R5) and the call time
   (`glmark2-frame` `gl` less `wait.ns`) first. `wait.untimed` must be 0.
1. **Overlap job N+1's bin with job N's render** (OSAL: the binner is free
   while the renderer runs; `zephyr_v3d.c:1391-1414` holds the next job until
   the first is over, because tile memory and the binner pool are single).
   Expected 1 to 4 ms (the neighbour's render hides each bin), against a
   cost of a second tile-state and binner pool. Not CPU time, but the
   largest GPU-side item the counters omit. **Confirm:** per job
   `total - render - tfu` in `glmark2-jobs` frame 10 summed over the 8
   jobs: 1 to 5 ms if bin is the item. Better, print the ring's `bin` and
   `prep` per job (`osal_v3d_recent_jobs_us` carries both,
   `zephyr_v3d.c:752`; `print_jobs`, `main.cpp:1402`, prints only `total`,
   `render`, `idle`, `tfu`). Job 3's `bin` above 1 ms confirms.
2. **Start the pending job without the CPU being in a wait**: poll
   `completed()` (`z_completed`, `zephyr_v3d.c:1482`) at the start of
   `glBindFramebuffer`, `glClear` and `glDrawArrays` (8 calls of each a frame; a register read
   each, not measured), so a finished job N-1 starts N within a
   record, not after it. Expected 0.5 to 1.5 ms (the exposed recording of
   the short jobs). **Confirm:** the sum of `idle` over the 8 jobs of
   `glmark2-jobs` frame 10 (`main.cpp:1414`): the gain is at most that sum
   less the 10 to 60 us a job the OSAL's edge costs anyway (0.1 to 0.5 ms). A
   sum over 1.5 ms puts this above candidate 1's expected value.
3. **Skip the tile-state `memset` of jobs whose state is already zero**
   (`zephyr_v3d.c:1431`; Mesa zeroes per job, `backend.rs:3327-3329`). With
   the GPU idle at that point it is exposed: 182,784 B a frame. Expected
   0.07 ms at R1 to 1.8 ms at R5. **Confirm:** `prep` of a job that
   started with the GPU idle (the frame's first job, whose submit finds no
   job running) in the ring above; under 0.1 ms a job closes the question,
   0.2 ms or more (33 KB at 6 ns/B or worse) means the rate is R2 to R5. The 2.56 ms
   already recorded is not this: see answer 2.
4. **Shorten the CPU's recording** (`texture_units` FS 5.3 us, `begin_job`
   and `emit_state` 6.5 us a job, `check_program` 1.6 us, per draw): all of
   it on the host is 0.42 ms of the 0.52 ms the draws take, 0.5 to 1.3 ms on the
   board. Only the part that is exposed counts (candidate 2's window), so the
   expected gain is **under 0.3 ms**. **Confirm:** `glmark2-frame` `gl` less
   `wait.ns` a frame (from candidate 0) falling by that much after a change;
   unchanged `idle`.
5. **Not candidates here, with the figure that rules them out:**
   waits' poll granularity (1 us a poll, 0.01 ms a frame at most,
   `v3d-wait-poll.md:171`), logging (0 lines a frame), cache flush at submit
   (no-op on the carve-out), CPU filter or tiling (0 after the mark fix),
   `rt_*_back` (`level0-back` and `chain-back` flat in the record). The
   GPU-side items of `v3d-terrain-gpu.md` (blur passes sampling one level,
   skipping chains, depth store) are about the 26.1 ms, not this part.

## Limits

- The 9.6 ms is not split by a measurement. The host proves the CPU part is
  small (1 ms) and shows which waits exist and where; the bin, `idle` and
  recording shares are estimates until the board's `wait.ns` and the
  per-job `bin`, `prep` and `idle` are read.
- One host run for the profile, no repeat; `GM2_PERF` could not run.
- The per-job record of `glmark2-jobs` at `17e91019` is gitignored
  (`v3d-terrain-gpu.md:50-54`); the `idle` sum and `total - render - tfu`
  are not known today. The first record from an image with the counters
  line gives them for free.
- The board has 8 jobs a frame only from `4255adf1`; the record `17e91019`
  predates it (9 jobs, one of them the dead window clear). The `pi4-board-after2`
  record (item running) is the first with 8.
