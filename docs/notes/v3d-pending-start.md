<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: starting the waiting job without the CPU in a wait (study)

Candidate 2 of `pi4-terrain-cpu-left.md`: on the Pi 4 a job submitted while
another runs waits in a slot, and starts only when the CPU next looks. This
note answers where, whether an interrupt could start it, which entry points
could look cheaply, whether that is exact, what it gains on terrain and what a
board record must show. Code is cited at `83c00813` (the tip this was written
on); no code was changed and no board was used. Figures are those of
`pi4-slow-terrain.md` (terrain 34.776 ms a frame at `e07d066a`, 8.003 jobs a
frame, `wait.ns` 28.38 ms a frame) and `pi4-terrain-cpu-left.md`.

**Result.** The start is already a single function (`start_pending`) and the
look that triggers it already exists as an OSAL op the back end calls (`completed`,
`z_completed`), so the hook needs **no OSAL change**: a call to
`completed()` from the V3D back end's draw, clear and flush paths, only while a
job is outstanding. It is exact (same job, same order, same fences). The gain
is the part of the GPU's idle time between a short job's end and the next
look: 0.4 to 1.6 ms a terrain frame, estimated; it was never measured. One
correction to the ranking note: the record value that confirms it is **not**
the sum of `idle` (see (e)).

## (a) Where a pending job is started today

A job is pending from `z_submit` finding one in flight until `start_pending`
runs. Created: `src/osal/zephyr_v3d.c:1395-1412` (`s.pending = true` at `:1412`,
the kick copied to `s.pend_kick`; the check is `s.inflight`, which stays set
until the CPU *looks*, not until the GPU is done). Started: only by
`start_pending` (`:1466-1478`), which calls `start_job` (`:1422`; the tile-state
`memset` at `:1431`, `v3d_submit_bin` at `:1457`). Every caller of
`start_pending`:

| call site | reached from | when |
|---|---|---|
| `zephyr_v3d.c:1316` end of `finish_inflight` | below | the job in flight has ended (FRDONE seen), its clean and queued TFU jobs run, fence completed |
| `zephyr_v3d.c:1215` end of `bin_finish`' failure branch | `finish_inflight` | a binning failure, after recovery |

So the real question is who calls `finish_inflight` (`:1254`):

| caller | line | trigger | blocks? |
|---|---|---|---|
| `z_wait` | `:1514`, `:1521` | the back end's `wait_job` (`src/v3d/backend.rs:3147`): `wait_queued` in `flush_job` (`backend.rs:3018`), `wait_in_flight` (swap, readbacks, settles), `rt_tfu_settle` (`:2542-2551`) | yes, until FRDONE (timeout 0 only polls) |
| `z_completed` | `:1486-1487` | the back end's `fence_done` (`backend.rs:3176`): `holder` (`:3119`), `rt_tfu_settle`, `v3d_kick_pending` (`:7099-7107`, GL core's `backend::kick_pending`) | no: only when `inflight_ready()` (`:1228`) sees FRDONE |
| `z_submit` | `:1395-1396` | one already pending: the running job is finished first | yes |
| `poll_completed` | `:395-398` | `poll_and_drain` (`:404`), from `z_gpu_alloc` only when frees are held (`:423-424`) and from every `z_gpu_free` (`:467`) | no (poll form, `finish_inflight(true)`) |
| `finish_all` | `:1322-1326` | `osal_v3d_tfu` (`:1344`), `z_gpu_alloc` out of memory (`:430`) | yes |

Between those, nothing looks. In terrain a job is one draw (8 jobs a frame),
and the order of events for job K (`flush_job`, `backend.rs:2935-3019`) is:
record K; `wait_queued` waits K-2, which makes the OSAL start K-1 from
`finish_inflight`; `z_submit(K)` finds K-1 in flight and parks K; then the CPU
records K+1. K-1 ends somewhere in that recording; K starts only at the
`wait_queued` of K+1's flush, i.e. after K+1 is recorded (0.1 to 0.4 ms on
the board, `pi4-terrain-cpu-left.md`). `gpu_free` reaches `poll_completed`
too, but the draw path frees little (terrain: the per-draw frees go through
the back end's arenas, not `gpu_free`; this was not counted, see Limits).

## (b) Is there an interrupt path?

Partly, and it is not the one that matters. `v3d_isr` (`:1010`) is connected
at init (`IRQ_CONNECT`, `:261-262`) and the mask holds only
`V3D_BIN_IRQS` = FLDONE | OUTOMEM (`:170`, unmasked in `start_job`, `:1456`).
On FLDONE it starts the *render* of the job whose bin just ended
(`v3d_submit_render`, `:1048`): so the binner-to-renderer hand-off needs no
CPU. It deliberately never completes a fence: "FRDONE, the TMU clean and the
fence stay in `finish_inflight` (the frees rule, `v3d_frees.h`)" (comment
`:1002-1008`). FRDONE is not unmasked and the code says "FRDONE has no
interrupt" (`:1273`; `v3d-wait-poll.md` section 2.4, which also notes the
status bit exists, `v3d_regs.h:258`). The reasons this is not used, as the code
and `v3d-wait-poll.md` section 5 item 2 give them:

1. What follows FRDONE is not ISR-safe: the TMU/L2T clean spins
   (`v3d_caches_clean`, `:1302`), `tfu_queue_run` runs TFU jobs with a busy
   spin (`:1311`, `tfu_now`), `complete` runs `drain()` (frees, allocator
   state), and `start_job` does a `memset` of up to 30 KB of non-cacheable
   memory (`:1431`). All of it would sit in interrupt context or move to a
   deferred thread.
2. `finish_inflight` and `start_pending` are thread-context code with no
   lock against the ISR except the `irq_lock` inside `bin_finish` (`:1167`).
   An FRDONE ISR would race with `z_wait`'s `finish_inflight` on
   `s.inflight`, `s.pending` and the fence state.
3. The hang bound and `recover` need the CPU looking.

What would be possible is a *minimal* FRDONE ISR that only starts the pending
job's bin (the registers are the same as `start_job`), leaving the clean,
TFU and fence to the thread. It cannot do that exactly: the TMU clean must run
before the next job (it is part of the previous job's end, and a queued TFU
job must run in the idle GPU between them, `:1311`), and the memset must come
first. So an ISR start would have to reorder them or do them in the ISR. That
is the "medium" risk of `v3d-wait-poll.md`; the polled hook below needs none
of it. An interrupt remains the only way to start with the CPU in a long
non-OSAL loop (a CPU mipmap filter, `texImage`), where no entry point is
called; terrain has none after the mark fix (`pi4-slow-terrain.md`, 0
`level0-back` and `chain-back`).

## (c) Entry points for a "kick pending" hook

The hook is `completed()`: `z_completed` (`:1482-1489`) reads two registers
when a job is in flight and its binner is done (`inflight_ready`, `:1228`:
core and hub `INT_STS`; one register, hub, while binning), none when nothing
is in flight, and then `finish_inflight(false)`, which does not wait (FRDONE is
up) but does the clean, the queued TFU jobs, the fence, `drain()` and
`start_pending` (the next job's `memset` and bin start). A read is a
non-cacheable device access; its cost on the Pi 4 was not measured; at 0.5 to
1 us 24 calls a frame (3 entry points x 8 jobs) cost 0.01 to 0.03 ms. The
calls that do finish a job cost what the same finish costs in a wait today
(10 to 60 us, `v3d-wait-poll.md` section 4) plus `memset`; they move that CPU
time from inside a wait into the recording.

Where to put it, cheapest and least conflicting first:

1. **`z_submit`, before `if (s.pending)` (`:1395`)**: poll once
   (`if (s.inflight && inflight_ready()) finish_inflight(false)`). Exactly
   the case where K-1 has already ended when K is submitted: K then starts at
   once and is not parked. One register pair a submit (8 a frame). Needs an
   OSAL edit (`src/osal/**`), and `tools/check.sh` has no host build of
   `zephyr_v3d.c` (`:2807-2823` build only its headers), so no host test.
2. **In the V3D back end, around the recording** (no OSAL edit, no
   `src/gl` edit): `if g.in_flight.is_some() { fence_done(...) }`, i.e. one
   call to the `completed` op, at the top of `draw_in_job`
   (`src/v3d/backend.rs:6130`), of the clear path
   (`backend.rs:6898-6910`, the `begin_job` there) and of `begin_job`
   (`:2654`). That is every point where terrain's recording passes: the GL
   entry points `glBindFramebuffer` (`src/gl/framebuffer.rs:1909`), `glClear`
   (`src/gl/state.rs:445`) and `glDrawArrays` (`src/gl/draw.rs:5294`) reach the
   back end through `v3d_draw` (`backend.rs:6675`), the clear path and
   `flush_job`, so the back end sees them with no `src/gl` change. Per call:
   a function-pointer call and the register pair. A hook in `src/gl/*`
   itself would reach every back end (PowerVR and Mali have other
   `completed`s with other costs, e.g. a fence read through the firmware) and
   conflicts with the items editing `src/gl/draw.rs` and
   `src/gl/framebuffer.rs`; do not.
3. Per-call placement inside a draw (not just at its start) buys little: the
   gain is the lag between a job's end and the *next* look, and with calls
   at 8 draws and 8 flushes a frame the longest unlooked stretch inside one
   job's recording is the part after its last call (the state emit, 6.5 us a
   job, and the draw's texture setup, 5.3 us, `pi4-terrain-cpu-left.md`
   candidate 4): tens of microseconds, not the 0.1 to 0.4 ms of recording.

