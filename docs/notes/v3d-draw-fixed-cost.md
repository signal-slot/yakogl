<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Where the V3D back end's time a glDrawElements goes (host, attributed)

Row F5 of [gl-draw-fixed-cost.md](gl-draw-fixed-cost.md) left the V3D back
end's ~6 us a draw (glmark2 `ideas`) unattributed. This note attributes it to
named steps, with the steps' costs for `ideas`, `buffer` and `terrain`, says
what Mesa's v3d does for the same work, and ranks the fixes. Measurement
only: nothing here changed the library.

## Result in short

* **99% of the back end's time on `ideas` is in named steps** (quiet run:
  4.69 of 4.75 us a draw).
* `v3d_job_draw` (the C job builder, `driver/src/v3d/v3d_job.c`) is **62%**
  (about 3 us). Inside it, no step is alone: the VS and CS uniform streams
  (0.54 + 0.52 us), the shader-state record (0.58), the attribute records and
  primitive packets (0.56) and `emit_state` (0.44) are each 15 to 20% of it.
* **`check_program` is 22%** (1.1 us on the quiet run, 1.7 to 2.2 us on the
  loaded ones): it hashes the program's whole code at every draw to look it
  up in `prog_ok`. This corrects the "suspect that did not hold" paragraph of
  gl-draw-fixed-cost.md (see "check_program" below).
* `texture_units` 5%, `state::job_state` 2%, `flush_job` 2%, `programs` 1%.
  The candidates the earlier note named first (`job_state`, `programs`) are
  not where the time is.
* A frame of `ideas` is **one job** of ~180 to 220 draws, so the submit is
  0.1 to 0.2 us a draw. `buffer` and `terrain` draw **once a job**: there the
  job's open, its first-draw state and its submit are the cost, not the
  per-draw steps (second table).

## Method

