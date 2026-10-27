<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: why a frame with more than one job waits at the swap, and how Mesa avoids it

Study for the Pi 4 item "v3d-swap-wait-study". Read at main `a7d3aea4`
(line numbers are that tree's). Read-only: no code is changed and nothing
here is measured on a board. The Mesa side is from knowledge of
`src/gallium/drivers/v3d` and the kernel's `drivers/gpu/drm/v3d`
(`MESA_V3D_TREE` is not set on this host), so it is marked as such; check
it against a tree before building on a detail.

How TFU and render jobs are ordered is `docs/notes/v3d-tfu-order.md`, and
which call takes each counted wait is `docs/notes/egl-swap-timeline.md`
("Why the Pi 4 still reads 'end waited for in N of N'"). Neither is
repeated.

## 1. The condition that makes the swap wait

`eglSwapBuffers` has no wait of its own for the frame it submits.

- `backend::flush()` (`src/gl/egl.rs:2373`) ends in `flush_job`
  (`src/v3d/backend.rs:2282`), which submits the open job.
- Then `finish_previous` (`egl.rs:2390`), which on the V3D is a no-op
  (`v3d_finish_previous`, `backend.rs:5448`): "at most the last job runs
  already". `finish` is taken only for a window that is not double-buffered,
  or an idle frame (`egl.rs:2389`).
- The show (`swap_window_buffers`, `egl.rs:2308`, `osal::present`) shows
  frame N-1's buffer. It is safe only because N-1's job has completed, and
  that is the wait below, not one in the swap.

The wait is `flush_job`'s `wait_in_flight(g)` at `backend.rs:2363`, "the job
before has run before this one is submitted: the OSAL runs one job at a
time". `wait_in_flight` (`:2439`) calls the OSAL's `wait` on the one fence in
`g.in_flight` (`InFlight`, `:982`, "at most one"). The condition is only
`g.in_flight.is_some()` when a job is submitted: there is no test of whether
the new job needs anything the old one writes.

Which job it waits for depends on how many jobs the frame has. Frame N with
an off-screen pass A and a window pass W:

| call | job submitted | job waited for |
|---|---|---|
| first draw into the window (a target change ends A) | A(N) | W(N-1), the window job of the frame before |
| `eglSwapBuffers` -> `flush` | W(N) | **A(N)**, the same frame's off-screen pass |

A one-job frame has only the second row's shape and waits for W(N-1): the
designed overlap (CPU records N while the GPU draws N-1). With two or more
jobs the swap's own submit waits for the frame's *own* previous pass, which
was submitted moments before at the window's first draw and is still
running. The CPU has recorded W(N) in full by then, so it is blocked for
the rest of A(N)'s GPU time with nothing left to do: no overlap with W(N)
at all. That is the swap column of scenes with more than one pass.

Other calls take the same `wait_in_flight`, each for the job in flight:

| site | why |
|---|---|
| `backend.rs:1969` `v3d_generate_mipmap` | the TFU must not run beside a render (the next item, `v3d-tfu-mipmap-nowait`) |
| `backend.rs:3643` `tex_state_for` | a texture sampled through its storage waits for the job that rendered it (`target_kick`); a render image costs nothing |
| `backend.rs:2432` `choose_arenas` | both arena sets busy |
| `backend.rs:4754` | no GPU memory for a kept tiled copy with a job open |
| the GL core's depth resolve | `finish` before the CPU detile (`src/gl/texture.rs`) |

Below the back end the OSAL enforces the same thing: `z_submit`
(`src/osal/zephyr_v3d.c:1228`) opens with `finish_inflight(false)`, so even
if the back end did not wait, the OSAL would, at the CPU, before starting
the binner. `z_wait` (`:1303`) waits only for the fence that is in flight.

## 2. How Mesa avoids it (from knowledge of Mesa and the kernel)

Mesa's v3d never waits on the CPU for a job when it submits the next:

- `v3d_job_submit` (`v3d_job.c`) fills `drm_v3d_submit_cl` and calls
  `DRM_IOCTL_V3D_SUBMIT_CL`. The ioctl returns when the job is queued. The
  context keeps one out-sync syncobj (`v3d->out_sync`); a submit passes it
  as `out_sync` and, for the next one, as an in-sync, so a context's jobs
  are ordered by dependency, not by a CPU wait.
- The kernel (`v3d_sched.c`) has a queue per unit (BIN, RENDER, TFU, CSD).
  A render job is split into a bin job and a render job; the render job
  depends on the bin job's fence. Each queue runs in submit order, and the
  scheduler starts the next job as soon as the previous one's fence signals,
  on the GPU side, with no CPU in the path.
- Buffers carry the hazards: every job lists its BOs, and the kernel
  attaches read and write fences to each (`v3d_lookup_bos`,
  `v3d_attach_fences_and_unlock`) and makes a job depend on the fences of
  the BOs it touches. A frame's off-screen pass writes a texture's BO; the
  window pass reads it, so the kernel orders the window job after it. The
  TFU queue works the same way: the TFU job is ordered after the render that
  wrote its input by the BO fence, not by a CPU wait (this is what the
  no-wait mipmap item relies on, `v3d-tfu-order.md`).
- Mesa never waits at the swap either. `v3d_flush` ends the frame, and the
  display side waits on the out fence in the kernel (explicit or implicit
  sync of the window BO), or, for a plain swap, not at all.
- The things the CPU has recorded stay alive by reference: a job holds its
  BOs (`job->bos`) and the kernel drops the references when the job
  retires. Per-job scratch (the tile state, the binner's overflow BO) is
  allocated fresh per job (`v3d_job.c`, `tile_alloc`/`tile_state` BOs), so
  two jobs in the queue never share it.

## 3. What the one-in-flight wait protects here

1. **The OSAL's single job state.** One `s.if_fence` and one `inflight` flag;
   one tile-state array zeroed at every submit
   (`zephyr_v3d.c:1236`, "the previous job has finished with it"); the
   binner's overflow pool, the TMU/L2T clean after FRDONE
   (`v3d_hw.c:259`), the fault record of the last job. A second job started
   before the first has finished would share and overwrite all of them.
2. **The arenas.** A job's bin, render and indirect lists and its data and
   texture arenas are two sets (`arenas`, `spare`, `choose_arenas`,
   `backend.rs:2411`). `busy` is true while the job in flight holds a set.
   With one job in flight at most one set is busy, so the other is always
   free; the code states this as an invariant ("the set it does not hold is
   free", `:2406`). With a queue of two, both sets would be busy at the
   second submit and the third job would wait on the CPU anyway, unless
   there is a third set or sets freed per job.
3. **Cross-job data hazards, which the hardware does not order.** There is
   no BO fence in this stack: a render job writes a target's storage and
   its render image, and a later job's TMU reads them. Only the CPU's
   seeing FRDONE and the cache clean makes that safe (`v3d-tfu-order.md`
   section 1). `tex_state_for` (`:3643`) already waits explicitly for the
   one case it can name (a sampled target's storage).
4. **The window image.** The swap's show of N-1 needs W(N-1) complete.
   With one in flight that is implied by W(N)'s submit. This is protected
   *indirectly* and is cheap to protect directly (section 4).
5. **Deferred frees** (`src/osal/v3d_frees.h`): memory freed while a job
   runs is held until it completes; the OSAL learns of completion only at
   `finish_inflight`. Those holds already work for "the job in flight",
   not for a queue of several.

The wait does not protect the frame's own pictures: the window job N is
never waited for by its own swap.

## 4. What would have to change to drop it

The proposal is a **queue of two jobs** (one running, one waiting), not a
general one. Order of work, and the files:

1. `src/osal/zephyr_v3d.c` (and `include/gles_osal.h` if a call is added):
   hold two job slots (`if_fence`, per-slot cycle counters, per-slot tile
   state and overflow pool), start the binner of job 2 when job 1's binner
   is done (the V3D runs bin and render of different jobs in parallel: the
   bin can overlap the other job's render), and start job 2's render only
   after job 1's FRDONE. `z_submit` must stop calling `finish_inflight`
   first; `finish_inflight` finishes the *oldest*. The TFU stays
   serialised behind both (`v3d-tfu-order.md`). `src/osal/linux.c` needs a
   model with the same two slots so the host can count.
2. `src/v3d/backend.rs`: `in_flight: Option<InFlight>` (`:1059`) becomes
   two slots; `flush_job` waits only when both are taken, and for the
   oldest (`:2363`); `choose_arenas` (`:2411`) needs a set for each job in
   the queue plus the open one (three sets, or the 8 MiB set sized down;
   the carve-out cost is the open question, see `:83`); `v3d_wait_kick`
   (`:5430`) and `v3d_finish_previous` (`:5448`) look at the right slot.
   `v3d_finish_previous` stops being a no-op: it must wait for the *older*
   of two in flight, which is W(N-1), so that the show of N-1 is safe.
3. **The hazards of 3 above.** Keeping two jobs apart is only correct
   if a job that reads what the one before writes is not started early.
   Pick one: (a) **per-job read/write sets**: the back end already knows,
   per job, its target (`g.j.target`, `rt_images`), the textures it
   samples (`tex_state_for`) and its `ZsKnown` history; a job that samples
   a target written by the job in flight waits (for that job only), as
   `:3643` does today, so a window pass that samples nothing from A(N) —
   for instance the first draws of a desktop frame — queues without a wait,
   while shadow and refract wait at the sampling draw instead of the swap;
   or (b) a **render-after-render order** in the OSAL (job 2's render
   starts after job 1's FRDONE and cache clean, always), which makes every
   case safe at the cost of GPU overlap between bin and render only. (b)
   is the first step; (a) can follow.
4. `src/gl/egl.rs`: nothing for the swap itself (it already asks for
   `finish_previous`). The depth resolve in `src/gl/texture.rs` and the
   mipmap wait stay as they are; they are other items.

What this does **not** remove: the wait of a sampling draw for the pass
that rendered its texture (with (b) that wait is hidden on the GPU; with
(a) it is still a CPU wait at the draw), the depth resolve, and the TFU
mipmap wait.

## 5. Expected gain per scene

The board table (`pi4-glmark2-cpu.md`, "On the board", main `dcbfb30b`) gives
the sum of GL-call and swap time, not the GPU's. What bounds the gain is
the GPU time per job, which the table does not give directly. A one-pass
scene with next to no CPU work (texture, shading, effect2d: gl 0.02 to
0.07 ms) shows what one 800x600 window job costs the frame: **swap 3.6 to
4.4 ms** (this includes the copy of the window into the firmware
framebuffer, 800x600x4 bytes, which `egl-swap-timeline.md` puts inside the
swap). So a job is about 3.5 ms of frame time.

| scene | frame ms | gl ms | swap ms | jobs a frame | frame at "N jobs x 3.5 ms" | gain bound |
|---|---:|---:|---:|---|---:|---|
| shadow | 7.30 | 0.97 | 6.33 | 2 (depth, window) | about 7 | **about 0** (already at the GPU floor); at most the 0.97 of CPU |
| desktop shadow | 6.82 | 0.83 | 5.98 | 2 | about 7 | **about 0** |
| desktop blur | 13.03 | 7.11 | 5.91 | several blur passes | unknown | up to the 7.11 ms CPU share, if the blur passes are GPU-light; **not estimable** without the pass count and GPU times |
| refract | 18.77 | 11.59 | 7.17 | 3 + a TFU job | about 11 | 11.59 of GL time is mostly the TFU wait and spin (item `v3d-tfu-mipmap-nowait`), not this wait; **0 to 2 ms from the queue**, the rest from that item |
| terrain | 178.94 | 173.94 | 4.88 | 5 TFU waits | - | **none from the queue**: 174 ms is not this wait (item `pi4-terrain-uniforms-study`) |

Reading: shadow and refract's swap columns of 6.3 and 7.2 ms look like
a wait, and they are one, but the wait is for GPU work that has to run
anyway. A job's frame time (about 3.5 ms) times the job count is already
about the frame, so a deeper queue lets the CPU run ahead without making the
GPU faster. The queue removes the *gaps*: after A(N) ends the GPU idles
until the CPU returns from `wait`, runs `v3d_job_end`, submits and the
binner of W(N) runs (a bin phase before its render). That is tens of
microseconds to a fraction of a millisecond a job on this CPU, so **about
0.2 to 0.5 ms a multi-job frame (3 to 7%) is the realistic gain**, an upper
bound of the CPU time per frame (the gl column) where the GPU is idle
waiting for the CPU. It is not "the swap column", and the 6 ms in it should
not be expected back.

This is an estimate from the table. Two things decide it on the board and
the host cannot show either: the GPU time of each job (the OSAL keeps
`last_job_cyc` per job, `zephyr_v3d.c`, which a board session can print per
pass), and whether the V3D overlaps the bin phase of job 2 with the render
of job 1 as hoped.

## 6. Recommendation

1. Do not build the queue for the glmark2 numbers: the expected gain is a
   few percent on the multi-pass scenes and zero on shadow, against a
   rewrite of the OSAL job state and the arena sets.
2. Before any change, have the next Pi 4 board session print the GPU time
   of every job in a multi-pass scene (`last_job_cyc.total`) and the CPU
   idle on the GPU between them; if the gaps are over 1 ms a frame, the
   queue is worth it.
3. The mipmap and depth-resolve waits are separate items and are each
   worth more than this one.