**Reentrancy and locking the OSAL requires.** There is no mutex or
`k_spin`: the OSAL's calls (`z_submit`, `z_wait`, `z_completed`,
`osal_v3d_tfu`, `z_gpu_alloc`, `z_gpu_free`) run in the one application
thread; only `v3d_isr` is asynchronous, and it touches `bin_pending`,
`bin_rc`, `s.ov`, `s.bin`, `if_c_fldone`, `if_c_render` and the render
registers (`:1010-1056`) and never `s.inflight`, `s.pending` or a fence. The
hook must therefore:

- be called only from that thread, never from the ISR or a callback;
- not be called from inside a back-end sequence that has half-updated
  `g.queued` / `g.in_flight` (`flush_job` between `wait_queued` and
  `submit_job`): `finish_inflight` completes a fence, and `fence_done` users
  re-read `g.in_flight`'s fence (the back end only keeps the two fences, it
  does not reorder on completion, so a call at the *top* of a function is
  safe);
- not hold an `irq_lock` over the call (`bin_finish` takes one itself and
  spins `k_busy_wait(10)` inside its loop, `:1220`; `z_completed` avoids
  that loop because it enters `finish_inflight` only when `inflight_ready()`
  is true, and `inflight_ready` is true during binning only for a fault or the
  hang bound);