* Commit measured: main `41e95169` (`hostharness: compare.py ...`), glmark2
  `22c527cb0556f3a1ac4445aaa52cc532760928d5`, V3D host build, `run.sh` as the
  harness README gives it (`YAKOGL_V3D_TFU=1`, the Pi 4's carve-out).
* **perf was not usable**: `kernel.perf_event_paranoid` is 4 on this host, and
  `GM2_PERF=1` fails with `Failure to open event 'cpu-clock:u'`. So the second
  method of the task was used: **clock laps in a local, uncommitted build**.
  `rdtsc` laps (about 20 cycles each, converted with a `clock_gettime`
  calibration) around `state::refusal` to `extra_targets`, `job_state`, the
  rest of `draw_call`, `begin_job`, `flush_job`, `programs`, `check_program`,
  `texture_units` (FS and VS/CS apart), the attribute and index preparation,
  `v3d_job_draw`, and, inside `v3d_job_draw`, `draw_is_supported`, `emit_state`,
  each of the three `write_uniforms`, the shader-state record, the attribute
  records plus primitive packets, and the pack-key copy. The totals were
  printed at every 10th job submit as a cumulative line; the figures are the
  difference between the first and the last line (so first-use and setup
  draws are out). All of it was reverted; `git status` was clean before this
  note was written.
* The OSAL clock was refused (`GM2_NOCLK`, a local patch of
  `OSAL_QUERY_CLOCK_NS`), so the GL core's six `draw_steps` laps cost nothing,
  as in the 6.3 us column of gl-draw-fixed-cost.md.
* `GM2_PROFILE=1 GM2_FRAMES=100`, entries `ideas:speed=duration`, `terrain`
  and `buffer:...:update-method=map:interleave=false` (the first of the three
  buffer entries). `ideas` and `buffer` ran 99 counted frames; `terrain` 68.
* Repeats: 3 runs of all three scenes, plus 3 runs of `ideas` alone (and 3 of
  `ideas` with `check_program` skipped, below): 6 runs of `ideas` with the
  step in place. **The host was loaded the whole time** (load average 13 to
  33 on 12 cores). Per `FINDINGS-variance.md` the minimum is the figure to
  trust, so the tables give the **quiet run** (the lowest total) and the
  range. Shares inside one run hold better than milliseconds across runs
  (every lap of a run saw the same load).

## Us a draw, by step (host)

`ideas:speed=duration`, 1 job a frame, 180 draws a job in the quiet run (193
to 223 in the loaded ones), 16,200 draws measured in the quiet run.

| Step | quiet run, us a draw | share | range over the 6 runs |
|---|---|---|---|
| **`v3d_job_draw`** (C, total) | 2.96 | 62% | 2.96 to 7.02 |
| &nbsp;&nbsp;`write_uniforms` VS | 0.54 | 11% | |
| &nbsp;&nbsp;`write_uniforms` CS | 0.52 | 11% | |
| &nbsp;&nbsp;GL_SHADER_STATE_RECORD | 0.58 | 12% | |
| &nbsp;&nbsp;attribute records + primitive packets | 0.56 | 12% | |
| &nbsp;&nbsp;`emit_state` (packets skipped when unchanged) | 0.44 | 9% | |
| &nbsp;&nbsp;`write_uniforms` FS | 0.10 | 2% | |
| &nbsp;&nbsp;`draw_is_supported`, pack-key copy | 0.10 | 2% | |
| **`check_program`** | 1.06 | 22% | 1.06 to 2.21 |
| `texture_units` (VS+CS 0.21, FS 0.04) | 0.25 | 5% | 0.25 to 0.56 |
| `state::job_state` | 0.09 | 2% | 0.09 to 0.16 |
| `flush_job` (90 submits, 16 us each) | 0.09 | 2% | 0.09 to 0.23 (16 to 48 us a submit) |
| `programs` | 0.04 | 1% | 0.04 to 0.08 |
| attribute and index preparation | 0.03 | 1% | 0.03 to 0.10 |
| the rest of `draw_call`, `record`, after `v3d_job_draw` | 0.17 | 4% | |
| **back end total (`v3d_draw`)** | **4.75** | | 4.75 to 10.39 |
| attributed to the named steps | 4.69 | **99%** | |
| `glDrawElements` wall (profile) | 7.02 | | 7.02 to 14.38 |

The 2.3 us between the back end (4.75) and `glDrawElements`' wall time
(7.02) is outside the back end: the GL core (about 0.2 us, per
gl-draw-fixed-cost.md), the harness's call wrapper and the profile clock
reads. It was not split further.

The loaded runs agree on the order and the shares (e.g. 8.26 us total:
`v3d_job_draw` 5.12 = 62%, `check_program` 1.77 = 21%, `texture_units` 0.46,
`flush_job` 0.19, `job_state` 0.16). Their absolute figures are 1.7 to 2.2
times the quiet run's.

`buffer` (update-method=map) and `terrain`: **one draw a job**, so every job
pays its open, the whole first-draw state (nothing to skip) and its submit.
Best (lowest-total) run of three, us **a draw**, which is also a job; 90
jobs of `buffer`, 860 `v3d_draw` calls and 765 `v3d_job_draw` calls of
`terrain` (9 jobs a frame, `gm2: v3d-frames`):

| Step | `buffer` | `terrain` | what it is |
|---|---|---|---|
| `v3d_job_draw` (C, total) | 13.9 | 12.6 | |
| &nbsp;&nbsp;`emit_state` | 6.8 | 6.5 | every state packet of a job's first draw |
| &nbsp;&nbsp;shader record + attribute records | 4.5 | 3.2 | |
| &nbsp;&nbsp;`write_uniforms` (3) | 1.6 | 2.1 | |
| `flush_job` (the submit) | 24.9 | 15.9 | host OSAL model: see below |
| checks after `job_state`, up to the draw | 7.1 | 8.4 | **inferred**: this stretch holds `begin_job`, which opens the frame's job on the TLB-clear path (`begin_job` was lapped only inside `draw_in_job`); not lapped on its own |
| `texture_units` FS | 0.3 | **5.3** | `tex_state_for` for terrain's sampled textures (tiled copies, TFU); one draw in terrain pays it |
| `check_program` | 1.6 | 1.6 | |
| `job_state`, `programs`, `texture_units` VS/CS, attribute prep | 1.4 | 1.3 | |
| **`v3d_draw` total** | **25.5** | **32.3** | the submit is not inside this total except when a draw closes a job |

(`flush_job`'s time is mostly at `eglSwapBuffers`, outside `v3d_draw`; it is
listed because it is the largest step of a one-draw job.) Run-to-run the
terrain total ranged 32 to 60 us and buffer 19 to 44 us under the load.

## What is host-model artefact, and what shows on the Pi 4

* **Submit (`flush_job`): 16 to 48 us a job here.** The harness's
  `submit()` (`src/osal/linux.c`, "the V3D") checks the kick (magic, lists,
  tile memory inside GPU memory) and zeroes the tile state, and completes the
  job at once. On the Pi 4 a submit is an ioctl to the kernel driver and the
  job runs on the GPU, so the cost is different, not smaller; what is
  specific to the host is the zeroing and list checks. For `ideas` it is 2%
  of the back end, so it does not matter for this note's question; for
  `buffer` and `terrain` (one job a draw) it is the largest line and the
  host figure should not be read as the board's.
* **The host `memset` in `l_gpu_alloc`** is not in the per-draw path of
  `ideas`: a draw allocates nothing on the GPU side (the arenas are taken at
  init and at job open). It may sit inside terrain's `texture_units` FS
  (5.3 us), whose tiled copies allocate; that was not separated, so it is
  *suspected, not shown*.
* **The step timer** (14.7 vs 6.3 us in gl-draw-fixed-cost.md) is out of
  these figures (clock refused).
* **Memory.** `v3d_job_draw` writes the draw's command-list and uniform words
  to the job arenas. On the host these are malloc'd, cached memory; on the
  Pi 4 the arenas are GPU memory the CPU maps write-combined or non-cached, so
  the same stores cost more there. `check_program` reads the cached CPU copy
  of the code (`ShcShader::usc`) and scales with the CPU only. So the board
  ranking should put `v3d_job_draw` (and any fix that writes fewer words)
  higher than here and `check_program` no higher. To be confirmed by a board
  run; none was done.
* **The rdtsc laps** cost about 13 x 20 cycles a draw (about 0.1 us); they are
  in the figures.

## check_program

`v3d/backend.rs` `check_program` (called from `record`) folds an FNV hash
over **the whole uploaded code of the linked program** at every draw, to
build the `prog_ok` key `(usc address, length, sum)`; only after a hit does it
skip the decode of the three stages. The hash runs one dependent multiply a
word. gl-draw-fixed-cost.md ("A suspect that did not hold") measured no gain
from returning at once. This session's laps put it at 1.06 us a draw on the
quiet run (22%) and 1.6 to 2.2 us on the loaded ones, in six `ideas` runs and
two other scenes (1.6 to 2.8 us).

A direct A/B (the same build with `check_program` skipped through a local
environment switch, 3 runs of each, alternating) did **not** settle it: the
host's load moved the totals by more than the step (back end total with the
check: 9.37, 4.75, 10.39 us; skipped: 7.43, 8.33, 5.99 us; the lowest of each
is 4.75 vs 5.99, i.e. noise). The lap is the in-run measurement; the earlier
A/B and this one both sit inside this host's 5 to 100% run-to-run spread, so
the earlier "no gain" is not evidence that the step is free. The fix below
should be judged on an idle host with 10+ alternated repeats (the
`compare.py` table).

