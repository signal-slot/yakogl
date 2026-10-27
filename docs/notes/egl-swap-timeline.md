<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The eglSwapBuffers timeline, and the glmark2 score drops

2026-10-17. Plan #150 item `swap-drop-cause`. The test is
`tests/egl_swap_timeline.rs`.

## The question

The AM62P glmark2 score fell from 462 (`fb9107af`) to 289 (`1d54a4eb`,
again at `c854e0ae`). The Pi 4 score fell from 188 to 157 (`c854e0ae`).
Both drops lie across `0247ac66` (#925: a window swap completes its
frame). After that, `1eb9783d` (the EGL double buffer) landed. It is meant
to bring the CPU/GPU overlap back. Is there a wait in the new swap that
the old one did not have?

## Which swap the scores were measured with

| run | tree | #925 | EGL double buffer | glmark2 |
|---|---|---|---|---|
| AM62P | `fb9107af` | no | no | 462 |
| AM62P | `1d54a4eb` (`dd48294b`) | yes | no | 289 |
| AM62P | `c854e0ae` (`f886a2b7`) | yes | no | 289 |
| Pi 4 | before `0247ac66` | no | no | 188 |
| Pi 4 | `c854e0ae` (`c6c6b673`) | yes | no | 157 |

`git merge-base --is-ancestor 1eb9783d c854e0ae` is false: **no glmark2
score has been measured under the EGL double buffer yet.** Both low scores
were measured with the #925 swap alone.

## The timelines

The test records every swap with a fake GPU (kicks complete only when the
library waits for them) and a fake display. The boards are modelled from
their code:

