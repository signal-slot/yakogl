<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): what glmark2's FrameTime measures on the panthor-gles pbuffer

Date 2026-11-08. Plan #332 item `frametime-note`, for the first glmark2-es2
record of the NanoPC-T6 (issue #20, `driver/docs/nanopc-t6-mali.md`). It is a
reading of the code, host research only: no board run, no file outside this
note changed, `gh` was not run.

## Source read

- yakogl at the plan's integration tip `722494ee` (`src/gl/egl.rs`,
  `src/gl/state.rs`, `src/gl/es3.rs`, `src/backend.rs`, `src/mali/backend.rs`,
  `tools/mali/panthor/src/osal.rs`, `tools/mali/panthor-gles/src/lib.rs`).
- glmark2 at `22c527cb0556f3a1ac4445aaa52cc532760928d5`, the pin of
  `tools/cts/mali-glmark2-build.sh` (`GLMARK2_PIN`, line 93): `src/scene.cpp`,
  `src/main-loop.cpp`, `src/canvas-generic.cpp`, `src/gl-state-egl.cpp`,
  `src/options.cpp`, with `tools/cts/mali-glmark2-pbuffer.patch` applied.

Line numbers are of those files at those commits. `src/mali/backend.rs`
lines below 1100 are the same on `origin/main` `c6b16f71`; above that, where
main is 50 lines further down (#309's work), they are given as
`integration/main`. The other files cited do not differ. Recheck them with the
section "When to recheck".

## What FrameTime is

`FrameTime: N ms` is `1000 * stats.average_frame_time`
(`main-loop.cpp:178-180`, `:191-192`), and `average_frame_time` is
`realTime_.elapsed() / currentFrame_` (`scene.cpp:150`): the wall clock from
the end of the scene's setup (`realTime_.start = realTime_.lastUpdate`,
`scene.cpp:249-251`, taken after `load()` and `setup()`, so shader compilation
is not in it) to the **last `Scene::update()`** (`scene.cpp:112`, which
`elapsed()` reads, `scene.h:262`), divided by the frames counted. The scene
runs until `elapsed() >= duration_` (`scene.cpp:116`, the `duration` option) or
`nframes_` frames (`:119`). The FPS in the same line is
`ceil(1 / average_frame_time)` (`main-loop.cpp:178`); the score adds
`currentFrame_ / elapsed()` (`scene.cpp:141`, `main-loop.cpp:118`). Both are the
same quantity.

glmark2 reads no GPU time. The per-frame sequence of the plain loop is
`canvas_.clear()`, `scene_->draw()`, `scene_->update()`, `canvas_.update()`
(`main-loop.cpp:130-138`); the pbuffer flavour's `MainLoopPicture::draw` is the
same with a `glReadPixels` of the **last** frame inserted between `update()`
and `canvas_.update()` (`mali-glmark2-pbuffer.patch`, `MainLoopPicture::draw`).
`canvas_.update()` on this flavour is `FrameEndSwap`: a pbuffer is not
`offscreen_` and `--validate` is off, so the default frame end is
`gl_state_.swap()` (`canvas-generic.cpp:107-128`), which is
`eglSwapBuffers` with its result ignored (`gl-state-egl.cpp:464-467`). No
`glFinish`, sync or `eglWait*` is on the default path. glmark2 also asks for
swap interval 0 (`gl-state-egl.cpp:426-430`, the pbuffer flavour is not
`GLMARK2_USE_NULL`); the library does not wait for a vertical period on this
path either: the panthor OSAL has no `wait_vblank`
(`tools/mali/panthor/src/osal.rs:553`, `None`), so the swap's period loop
(`src/gl/egl.rs:2438-2442`) answers `-ENOTSUP` and stops at the first try.

## What the swap waits for

`eglSwapBuffers` (`src/gl/egl.rs:2352`) on a pbuffer:

1. `backend::flush()` (`:2396`) ends and submits the open frame
   (`mali_flush`, `src/mali/backend.rs:2925`/`:2975`): one kick, one fence, into the
   frame slot `g.next` (`:2590`/`:2640`).
2. The pbuffer is neither `complete` (that is a window rendered in place) nor
   `idle` (`back` only), so the wait is `backend::finish_previous()`
   (`:2405-2413`), not `finish()`.
3. `mali_finish_previous` (`src/mali/backend.rs:2944-2952`/`:2994-3002`) waits for slot
   `g.next`, which after the submit is the slot the frame before it used (with
   `SLOTS = 2`, `:315`); if that slot holds nothing in flight it returns at
   once. The wait is one fence wait of at most `WAIT_US = 5_000_000` (`:318`,
   `:949-952`) on the panthor OSAL's syncobj wait
   (`tools/mali/panthor/src/osal.rs:413-435`).