## What Mesa's v3d does for the same work

Read in Mesa 26.2.1, `src/gallium/drivers/v3d/`:

* **Dirty flags** (`v3d_context.h:57-89`, `V3D_DIRTY_*`): `v3dx_emit.c`
  `v3dx_emit_state` writes a state packet only if its dirty bit is set
  (e.g. 230, 294, 407, 525, 587, 649). `v3d_program.c`
  `v3d_update_compiled_fs` (582) and the VS/CS/GS updates return at once unless
  one of the bits they depend on is set (`V3D_DIRTY_PRIM_MODE | BLEND |
  FRAMEBUFFER | ZSA | OQ | RASTERIZER | SAMPLE_STATE | FRAGTEX |
  UNCOMPILED_FS`): a draw that changes none of them does no program lookup
  at all. The repository has the equivalent for the *packets* (`PACK` rows
  in `v3d_job.c`, `emit_state`), but not for the program: `programs()`,
  `check_program()` and `texture_units()` run at every draw.
* **The compiled program is cached on the program object**, found through a
  key hash and `_mesa_hash_table_search` (`v3d_program.c:472`) and only
  when a dirty bit says the key may have changed. Nothing hashes the shader
  code per draw: the code is checked once when compiled.
* **Uniform streams are *not* reused**: `v3dx_draw.c:697-715` calls
  `v3d_write_uniforms` for the FS, VS and CS at every draw, and
  `v3d_uniforms.c` (236-255) reserves `count + 1` words in `job->indirect`
  and fills them word by word. So the repository's three-stream write per
  draw (`write_uniforms`) is what Mesa does; any reuse is beyond Mesa.
* The **shader-state record** is rebuilt at every draw in Mesa too
  (`v3dx_draw.c` `emit_shader_state_record`, ~553-682), and so are the
  attribute records (798-864). Mesa packs them with the same field-by-field
  `cl_emit` macros as the repository's `v3d_cl_emit` (this code is a port).
  The compiler-side facts (`vattr_sizes`, VPM sizes) are in the compiled
  program, so Mesa also reads them from there.