| board | show (`present` -> `board_show`) | EGL vertical sync (`wait_vblank`) |
|---|---|---|
| AM62P | DSS plane flip by address, latched at the next vsync, no wait (`soc/am62p/dss.rs` `set_app_buffer`) | none (weak `yakogl_zephyr_wait_vblank` in `src/osal/zephyr.c`) |
| Pi 4 (GM image) | copy into the firmware framebuffer (no `SCANOUT_VIEW`), no wait | none (`src/osal/zephyr_v3d.c` has no `.wait_vblank`) |
| i.MX8MP | waits for the LCDIF latch (`SCREEN_BUFFERS` 1) | none yet (`zephyr_gc.c`, #737) |

Frame N's swap, per window kind (N-1 is the frame before):

1. **Before #925** (the 462 and the 188; today `GLES_NATIVE_WINDOW_PIPELINED`):
   submit N, wait for N-1, return. The application shows the window
   itself. The CPU records N+1 while the GPU draws N. The window that is
   shown is the one the GPU is drawing (the i.MX8MP's black band).
2. **#925, in place** (the 289 and the 157; today `EGL_SINGLE_BUFFER`):
   submit N, **wait for N's end**, return, the application shows N. The
   CPU work of N+1 starts on an idle GPU. A frame costs CPU + GPU, not
   max(CPU, GPU).
3. **EGL double buffer** (`EGL_BACK_BUFFER`, what glmark2 gets today):
   submit N, wait for N-1, show N-1's buffer, bind the third buffer,
   return. The CPU records N+1 while the GPU draws N. N+1 goes into
   neither N's buffer nor N-1's.

The test pins all three. For timeline 3 on the AM62P and the Pi 4 it
checks, frame by frame:

- the swap never waits for its own frame's end;
- frame N+1's CPU work starts with exactly frame N on the GPU;
- the swap shows N-1's buffer, and that buffer is complete;
- frame N+1 is drawn into neither N's buffer nor N-1's;
- the swap waits at most one vertical period (none on these two boards).

Timeline 3 waits the same as timeline 1. The only addition is the show,
which was in the application before (glmark2's `flip()`, now skipped for a
`BACK` window). Neither board's show waits. **The host finds no extra wait
in the double-buffered swap on the AM62P or the Pi 4.**

## Cause

The drops are the overlap that #925 removed: every swap waited for its
frame's end (timeline 2). (The Pi 4's "end waited for in N of N" was read
as this wait. It is not: the counter stays N of N under the double buffer
for another reason, see "Why the Pi 4 still reads 'end waited for in N of
N'" below.) The EGL double buffer gives the overlap back without showing a buffer the
GPU is drawing. So the scores should come back close to 462 and 188.
This is still a host result. The board A/B below confirms it.

## Two defects found and fixed

The test found two defects in the double-buffered swap. Neither changes
glmark2's score.

1. **A swap with nothing drawn showed a running buffer.** "Wait for all
   but the last kick" assumes that the frame just swapped has a kick. When
   nothing was drawn since the last swap, the last kick is the previous
   frame's, so the swap showed it while the GPU still wrote it. The swap
   now remembers `kick_id()` after each swap (`EglSurface::frame_kick`).
   If the next flush leaves it unchanged, the swap waits for everything
   (`finish`). glmark2 clears every frame, so it never takes this path.
2. **A show that waits plus swap interval 1 waited two periods.** The
   i.MX8MP's show waits for the LCDIF latch. Once its OSAL gets
   `.wait_vblank` (#737), the interval loop would wait a second period:
   30 fps on a 60 Hz panel. `present` may now return 1, "shown, and I
   waited for a vertical period" (`gles_osal.h`). The swap counts that
   period as the first of its interval. 0, which every board returns
   today, behaves as before. The i.MX8MP board layer must return 1 from
   `yakogl_zephyr_present` when its show waited. That is the
   `imx8mp-wait-vblank` item's change, not this one.

## Findings not fixed here

- **AM62P: three buffers are not enough above the panel rate.** The DSS
  flip does not wait. The buffer drawn next was shown two presents ago.
  If fewer than one vsync passes in two swaps (glmark2 at 462 fps on a
  60 Hz panel), that buffer can still be on the screen, so the panel may
  scan a buffer the GPU is drawing. The pixels the run reads back are
  right; only the panel picture can tear, as it could before. A fix needs
  the display to say which buffer it scans (a fourth buffer, or `present`
  reporting the latched address). That is a design change for the
  owner.
- **Pi 4: `SCANOUT_VIEW` with a `BACK` window is refused.** `bufs[0]`
  would be the framebuffer itself, and the frame drawn into it would be
  on the screen while it is drawn. The board sets
  `GLES_NATIVE_WINDOW_SCANOUT` on a view (the i.MX8MP view too), and
  `eglCreateWindowSurface` answers `EGL_BAD_MATCH` for it under
  `EGL_BACK_BUFFER`. `EGL_SINGLE_BUFFER` draws the view in place. The flag
  says it, not the stride: a view as wide as the screen has a packed
  stride. Slint (the SV image) asks for `EGL_SINGLE_BUFFER`.
- **The Pi 4 show is a copy.** Each swap copies 800x600x4 bytes into the
  framebuffer. Before, the copy ran after the swap; now it runs inside
  it, after the wait for N-1, while the GPU draws N. The cost is the
  same; glmark2's "show" column moves into "swap".

## Ranked hypotheses if a board does not come back

1. **Most likely, the scores come back** (timeline 3 = timeline 1 plus a
   non-waiting show).
2. **The swap is not double-buffered on the board.** For example, the two
   buffers did not fit in `GLES_HEAP_DATA`, so the window stays in place
   (`eglSwapBuffers: no memory for the window's buffer`). Check the
   console for `glmark2: the window is double-buffered by EGL`.
3. **The show costs more than before.** Look at the swap column of
   `glmark2-frame`: on the Pi 4 the copy is now from the V3D carve-out
   alias instead of the image's window buffer.
4. **Memory pressure from the two extra 1.92 MB buffers** (and, on the
   PowerVR, a depth buffer for each colour buffer). Refract on the Pi 4
   peaked at 69 of 96 MiB. Compare the `glmark2-mem` lines.

## The board A/B that decides

One board, the GM image from current main, per-scene ms/frame
(`common-tally.py`), three arms:

| arm | window | expected |
|---|---|---|
| a | stock (`EGL_BACK_BUFFER`) | close to c |
| b | `EGL_SINGLE_BUFFER` (timeline 2) | close to 289 / 157 |
| c | `GLES_NATIVE_WINDOW_PIPELINED` (timeline 1) | close to 462 / 188 |

If a is close to c, the cause is confirmed and fixed. If a is close to b,
check hypothesis 2. If a lies between them, compare the swap columns
(hypothesis 3). The `am62p-db-baseline` and `pi4-db-baseline` items run
these arms.

## Why the Pi 4 still reads "end waited for in N of N"

2026-10-18. Plan #150 item `pi4-end-wait`. The test is
`tests/egl_swap_end_wait.rs`.

Under the EGL double buffer the swap waits only for the frame before. Yet
the Pi 4's `glmark2-frame:` line at `33a0f264` and `499b946e` stays at
"end waited for in N of N" in terrain (56), shadow (58), refract (52),
desktop blur (768) and desktop shadow (1470).

### What the counter counts

After each `eglSwapBuffers` the glmark2 sample (`GLStateTimed::swap` in
`driver/samples/zephyr/glmark2/src/main.cpp`) reads the last job that
completed (`osal_v3d_last_completed_job_us`). It counts the frame when
that job's `render_late` is 0: the OSAL found the render still running and
waited for its end (`finish_inflight` in `src/osal/zephyr_v3d.c`). The
counter does not say which call waited, nor for which frame. Any wait of
the CPU for a running job counts, wherever it happens. It is not a count
of swaps that waited for their own frame.

### Where the V3D waits

From `src/v3d/backend.rs` and `src/gl/texture.rs`:

1. One job in flight. A submit first waits for the job in flight
   (`flush_job` -> `wait_in_flight`). A draw into a new target submits the
   open job, so it waits for the job before that one.
2. The depth resolve. `resolve_depth` flushes and finishes the depth pass
   before `v3d_depth_resolve` detiles it on the CPU.
3. `glGenerateMipmap` on the TFU. `v3d_generate_mipmap` submits the job
   that renders into the target and waits for it. The CPU fallback waits
   too (`settle_texture`).
4. The swap. `backend::flush` submits the window's job. `v3d_finish_previous`
   then waits for the older of the two outstanding jobs, so the frame the
   swap shows is complete (see "The queue of two jobs" below for the
   rules once two jobs are outstanding).

A render target sampled from its render image costs no wait
(`tex_state_for`).

### The host repro

`tests/egl_swap_end_wait.rs` runs the real GL core and the real
`eglSwapBuffers` (a `EGL_BACK_BUFFER` window, swap interval 0, a show that
does not wait, as the Pi 4 GM image). The back end is a stand-in that
waits where the V3D waits (rules 1 to 4) and has a clock. Each job has a
GPU time and runs after the job before it. A wait for a job whose end is
still ahead of the CPU is "waited for" (`render_late` 0). The sample's
counter is computed the same way. Model times: a pass 3 ms on the GPU, the
window pass 2 ms, the window's clear 0.2 ms, the depth resolve's detile
6 ms of CPU, a TFU job 1 ms. 8 frames; the first 2 are not counted.

| scene shape | end waited for | the call that takes the counted wait | the job it waits for |
|---|---|---|---|
| one pass, CPU-bound (20 ms CPU, 2 ms GPU) | 0 of 6 | none (the frame before is over when the swap's flush looks) | - |
| one pass, GPU-bound (1 ms CPU, 10 ms GPU) | 6 of 6 | `eglSwapBuffers` | the frame before's window job (the designed wait) |
| shadow | 6 of 6 | the ground's `glDrawArrays` (the depth resolve) | the same frame's depth pass |
| terrain | 6 of 6 | `glGenerateMipmap` | the same frame's pass into the target |
| refract | 6 of 6 | `glGenerateMipmap` | the same frame's colour pass |
| desktop blur | 6 of 6 | `eglSwapBuffers`, in the back end's flush | the same frame's last blur pass |
| desktop shadow | 6 of 6 | `eglSwapBuffers`, in the back end's flush | the same frame's desktop pass |

In every shape, no swap waits for the window job it submits, and no swap
calls `finish`. The same 7 tests pass in the PowerVR, Vivante and V3D
builds.

### Cause

**The swap does not wait for its frame's end.** The five scenes render to
textures before their window pass. The counted wait is the CPU waiting for
one of those off-screen jobs of the same frame:

- shadow: the depth resolve, in the ground's `glDrawArrays`, for the depth
  pass;
- terrain and refract: `glGenerateMipmap`, for the pass that rendered the
  target (the TFU, or the CPU filter, needs it done);
- desktop blur and desktop shadow: the one job in flight. The swap's flush
  submits the window's job and first waits for the frame's last off-screen
  pass. That pass was submitted only at the window's first draw, so it is
  still running.

The window job itself is never waited for by its own swap. So the EGL
double buffer works as designed, and `src/gl/egl.rs` needs no change. A
GPU-bound single-pass scene also reads N of N, from the designed wait for
the frame before. The counter alone cannot tell these cases apart.

### What this means for the frame rate

The waits come from the V3D back end and the scenes' own work, not from
the swap:

- The depth resolve (shadow, refract) is item `pi4-depth-resolve-gpu`.
- The mipmap wait (terrain, refract) stays while `glGenerateMipmap` needs
  the render done. Terrain builds its chain twice a frame (item
  `pi4-terrain-cpu`).
- One job in flight limits the overlap in every multi-pass scene. The run
  shows that the window job of frame N is waited for at frame N+1's first
  off-screen submit. So frame N+1's CPU work runs beside frame N's window
  job only until that submit. A queue of two jobs in the V3D OSAL and back
  end removes this wait. That is a change in `src/v3d` and
  `src/osal/zephyr_v3d.c`, not in the swap.

### The queue of two jobs

The back end keeps two submitted jobs (`V3dState::queued`, `in_flight`).
`flush_job` waits only for the older one, and only when two are
outstanding (`wait_queued`). Every other wait (a texture read, a CPU read, a
sync, `v3d_finish`) waits for both (`wait_in_flight`). The OSAL runs one job
on the GPU and holds the second kick in a slot (`s.pending`): the binner
and the tile memory are shared, so the second job starts only when the first
is finished (`start_pending`). TFU jobs are tagged with the job they follow.
The OSAL keeps the result of the last two completed fences (`z_wait`), so
an error of the older job is still reported when the newer one has
completed before the older is waited for.

The host test is `a_frames_first_offscreen_submit_does_not_wait_for_the_previous_window_job`
in `src/v3d/backend_tests.rs`: frame N+1's first off-screen submit waits for
the off-screen job of frame N, not for its window job. The OSAL's
per-fence result is Zephyr C and has no host test.

Not measured: the waiting job starts when the OSAL next looks (`wait`,
`completed`, a submit, an allocation), because FRDONE has no interrupt. If
the CPU records for a long time without calling the OSAL, the GPU idles
after the first job. An FRDONE interrupt that starts the waiting job would
close this gap. The Pi 4 glmark2 run against `33a0f264` has not been made.