So after a swap at most one frame (the one just submitted) is in flight, and
the swap returns when the frame before it is complete. The next frame's first
draw takes slot `g.next`, the same slot `finish_previous` just waited for
(`open_frame`, `:2475-2476`/`:2525-2526`), so it waits for nothing new. The claim of the
investigation holds, with two refinements:

- `SLOTS` also bounds the **recording** side: the CPU may be at most one
  frame ahead of the GPU, no more, because there are no more slots.
- A swap with no draws since the last one submits no kick and leaves `g.next`
  where it is, so `finish_previous` then waits for the slot of the last real
  frame, not the one before it. glmark2 scenes draw every frame, so this does
  not occur in a scene; it would if a scene clears nothing and draws nothing.

## One frame of slack, and a little more

`update()` of frame *k* runs **before** frame *k*'s swap. When the last
`update()` (frame *N*) stamps `lastUpdate`, the swaps of frames 1..*N*-1 have
returned, so frames up to *N*-2 are complete (swap *N*-1 waited for frame
*N*-2); frame *N*-1 is submitted and may be running; frame *N* is still being
recorded and is not submitted. The elapsed time therefore contains the GPU
time of about *N*-2 frames plus the CPU time of *N* frames, and is divided by
*N*. The investigation's "one frame of slack" counts the frame in flight at a
swap; measured at glmark2's own stamp it is up to **two frames of GPU time
not in the elapsed time** (frames *N*-1 and *N*), and the readback and the
final swap that wait for them come after the stamp, so they are never in it.