- leave the TFU rule alone: `osal_v3d_tfu` still starts with `finish_all`
  (`:1344`), and a queued TFU job (`tfu_queue_run`, `:1311`) now runs in the
  hook's call rather than in the next wait, so a call that finishes a job
  with TFU jobs behind it blocks for their run (806 to 114 us each for
  terrain's chains, `pi4-slow-terrain.md`; 2.64 ms a frame in all). That
  time is GPU time that had to elapse, but the recording is late by it: it
  is the one way the hook can cost frame time (below).

## (d) Is it exact?

Same job, same order, same results; what changes is when.

- Order: jobs run one at a time in submit order (`start_pending` starts only
  `s.pend_kick`, the next job; there is one pending slot, `:1395`), the
  render still follows from the ISR at FLDONE, the clean and TFU jobs still
  run between the render and the next bin (`:1302-1311`, then `:1316`). An
  earlier `completed()` runs the same `finish_inflight`.
- Fences: `complete` (`:1144`) gives the same `completed` /
  `prev_completed` pair; `z_wait` answers an old fence from them (`:1527`),
  so a wait after an earlier completion returns the same `last_rc`. An older
  fence's result is kept for two completions (`prev_*`); the hook can finish
  two jobs before the back end waits for the older one, which is the case the
  comment on `z_wait` already names ("both may have completed, in either order
  of its waits").
- Results: unchanged pixels. Earlier completion lets `drain()` return frees
  earlier, so `gpu_alloc`'s first fit may place a later block at another
  address (visible in a dump, not in a picture), and `kick_pending`
  (`:7099`) answers false earlier, which makes the GL core's settle paths
  skip a wait that would have returned at once; the counters `waits`
  (`g.waits`, each `wait_job`) do not change, since the back end still calls
  `wait` for each fence. The `~` mark (`render_late`, `:1273-1275`) stays on
  those jobs (FRDONE is up when the hook looks) but their `render` shrinks
  to nearly the real time.
- A failure: a hung or faulted job is recovered by the same code
  (`:1286-1294`) one look earlier; `hung_refused` and the `-EIO` path are
  the same.
- ES 2.0 answers: none depends on timing; `tests/es2_report_guard.rs` pins
  strings and queries, and a change in `src/v3d/backend.rs` that only adds a
  call to an op cannot reach them. `tests/es2_report_guard.rs` should be
  in the gate rows regardless (`tools/check.sh --changed` reaches it through
  `src/v3d/**`).

## (e) Expected gain, and the record that confirms it

The loss is the GPU idle between a short job's end and the look that starts
its successor. Let R be the time it takes to record the next job after the
successor is parked (0.1 to 0.4 ms), and D the duration of the running job.
The exposure of a pair is about `max(0, R - D)` less the stretch after the
last call, and the 8 jobs of terrain are 4 short ones (jobs 1, 2, 4, 5:
16 tiles, well under 1 ms) and 4 of 7 to 9 ms (`pi4-terrain-cpu-left.md`):

    4 short jobs x (R - D) ~ 4 x (0.1 to 0.4) = 0.4 to 1.6 ms a frame

consistent with the candidate's 0.5 to 1.5 ms. It is a bound only if the
frame is GPU-bound at those points, which it is (28.4 of 34.7 ms is inside
`wait_job`); the hook cannot help a stretch where the CPU is the limit, and it
moves the clean, the memset and any queued TFU run from the wait into the
recording, so the CPU time outside the waits (34.717 - 28.38 = 6.34 ms at
`e07d066a`) rises by what moved (the memset 0.07 to 1.8 ms, 10 to 60 us a
clean) while the waits fall by the same and the frame falls by the idle that
went. If the recording is the critical path at a point the hook moves a TFU
run into, the frame can lose up to that TFU run (0.1 to 0.8 ms): terrain's
chains are queued behind the *last* job of the frame's mipmap pass, so check
this on the board before keeping the hook at the clear path.

**What does not confirm it: `idle`.** `pi4-terrain-cpu-left.md` candidate 2
says to read the sum of `idle` over the frame-10 line. That field is
`s.if_c_bin - recent_prev_end` (`:769`), and `recent_prev_end` is `c_end`, the
stamp the OSAL takes when the CPU *saw* the previous job end (`:1312`, then
`note_job_done`, `:1315`). A parked job starts inside the same
`finish_inflight` that stamped it, so its `idle` is only the OSAL's own edge
(the fence, `drain()`, the `memset`: 10 to 60 us plus the memset), whether the
GPU waited a microsecond or a millisecond for the look. The lag is inside the
**previous** job's `render` (`last_job_cyc.render = c_frdone - s.if_c_render`,
`:1285`, taken when the CPU looked, flagged `~`). `idle` is large only for a
job submitted to an idle OSAL (no job outstanding), which this hook does not
touch. So `idle` stays about the same after the change; do not use it as a
gain and do not read a small sum as "no gain".

**Record values that confirm it** (Pi 4, `tools/cts/board-stage.sh rpi4
common`, the same image otherwise; `glmark2-jobs` frame 10 and
`glmark2-counters: terrain`; `glmark2-jobs-bin-prep` adds `prep` and `bin`):

1. **Terrain FrameTime and `glmark2-frame` frame us fall by 0.4 to 1.6 ms**
   from 34.776 / 34717 us at `e07d066a` (the same commit plus the hook;
   more than 1.6 ms would mean idle the estimate did not count, none would
   mean the recording is not the critical path). This is the confirming
   value.
2. **The sum of `render` over the `~` jobs of the frame-10 line falls by
   about the same amount** (its `render` carries the lag; the line's render
   sum is 23.49 ms at `e07d066a` and went to 27.26 ms in a later run, which
   `v3d-jobs-readout` is explaining; a rise of that kind with a flat frame
   time is this lag moving). The `~` marks stay; the render of a short job
   goes from about the recording time to about its real 0.2 to 0.8 ms.
3. `glmark2-counters` `wait.ns` a frame falls by the moved clean and
   memset, and **frame less `wait.ns`** (6.34 ms) rises by that much, not
   by more than 2 ms (more means the TFU run moved into the recording and the
   chains are the cost). `waits` stays 8.003 a frame (the back end still
   waits for each fence): a change in it means the hook changed an order,
   which it must not.
4. `idle` sum unchanged within the 10 to 60 us a job (it is a control,
   not the gain). With `prep` printed: the `prep` of a parked job falls by
   the lag (it counts from the submit, `:1406`, `last_job_cyc.prep = c_bin -
   c0`), the others do not.

## (f) Diff sketch (not applied) and the test that pins it

Back end only (`src/v3d/backend.rs`, owner of that tree: plan #308; apply
after it releases the file, small, on the current `origin/main`). `fence_done`
is the existing look (`backend.rs:3176`); a variant that ignores the result
and skips the call when nothing is outstanding:

```diff
+/// Let the OSAL look at the job in flight (a register read) so that a job
+/// parked behind a finished one starts now, not at the next wait: the one
+/// call to `completed`, only while a job is outstanding. Order and fences
+/// are the OSAL's (docs/notes/v3d-pending-start.md).
+fn kick_pending_look(g: &V3dState) {
+    if g.in_flight.is_none() {
+        return;
+    }
+    if let Some(c) = osal::ops().and_then(|o| o.completed) {
+        // SAFETY: a plain query.
+        let _ = unsafe { c() };
+    }
+}
@@ fn draw_in_job(g: &mut V3dState, target: Target, d: &GlesDraw, ...) -> i32 {
+    kick_pending_look(g);
     let mut retried_for_memory = false;
@@ fn begin_job(g: &mut V3dState, target: Target) -> i32 {
+    kick_pending_look(g);
     let rc = choose_arenas(g);
```

and the same one call at the top of the clear path (`backend.rs:6898`). An
optional `z_submit` poll (candidate 1 of the list in (c)) is a second,
independent change in `src/osal/zephyr_v3d.c` (`:1395`):

```diff
-	if (s.pending)
-		(void)finish_inflight(false);
+	if (s.inflight && !s.pending && inflight_ready())
+		(void)finish_inflight(false); /* K-1 is over: start this job at once */
+	if (s.pending)
+		(void)finish_inflight(false);
```

**Host test that pins it.** The OSAL cannot be run on the host (no build of
`zephyr_v3d.c` is in `tools/check.sh`), so the pin is on the back end's
sequence of OSAL calls, with the V3D back end test platform
(`src/v3d/backend_tests.rs:334-527`, `p_completed`; its model completes jobs
as the test sets it, `async_jobs`) or a `tests/v3d_*.rs` file in the style of
`tests/v3d_depth_storage.rs` with its own `OsalOps`:

1. Record terrain-shaped jobs (one draw each, 8 jobs, the second
   framebuffer's clear first) with a fake `completed` that counts calls and
   a fake `wait`/`submit` that log `(op, fence)`. Assert: (i) the sequence of
   `submit` and `wait` fences is **identical** with and without the hook
   (order, fence numbers, `wait` count: 8 a frame); (ii) `completed` is
   called before each draw's first record and the clear's, **never** with no
   job outstanding, and never from `flush_job` between `wait_queued` and
   `submit_job`; (iii) with a fake in which job K-1 completes at the call,
   the parked job's start is reported by the fake before the next `wait`
   (the hook's whole effect).
2. `tests/es2_report_guard.rs` unchanged (ES 2.0 answers), plus
   `tests/v3d_terrain_jobs.rs` (`v3d-terrain-job-pin`: 8.003 jobs a frame, 289
   of 289 waited) unchanged, which pins that no job is added or merged.
3. For the OSAL part (optional poll in `z_submit`), a C check beside
   `tools/v3d/osal_frees_check.c` that includes `zephyr_v3d.c` against a mock
   of the registers (`tools/v3d/hw_mock_test.c` is the starting point) is
   needed before it can be gated; without it only the board checks it, so it
   is left out of the first change.

## Limits

- No record value of `prep`, `bin` or the per-job lag exists: every figure
  under (e) is an estimate from the recording time and the short jobs'
  durations, as in `pi4-terrain-cpu-left.md`. The first board run decides.
- The register-read cost (0.5 to 1 us) is assumed, not measured; `idle`'s
  10 to 60 us is the only recorded OSAL edge.
- Which `gpu_free` calls the draw path makes in terrain was not counted:
  each is a free poll (`poll_completed`) and so already a look. If there
  are several a draw, part of this gain is already taken and the board's
  `~` render values will be smaller than assumed.
- Not touched: `src/osal`, `src/v3d`, `src/gl`, `driver/`, other notes.
  The correction on `idle` belongs in `pi4-terrain-cpu-left.md` candidate 2
  and `v3d-wait-poll.md` section 6 (the "(b) is the gap" row), by their
  owners.
