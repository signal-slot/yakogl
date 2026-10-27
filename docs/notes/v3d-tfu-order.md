<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: how render and TFU jobs are ordered, and a `glGenerateMipmap` without the CPU wait

Study for the Pi 4 item "v3d-tfu-mipmap-nowait". Read at main `eac6c257`
(line numbers below are that tree's). Nothing here is measured on the
board; where the board decides, the text says so.

Today each TFU `glGenerateMipmap` (`v3d_generate_mipmap`,
`src/v3d/backend.rs:1938`) submits the open render job, **waits for it on the
CPU** (the backend's `wait_in_flight`, `:1969`; the OSAL's own
`finish_inflight` inside the TFU query then finds nothing in flight and
returns at once) and then **spins on the CPU for the TFU job itself**.
Removing only the backend's wait removes nothing: the OSAL waits anyway.
The change has to reach the OSAL.

## 1. How the board's OSAL submits the two kinds of job

Render job (`z_submit`, `src/osal/zephyr_v3d.c:1211`..`1270`):

- `finish_inflight(false)` first (`:1228`, comment "one job in flight at
  most"): the previous job is finished (FRDONE, TMU clean, fence completed).
- the fence is `++s.submitted` (`:1238`); `s.inflight = true`.
- `v3d_submit_bin` (`driver/src/v3d/v3d_hw.c:283`, CT0QBA at `:299`) starts the
  binner. The FLDONE interrupt (`v3d_isr`, `zephyr_v3d.c:934`) starts the
  render with `v3d_submit_render` (CT1QBA, `v3d_hw.c:303`..`310`) and `submit`
  returns with the render running. The ISR unmasks only
  `V3D_BIN_IRQS` = FLDONE | OUTOMEM (`:156`); FRDONE has no interrupt
  (`:1135`).
- the job ends in `finish_inflight` (`:1118`..`1170`): `v3d_wait` spins on
  FRDONE (bounded by `JOB_HANG_US` from the render's start), then
  `v3d_caches_clean` (`v3d_hw.c:259`: TMUWCF, L2T clean), then `complete()`.
  It is called by `z_submit`, `z_wait` (`:1303`), `z_completed` (`:1278`, only
  when `inflight_ready()`), the free poll (`poll_completed`, `:376`) and
  `z_gpu_alloc` when memory ran out (`:412`).

TFU job (`osal_v3d_tfu`, `zephyr_v3d.c:1177`, reached by
`OSAL_QUERY_V3D_TFU_RUN` at `:1368`):

- `finish_inflight(false)` first (`:1190`..`1194`). The comment says why:
  "the TFU must not run beside a render (the render may read what it writes,
  and both use the TMU/L2T caches)" (tex, 2026-09-26). That is the **only**
  serialization: the V3D hardware does not order a TFU job after a render
  job already submitted. They are separate units (TFU registers `V3D_TFU_*`
  in the hub block, render CT1Q in the core block) that run at the same
  time if both are started. The OSAL orders them by never starting the TFU
  before the CPU has seen FRDONE.
- `v3d_tfu_run` (`v3d_hw.c:400`..`456`) writes IIA, IIS, ICA, IUA, IOA, IOS,
  COEF0 and, last, ICFG with the IOC bit (`:420`..`427`), then **polls**
  `V3D_HUB_CTL_INT_STS` for TFUC (or an MMU fault bit) with `k_busy_wait(1)`
  up to `JOB_HANG_US` (`:429`..`442`), checks that CVTCT advanced by one
  (`:449`..`453`) and returns. There is no fence and no queue for a TFU job:
  it is not counted in `s.submitted`, and nothing is in flight after the
  query returns. A fault or timeout takes the same `recover(0)` path as a
  render (`:1198`..`1208`).
- The TFU job writes no TMU clean afterwards; the next job's start
  (`z_submit`) is what follows it. (Whether a TMU/L2T flush between a TFU
  write and a render read is needed is not modelled anywhere in the
  repository; the pi4 board runs of the TFU path, `pi4-glmark2-cpu.md`, give
  no sign of trouble. Keep it as it is.)

Host OSAL (`src/osal/linux.c`): every submit completes inside `l_submit`
(`:827`..`850`), `l_wait` is `0` for any fence <= `submitted` (`:853`),
`l_completed` is `submitted` (`:859`). `tfu_query` (`:1056`) runs the job on
the CPU model and returns; no fence, no wait. So the host **cannot show the
wait's time**; it can show the **number of waits** only through a stand-in
OSAL (section 4).

Git: the TFU path came with `c6f17516` ("v3d: sample a render target from
its render image ... make its mip chain with the TFU"); the OSAL's comment
on the TFU rule is dated 2026-09-26.

## 2. What the OSAL offers to wait, or to order

`include/gles_osal.h:100`..`110`, `src/osal/mod.rs`:

| call | meaning |
|---|---|
| `submit(kick) -> fence` | start a job, return its fence (`>= 0`) |
| `wait(fence, timeout_us)` | 0 done, `> 0` timeout/still running (timeout 0 polls), `< 0` error |
| `completed() -> fence` | highest completed fence, never blocks |
| `query(OSAL_QUERY_V3D_TFU_RUN, OsalV3dTfu)` | run one TFU job, synchronously (`mod.rs:100`, struct 40 bytes, `mod.rs:119`..`131`, pinned by `BUILD_ASSERT` at `zephyr_v3d.c:1318`) |

There is **no way to make one job wait for another** (no in-sync, no out-sync
as in Mesa's `drm_v3d_submit_tfu` `in_sync`/`out_sync`, `v3d_tfu.c`; the
Mesa tree was not available on this machine, so that is from the task
description, not read). The only order the interface gives is "a job submitted
after another has run after it", and that only for render jobs. The backend
keeps one `InFlight { kick, fence, alias }` (`backend.rs:982`..`996`, field
`in_flight` `:1059`); `wait_in_flight` (`:2439`..`2457`) takes it, calls
`wait`, counts `g.waits`. `fence_done` (`:2461`) is `completed() >= fence`.

## 3. Who must be ordered after a TFU-made chain, and how today

The TFU writes the render image's levels 1.. (`RtImage`, `backend.rs:303`, its `chain` field `:331`
`chain`) from level 0, which the TLB stored (`rt_images[i].stored`,
`:2379`). Readers and writers of that memory:

1. **A draw sampling the chain** (`rt_sample`, `:1782`..`1830`, needs
   `stored` and `chain`). It lives in a later render job. Today ordered by
   the CPU: the TFU has finished before the query returned, so any later
   submit is after it. With the TFU asynchronous the ordering must come from
   the OSAL (`z_submit` finishing the TFU, below).
2. **The next render into the same target** (level 0 of the image is written
   by the TLB while the TFU reads it): write-after-read, same fix as 1.
3. **A CPU read of the chain**: `v3d_return_mipmap_chain` -> `rt_chain_back`
   (`:1898`..`1925`) reads `image_cpu()` levels 1.. with `v3d_tex_load_box`
   and writes the GL storage. Called by the GL core before it reads or
   writes levels 1.. on the CPU. Ordered today by the synchronous TFU.
4. **A tiled copy** (`copy_image_for`, `:3625`..`3650`). It reads the GL
   storage, not the image, and the TFU writes only the image. It waits for
   the *render* that rendered into the texture (`target_kick`,
   `v3d_kick_pending`, `wait_in_flight`, `:3640`..`3646`); no TFU is
   involved. No change.
5. **Freeing or re-laying-out the image** (`rt_free_image` `:1717`,
   `rt_relayout`): `gpu_free` is deferred until the newest fence that may use
   the block has completed (`v3d_frees.h` header; on the Zephyr OSAL
   `fence = submitted` with no job open, `+1` with one open). A TFU that
   takes a fence from `s.submitted` is therefore covered, a TFU with no
   fence (today) is covered only because it has already ended.
6. **`v3d_finish`, `v3d_wait_kick`, `v3d_shutdown`, `v3d_kick_pending`**
   (`:5403`, `:5427`, `:4961`, `:5515`): what the application means by
   "done" must include a queued TFU job (`glFinish`, an EGL/GL fence made
   after `glGenerateMipmap`, exit).
7. **The next `glGenerateMipmap` on the same image** (the frame after):
   its own render job is submitted first, which finishes the TFU of the
   frame before (as 2).

The backend's own orders, for reference: `flush_job` waits for the previous
job before submitting (`:2363`), so `in_flight` is at most one render job;
`choose_arenas` waits for a busy arena set (`:2431`); the draw paths that must
see a finished render use `wait_in_flight` (`:3643`, `:4754`).

The existing test `a_delegated_glgeneratemipmap_waits_for_the_render_in_flight`
(`src/v3d/backend_tests.rs:4273`..`4290`) pins today's order: with an
asynchronous stand-in OSAL it expects exactly **one** `wait` for the
render before the TFU query.

## 4. The smallest design

The OSAL gets a deferred TFU job; the backend stops waiting. The hardware
stays as it is: no TFU beside a render (the OSAL starts it only after FRDONE
has been seen), so the order rule of `zephyr_v3d.c:1190` is kept. What
changes is **who** waits and **when**: nobody on the CPU at the call.

### OSAL (`src/osal/zephyr_v3d.c`, `src/osal/linux.c`, `src/osal/mod.rs`, `src/osal/osal_internal.h`)

1. A new query `OSAL_QUERY_V3D_TFU_QUEUE` (next free number `17`: 15 and 16 are the Mali queries, `osal_internal.h:129-130`;
   mirror in `mod.rs` and `osal_internal.h`), in/out a struct equal to
   `OsalV3dTfu` plus `fence: i64` (out). `OSAL_QUERY_V3D_TFU_RUN` stays for
   the tiled copy (`tfu_tile`, `backend.rs:3370`..`3412`), which needs its
   result at once. `-ENOTSUP` where the OSAL has no queue: the backend falls
   back to today's path.
2. `zephyr_v3d.c`: split `v3d_tfu_run` into `v3d_tfu_start` (the register
   writes) and `v3d_tfu_finish(poll, timeout)` (the TFUC / fault poll and the
   CVTCT check). State: `s.tfu_fence`, `s.tfu_job` (copy of the registers),
   `s.tfu_state` in {none, queued, running}.
   - queue: `fence = ++s.submitted`; if no render is in flight start it now,
     else keep it queued. Returns at once.
   - `finish_inflight`: after `complete(render fence)` (it already did the
     TMU clean), start a queued TFU job. This is the only place a TFU job
     starts behind a render. FRDONE has no interrupt, so "after the CPU saw
     FRDONE" means: at the next `z_completed` poll that finds it ready, at
     the next `z_wait`, `z_submit`, `z_gpu_alloc` or free poll.
   - `finish_tfu(poll)`: new, the same shape as `finish_inflight`; completes
     the TFU's fence (`complete(s.tfu_fence, rc)`), runs `recover(0)` on a
     fault or timeout as `osal_v3d_tfu` does now.
   - `z_submit` (`:1228`), `osal_v3d_tfu` (`:1193`), `z_gpu_alloc`'s
     out-of-memory retry (`:412`) and `poll_completed` (`:376`): call
     `finish_inflight` **then** `finish_tfu`, so "one unit at a time" holds
     for the pair. That gives readers 1, 2, 7 and 5 their order with no
     backend wait.
   - `z_wait(fence)` and `z_completed` handle `s.tfu_fence` as they do
     `s.if_fence` (`:1303`, `:1278`): a wait on the TFU's fence finishes the
     render first, then the TFU. Fences are in submission order (render N,
     then TFU N+1), so `s.completed` stays monotonic.
3. `linux.c`: the queue runs the model at once (as `tfu_query`), fence
   `++v.submitted`, so a stand-in or the host harness sees no change in
   output. The host's wait count stays 0.

If the board shows that waiting for the CPU to notice FRDONE leaves the
GPU idle too long (the TFU starting only at the next poll), the next step is
to unmask FRDONE in `V3D_BIN_IRQS` and start the queued job from `v3d_isr`,
which already starts the render from FLDONE (`:972`). That is a larger step
(the ISR then also owns the TMU clean) and is **not** in this design.

### Backend (`src/v3d/backend.rs`)

1. `v3d_generate_mipmap` (`:1938`): drop the `wait_in_flight` at `:1969`.
   Keep the `flush_job` at `:1960` (the render must be submitted before the
   TFU is queued: the OSAL fence order is render then TFU). Use the queue
   query; keep `OSAL_QUERY_V3D_TFU_RUN` as the fallback when the queue
   answers `-ENOTSUP`.
2. State: `V3dState.tfu_flight: Option<TfuFlight { fence, image }>` next to
   `in_flight` (`:1059`). `wait_tfu(g)`: `wait(fence)`, count `g.waits`,
   log a failure once, and on an error `rt_images[image].chain = false`
   (the image levels cannot be trusted) and `g.rt_chains_failed += 1`.
   `fence_done`-style check for the pending test.
3. Call `wait_tfu` in: `rt_chain_back` (`:1898`, reader 3, only after its
   `r.chain` test, before it reads the image), `v3d_finish` (`:5403`),
   `v3d_wait_kick` (`:5427`), `v3d_shutdown` (`:4961`), and `v3d_kick_pending`
   answers `true` while the TFU fence is not done (`:5515`). `flush_job` does
   **not** wait for it: the OSAL does.
4. A TFU that fails after the call answered 1 leaves the GL core without a
   chain (it filtered nothing) and the backend with a bad image. Today a
   failed job answers 0 and the GL core filters. Keep that: **the first
   queued job per image (`rt_chains == 0` for it, or after a relayout) runs
   through the synchronous `OSAL_QUERY_V3D_TFU_RUN`**, as today. A job that
   works once with the same registers works again; later ones are queued.
   A late failure is counted (`rt_chains_failed`) and logged, and the
   next frame's `glGenerateMipmap` finds `chain` false and tries again.
5. Reason counters (the planner's item): one counter per `return 0` in
   `v3d_generate_mipmap` (not initialised, `levels < 2` / not a render
   target, no image, `seen_gen`, relayout, `tfu_min_bytes == 0`, flush
   failed, not stored / no image, wait failed, no layout, pack refused,
   no OSAL query, queue/TFU failed), printed on the frame-1 arena line.
   Unchanged by this study.

### Host test cases (stand-in OSAL, wait counts)

The stand-in is `Platform` / `p_submit` / `p_wait` / `p_completed` /
`p_query` in `src/v3d/backend_tests.rs:300`..`600` (async jobs, `tfu_min`,
`tfu_runs`, `counters()`). The backend's `g()` is private, so these cases
belong in that file (the item's `tests/v3d_tfu_mipmap_nowait.rs` can only
reach the public `YakoglBackend` table, with no way to install a counting
OSAL beyond what `tests/v3d_tfu_host.rs` does with `OsalOps` and the C
model). Extend `Platform` with the queue (a `tfu_queue: bool`, a TFU fence,
`waits_for: Vec<i64>`):

1. `generate_mipmap_queues_the_tfu_without_a_wait`: async jobs, a render in
   flight; `generate(&t, 3) == 1`; `counters().1` (waits) unchanged,
   `tfu_runs` has the job, the platform's TFU fence is after the render's.
2. `the_render_is_submitted_before_the_tfu_is_queued`: the platform's call
   log reads submit(render), queue(TFU).
3. `a_sampling_draw_after_the_queued_tfu_needs_no_backend_wait`: the next
   `flush` waits for the render only; the platform's `submit` is
   finish-then-submit (it finishes the TFU first, as the OSAL does) and the
   chain's levels are the filtered bytes at that moment.
4. `return_mipmap_chain_waits_for_the_tfu`: `v3d_return_mipmap_chain`
   with the TFU queued -> exactly one `wait` of the TFU fence, then the
   bytes are the chain back in the storage; with none queued, no wait.
5. `finish_wait_kick_and_shutdown_wait_for_the_tfu`, and
   `kick_pending_is_true_until_the_tfu_completes`.
6. `a_failed_queued_tfu_clears_the_chain`: `tfu_rc = -5` on the second
   job: counted, logged once, `chain` false, the third call tries again.
7. `the_first_job_per_image_is_synchronous` and
   `an_osal_without_the_queue_keeps_the_sync_path`: the old test 4273 still
   passes unchanged with `queue` off (it expects one render wait).
8. `five_mipmaps_a_frame_wait_for_nothing`: terrain's frame (5 calls) with
   async jobs and the queue on: `waits` grows by the frame's end-of-frame
   waits only, not by 5.
9. OSAL, C-side, host (`tools/v3d/osal_frees_check.c` style, the Zephyr file
   is not built on the host; the order rule is in a plain-C header like
   `v3d_frees.h`): a small model of `finish_inflight` / `finish_tfu` that
   checks "queued TFU starts after the render's completion; submit and
   alloc finish both".

The host harness before/after (`tools/hostharness/glmark2`,
`YAKOGL_V3D_TFU=1`) counts CPU `wait` calls from the stand-in or the
OSAL's counters, not milliseconds: `linux.c` completes everything at once, so
the saving is a Pi 4 number (board session). Expected: terrain, 5 waits for
the render + 5 TFU spins a frame -> the waits of the flushes and the
end-of-frame only; the GPU time is not removed (render + TFU still run
back to back), only the CPU's idle spin, so the board gain is the CPU/GPU
overlap, bounded by the CPU work between two `glGenerateMipmap` calls.

## Open points for the implementer

- The Mesa tree (`MESA_V3D_TREE`) was not available; `v3d_tfu.c` and the
  `in_sync` fields are cited from the task text, not read. Check them before
  copying names.
- Whether the first-job-synchronous rule is enough, or a late TFU failure
  should rather raise the CPU filter path, is a call for the board.
- `z_wait` semantics for a fence "older than the last completed" return
  `last_rc` only for the last one (`:1303`..`1310`); an error of a TFU job
  that is not the newest completed would be reported as `0`. Give the TFU
  its own `last_tfu_rc` or complete it strictly before any newer fence.