For a GPU-bound scene (frame GPU time *g* above the CPU time) the reported
FrameTime is about *g* x (*N*-2)/*N*, a low bias:

| frames *N* in the scene | share of the GPU time shown |
|---:|---:|
| 5 | 60 % |
| 10 | 80 % |
| 50 | 96 % |
| 200 | 99 % |

For a CPU-bound scene (shader and uniform work on the CPU per frame, the
slots never full) FrameTime is the CPU time and the GPU is not measured at all.
Either way it is a **throughput** of the whole pipeline, never a latency of one
frame: it says how often a frame leaves, not how long one took.

When it matters:

- **Short runs.** A first-record run with a short `duration` (a few seconds on
  a slow scene, or few frames because a scene is slow) has small *N*; below
  about 50 frames the bias is above 4 %. glmark2's default duration is 10 s
  per scene; the runner should record the number of frames (it is
  `FPS x duration`, or the `nframes` option) next to the FrameTime and a
  tally should not compare a scene run at 2 s with one at 10 s to better than
  the bias.
- **The first frame.** `realTime_.start` is taken after `setup()`, so the
  scene's shader compilation is not in it, but any work the first frames do
  once (first-use uploads, a first-draw program build, a first growth of a
  GPU-side buffer) lands in frame 1 and is counted. That is one frame of *N*;
  it matters when *N* is under about 20 and the first frame is much dearer
  than the rest. Which of these the Mali back end has is not read here: the
  board run shows it as a first frame far above the steady ones (compare the
  `--frame-end finish` run).
- **A scene that is slower than the duration.** A scene that renders one or
  two frames in `duration` (the heaviest `shadow`, `jellyfish` or `terrain`
  scenes on a slow GPU) has *N* of 1 to 2: the bias formula gives 0 to 50 %
  and FrameTime is mostly the CPU time. `nframes_ > 0` stops a scene at that
  frame count regardless (`scene.cpp:119`).

To measure the GPU's own throughput without that bias, run a scene with
`--frame-end finish` (`options.cpp:112-124`, `canvas-generic.cpp:131-133`):
`glFinish` after every frame, which waits for the frame just submitted
(below). That is a different measure (no overlap of CPU and GPU), so it is a
second column, never a replacement.

## glFinish, eglWaitGL, fences

- `glFinish` (`src/gl/state.rs:1534-1543`): emit the pending clear, `flush()`,
  then `finish()`; `mali_finish` (`src/mali/backend.rs:2931-2942`/`:2981-2992`) submits the
  open frame and `wait_all` (`:1003-1018`) waits for both slots, oldest first.
  The result of `finish()` is dropped, so `glFinish` returns no error: a lost
  frame shows only in the exit line's counts.
- `eglWaitGL` and `eglWaitClient` are the same wait (`src/gl/egl.rs:2467-2484`,
  `wait_gl`), only with a context current.
- `glFenceSync` flushes (`src/gl/es3.rs:1035-1060`) and records the last kick;
  `glClientWaitSync` and `eglClientWaitSync` wait for that kick with
  `backend::wait_kick` and fall back to `finish` when the table has none
  (`src/gl/es3.rs:1088-1120`, `src/gl/egl.rs:2556`). The Mali table has
  `wait_kick: None` (`src/mali/backend.rs:3231`/`:3281`), so on the panthor-gles pair
  every sync wait is a full `finish`. A wait that fails is `GL_WAIT_FAILED`.
- `mali_shutdown` waits for everything (`:272-279`), after the last scene.

glmark2 on the default frame end uses none of them (above); only
`--frame-end finish`, `--off-screen` and `--validate` do (`canvas-generic.cpp:
112-125`).

## A frame that never completes

`wait` (`src/mali/backend.rs:949-964`) returns one of three things:

- `rc > 0`, the OSAL's wait timed out after `WAIT_US` = 5 s: `WAIT_TIMED_OUT`,
  `-EIO`.
- `rc < 0`, a fault the OSAL reported (panthor's `o_wait` returns `-EIO` for
  a group that faulted or timed out, `osal.rs:430-435`): `WAIT_FAILED`.
- `0`, complete.

On either failure the slot keeps its frame (`wait_slot`, `:978-998`) with
`wait_failed` set. The consequences for FrameTime:

1. **The first such wait costs up to 5 s**, one time, if the fence never
   signals and the kernel gives no verdict first. A panthor job timeout or a
   CS fault reaches `o_wait` as `-EIO` earlier than that (the kernel side is
   `docs/notes/mali-panthor-kmsg.md`, sections 3 and 4), so the 5 s is the
   bound, not the usual cost.
2. **Every later use of that slot fails at once.** `wait_slot` of a slot with
   `wait_failed` returns `-EIO` without waiting unless `completed()` has
   passed the fence (`:983-989`). `eglSwapBuffers` then returns
   `EGL_CONTEXT_LOST` (`src/gl/egl.rs:2413-2414`) immediately, `open_frame`
   refuses the draw (`:2476-2479`/`:2526-2529`), and glmark2, which ignores the swap's
   result, keeps looping with draws that do nothing. After the first loss the
   scene runs **fast**: FrameTime falls toward the CPU time of an empty frame,
   and the FPS rises. A hung scene can therefore report a *better* FrameTime
   than a healthy one, with the one 5 s stall inside the average.
3. The picture the patch reads at the end is the pbuffer as the lost frames
   left it. Its verdict is the picture check's, not FrameTime's.

So for a scene with any `gpu_faults` or `job_timeouts` in the
`yakogl-mali-exit:` line (`tools/mali/panthor-gles/README.md:58`, counted by
`w_wait`, `tools/mali/panthor-gles/src/lib.rs:239-245`), and for any scene
with refused draws, **the verdict comes from those counts and from
`mali-kmsg.py`'s counts, not from FrameTime**; the FrameTime is recorded but
marked not comparable.

## The line for RESULTS.md's Mali glmark2 row

> FrameTime is the scene's wall clock to its last `update()` over its frames,
> through `eglSwapBuffers` on a pbuffer, which waits only for the frame before
> it (2 frame slots): GPU throughput with up to two frames of GPU work still
> in flight at the stamp, a low bias of about 2/N for N frames (N from FPS x
> duration), CPU-bound scenes show CPU time. A scene with a GPU fault, a job
> timeout or a refused draw is marked and its FrameTime is not a measurement.

The row gives the frames per scene (or the duration) next to FrameTime so the
reader can apply the bias.

## When to recheck

Recheck every claim above, with the line numbers, when any of these changes:

- `SLOTS` (`src/mali/backend.rs:315`) or `WAIT_US` (`:318`), or the
  slot-choice (`g.next`) logic in `end_open`/`open_frame` and
  `mali_finish_previous`.
- The swap path in `eglSwapBuffers` (`src/gl/egl.rs:2352-2445`): the
  `complete`/`idle` choice for a pbuffer, or a `finish()` instead of
  `finish_previous()` there (FrameTime would then be the full GPU time, with
  no overlap).
- A Mali `wait_kick` (`src/mali/backend.rs:3231`/`:3281`), a `wait_vblank` in the
  panthor OSAL, or a change of `wait`/`wait_slot` failure handling.
- The glmark2 pin (`GLMARK2_PIN`), or `mali-glmark2-pbuffer.patch`'s
  `MainLoopPicture::draw` or its swap interval.
- The first board run: it should confirm the bias on one GPU-bound scene by
  running it with the default frame end and with `--frame-end finish`, and
  the 5 s stall on one hung scene, if one occurs.
