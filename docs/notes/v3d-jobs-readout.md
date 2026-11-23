<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: reading the `glmark2-jobs` line (fields, the `~` mark, 23.49 vs 27.26 ms)

Plan #329 / #1209. Read from the code at `822d8ca9`; no board run was made
for this note.

The line is printed by `print_jobs`
(`driver/samples/zephyr/glmark2/src/main.cpp`, ~1405) from
`osal_v3d_recent_jobs_us` (`src/osal/zephyr_v3d.c:786`), which copies the
ring `recent_cyc` (16 entries, `OSAL_V3D_RECENT_JOBS`,
`src/osal/zephyr_v3d.h:117`) and converts cycles to microseconds
(`k_cyc_to_us_floor32`, rounded down). The line shows the jobs the swap of
frame 10 completed (the last `n` of the ring, `main.cpp:630-645`), so it can
carry the previous frame's last job.

## Fields (all in us, per job; `printk("total %u render %u[~] idle %u [tfu K/Tus] bin %u prep %u")`)

| field | start .. end | written at |
|---|---|---|
| `total` | job's CPU start `s.if_c0` (the submit call) .. `c_end`, the GPU's last work for the job: after FRDONE, the cache clean and the TFU jobs queued behind it (`tfu_queue_run`) | `finish_inflight`, `last_job_cyc.total = c_end - s.if_c0` |
| `render` | `s.if_c_render` (the render submit, taken in the FLDONE path, `zephyr_v3d.c:1047`) .. `c_frdone`, the moment `finish_inflight` saw FRDONE | `finish_inflight`, `last_job_cyc.render = c_frdone - s.if_c_render` |
| `idle` | `recent_prev_end` (end of the previous job's GPU work: its `c_end`, or the end of any TFU job run since, `tfu_now`) .. `s.if_c_bin` (this job's bin submit); 0 for the first job ever and when the stamps cross | `note_job_done` |
| `tfu K/Tus` | printed only when K != 0: K TFU jobs (`tfu_now`, one per glGenerateMipmap level chain step) run since the previous job completed, T = the sum of their run times. They are counted in `pend_tfu_*`, moved into the entry of the next job that completes | `tfu_now`, `note_job_done` |
| `bin` | bin submit `s.if_c_bin` .. the FLDONE interrupt `s.if_c_fldone` | `finish_inflight` (`last_job_cyc.bin`) |
| `prep` | CPU time from the submit call `c0` to the bin submit (`start_job`: the tile-state `memset` and setup) | `start_job` (`last_job_cyc.prep = c_bin - c0`) |

The ring is written only by `note_job_done`, called from `finish_inflight`
(the normal path and, at `zephyr_v3d.c:1209`, the failure path of a refused
pending job); `start_job` only resets `last_job_cyc`; the TFU path
(`tfu_now`, from `osal_v3d_tfu` directly or from the queue in `tfu_queue_run`)
feeds `pend_tfu_n/cyc` and moves `recent_prev_end`, never writing a ring entry
itself.

## The `~` mark

`~` is printed after `render` when `render_late` is 1. `finish_inflight`
sets it from one register read before it waits:
`INT_STS & FRDONE` already up when the OSAL first looks
(`zephyr_v3d.c:1275`). FRDONE has no interrupt on this path, so the render
ended at an unknown time before that look, and `c_frdone` is the look's time.
Therefore `render~` is **the render's start to the OSAL's look, an upper
bound that includes whatever the CPU did before asking** (e.g. the frame's
CPU work after the submit); the same holds for `clean`, `total`, and the
next job's `idle` (the real gap is longer, header comment of
`osal_v3d_recent_job_us`). Without `~` the OSAL waited inside `v3d_wait`, so
`render` is the render itself within the wait's poll. The sums in
`FrameCost` (`render_us`, `job_total_us`) exclude `render_late` jobs for the
same reason (`main.cpp:645-650`).

So `4394~` is a render the OSAL looked at late: 4.394 ms is not the GPU's
render time, only an upper bound.

## 23.49 vs 27.26 ms

The two sums are of different quantities. In the earlier images the frame-10
line's renders were read where the OSAL waited (the CPU blocks in `wait_job`
right after each submit, so `render` is close to the GPU's). At `e07d066a`
the dead window clear's job is gone (8.0 jobs a frame, `pi4-slow-terrain.md`),
so one render's end is now seen after other CPU work, and its entry is
marked `~`: its `render` then holds CPU time as well as GPU time. A `~`
entry makes the sum of `render` an upper bound; the gain from 23.49 to 27.26
(+3.77 ms) is not a GPU slowdown: the frame got 0.944 ms faster in the same
record, and the frame's GPU work stays about 29 ms from the `total` and `tfu`
fields.

What this note could not do: the record of `e07d066a` is not in this tree, so
the 3.77 ms was not split per entry. To confirm: on the board, read the
frame-10 line of `e07d066a` and drop every `~` entry from the sum, or take
`total` of the non-late entries, and compare with 23.49 for the same job
count. If they agree the difference is the `~` entry; if not, one job's
`render` really grew and `bin`/`idle` of the entries show where.

**Rule for the notes**: never add up `render` over entries with `~`; use
`total`, or report the number of `~` entries beside the sum.