* `v3d_start_draw` (119) and `v3d_flush_jobs_writing_resource` (152-195)
  check each sampled texture and vertex buffer for a pending writer at every
  draw; the repository's `drop_stale_copies` and `stamp`s do the same sort of
  work and measured small here (`pre`: 0.03 us).
* **No timer, no code hash** in Mesa's draw path.

## Ranked fixes (host us a draw; `ideas` unless stated)

| # | Fix | Where | Expected gain a draw | Risk and test |
|---|---|---|---|---|
| V1 | **Check a program's code once per link, not once per draw.** Key `prog_ok` by something the GL core bumps at link (a link serial in `ShcShader`, or the pointer plus a serial), never by a hash of the code; keep the decode and uniform-count check for the first draw after a link. | `src/v3d/backend.rs` `check_program` (and `ProgKey`, `prog_ok`); a serial field in `src/shaderc` / `src/gl/program` | **1.1 us (22% of the back end)** on the quiet run, 1.7 to 2.2 on the loaded ones | Low to medium. A relink or a `glProgramBinary` that reuses the same address must change the key; a test that relinks a different program into the same code buffer and expects the refusal of a bad stream. |
| V2 | **Reuse an unchanged uniform stream within a job.** Keep the last (FS, VS, CS) stream's address with the uniform words' identity (the host arrays, the viewport, the TMU config); on an equal draw reuse the address and skip the write. | `src/v3d/backend.rs` `record_arrays_and_draw`; `driver/src/v3d/v3d_job.c` `write_uniforms` | up to the 1.16 us of the three streams, about 0.6 to 0.8 us in `ideas` (the scene updates ~54 matrices for ~193 draws, so most draws reuse); none for one-draw jobs | Medium. Beyond Mesa. The MMU-fault record keeps the stream list (`g.j.streams`); a stream reused must stay inside the job. Golden tests of the job lists (`tests/v3d_*`). |
| V3 | **Make a program's invariants per link, not per draw**: the `ret32` unit mask and the unit list that `texture_units` rebuilds by scanning the VS/CS/FS streams (0.25 us), `programs()` (0.04), and the fixed fields of the shader-state record (`v3d_fs_prog`, the 20 or so fields that come from the program alone). | `src/v3d/backend.rs` `texture_units`, `programs`; `v3d_job.c` `record_draw` | 0.25 us (`texture_units`) + 0.04 + 0.2 to 0.3 of the record's 0.58 | Low for the scans, medium for a record template (the packed record must still be golden-equal). |
| V4 | **Pack the shader-state and attribute records from a per-program template** and patch the three uniform addresses and the attribute addresses, strides. | `driver/src/v3d/v3d_job.c` `record_draw` | 0.3 to 0.6 us of the 1.14 us the two packers cost | Medium: the generated pack functions (`driver/src/v3d/generated/`) must not be edited; build the template with them once. Golden streams. |
| V5 | **No CS uniform stream when it equals the VS stream's words** (a coordinate shader is the vertex shader with only position outputs, and the two often read the same uniforms). | `v3d_job.c` `write_uniforms` call for the CS | 0.1 to 0.5 us (the CS stream is 0.52 us here, as dear as the VS) | Medium: only if the stream *contents* match, including the P0/P1 words; Mesa writes both. Subsumed by V2 where V2 applies. |
| - | `state::job_state` (0.09), `programs` (0.04), `flush_job` on `ideas` (0.09), the attribute and index preparation (0.03): each under 2%, none worth a change for this scene. | | | |

Not for this scene but shown by the same laps (one draw a job):
* `texture_units` FS is 5.3 us of terrain's 32 us a draw, the largest
  single step there outside the job open; its source (`tex_state_for`, kept
  tiled copies, the TFU path) belongs with the tiled-copy items, not to this
  per-draw question.
* A job's first draw (`emit_state` 6.5 to 6.8 us) cannot skip a packet: the
  fix there is fewer jobs (the swap-wait and render-target items), not
  cheaper packets.

## What this does not show

* The boards. The ranking carries over, the microseconds do not (see the
  artefact list above). The `v3d_job_draw` share is probably higher on the
  Pi 4 (non-cached stores).
* An idle host. All figures were taken at a load average of 13 to 33 on 12
  cores; the quiet run is the lowest of 6, not a measured idle machine.
  The step shares are the dependable part.
* The A/B of V1. See "check_program".
* `begin_job` on its own and the `l_gpu_alloc` memset in terrain's
  `texture_units`: inferred, not lapped.
* The 2.3 us between the back end and `glDrawElements`' wall time.
