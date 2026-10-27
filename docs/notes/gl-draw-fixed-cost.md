<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The fixed cost of a glDrawElements (glmark2 `ideas:speed=duration`)

Where the microseconds of each `glDrawElements` of glmark2
`ideas:speed=duration` go on the host harness, per step and back end, and
which fix to build first. Host microseconds, never the boards' (the A53 and
A72 are several times slower and write command streams to non-cacheable
memory). The ranking carries over; the figures do not.

## Result in short

* The GL core (`src/gl/draw.rs`) is **not** where the fixed cost is: all its
  steps together are about 0.2 us a draw (under 0.6 us), on every back end.
* **On the V3D host build most of the figure is the step timer.** The
  harness's Linux OSAL answers the clock query for the V3D only
  (`src/osal/linux.c:1136`), so only there every `draw_steps` lap is a real
  `clock_gettime`: 14.7 us a draw with it, 6.3 us without. The V3D findings
  (`FINDINGS-v3d-allscenes.md`, "about 9 us each") include it.
* The rest is the back end: V3D about 6 us, PowerVR about 4 us (this host,
  loaded), GC7000 about 2.7 us. On the GC7000 the one step that can be named
  is `emit_draw`: 1.1 us a draw.

## What was run

* YakoGL `ad1724cc` (main when the item started), glmark2 `22c527cb0556`
  (`22c527cb0556f3a1ac4445aaa52cc532760928d5`), one process per scene,
  `ideas:speed=duration`, 200 frames (199 counted), x86-64 host, shared with
  other jobs (the same scene on the PowerVR measures 4.5 us a draw today and
  1.9 us in `FINDINGS-pvr-allscenes.md` at `b9130d64d878`: read ratios, not
  absolute figures; repeat runs of one build differ by about 5%).

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=200 GM2_ENTRIES='^ideas:speed=duration$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-draw powervr vivante v3d
```

* The scene makes 180 to 202 `glDrawElements` a frame (it varies with the
  animation), all small triangle strips and fans from element buffers over
  float vertex buffers, no texture (`scene-ideas/{table,logo,lamp}.cc`,
  `characters.h`). Every draw takes the cheapest GL core path (indices and
  attributes in place, `draw.rs:4368`, `3633`).
* The per-step laps are the library's own (`src/draw_steps.rs`). The host
  OSAL discards them, so the numbers below come from **throwaway local
  patches, reverted, not committed**: `osal_linux_query` made to add up
  `OSAL_QUERY_DRAW_STATS` and print the totals at exit (and to answer the
  clock for the PowerVR and GC7000 as the V3D's already does), `GM2_NOCLK`
  to refuse the clock, and a run with `check_program` returning at once.
  Making that permanent is item F0.

## Per-draw cost, measured (host us a draw)

`glDrawElements` wall time a call, from `profile-<gpu>.md`:

| Back end | clock answered by the OSAL (default) | clock refused (no laps) | with all laps timed |
|---|---|---|---|
| PowerVR | 4.5 (0.840 ms / 185.1) | the same: the host OSAL has no clock | 16.5 (3.273 / 197.8) |
| GC7000 | 2.9 (0.529 / 180.5) | the same | 33.8 (6.836 / 202.2) |
| V3D | **14.7** (2.889 / 196.4) | **6.3** (1.210 / 192.1) | 14.5 (second run) |

So a lap costs about 1.4 to 1.5 us on this host, and the V3D default includes
6 reads and a report a draw: 8.4 us of 14.7.

The GL core's steps, summed over the 43,000 to 44,000 draws of the run and
divided (the V3D run; the PowerVR and GC7000 runs agree to 5%), with the
lap cost taken out:

| Step | laps | us a draw as lapped | less laps (1.45 us each) |
|---|---|---|---|
| GlProgram (`draw.rs:3265-3430`) | 1 | 1.47 | 0.0 |
| GlUniforms (`3441-3535`) | 1 | 1.46 | 0.0 |
| GlAttribs (`3537-3779`) | 1 | 1.48 | 0.0 |
| GlTextures (`3267`, `3781-3998`) | 2 | 2.83 | 0.0 |
| GlState (`4000-4055`) | 1 | 1.40 | 0.0 |
| **GL core, 6 laps** | | 8.64 | **0.2** (the 8.4 us of laps leaves 0.24; the lap cost is known to about 0.1 us, so under 0.6) |

Each step costs the same as a clock read, within the error: **no GL core step
is measurable here**, including the `gl_DepthRange` fill and the program,
attribute and state steps. The estimates this note first carried for them
(0.4 to 0.9 us for the uniform fill) were wrong and are withdrawn.

The GC7000 back end (`vivante/backend.rs`, laps at 2553-2886), 44,178 draws:

| Step | laps | as lapped | less laps (1.50 us each) |
|---|---|---|---|
| GcRest | 3 | 4.57 | 0.07 |
| GcDs (`ds_cache::select`, 2554) | 1 | 1.65 | 0.15 |
| GcTiledCopy (2597) | 1 | 1.53 | 0.03 |
| GcFrame (2631) | 1 | 1.51 | 0.01 |
| GcUniforms (`take_uniforms`, `ubo_copy`) | 2 | 3.04 | 0.04 |
| GcVertex (`build_vertex_input`) | 2 | 3.00 | 0.00 |
| **GcEmit (`emit_draw`, 1903-1937)** | 2 | 4.13 | **1.1** |
| laps total | 12 | | 1.4 |

So of the GC7000's 2.9 us, the GL core is 0.2, `emit_draw` 1.1, the other
laps 0.3 together, and about 1.3 us is outside every lap: `gc_draw`, the
GL core's `submit_draw` and `restamp_draw` (`draw.rs:576`, `525`), the call
into the back end, and the harness's own call wrapper (`shim.c`).

The PowerVR and V3D back ends have no `Step` laps. Their totals above minus
the GL core's 0.2 us are the back end and everything around it: about 4 us
(PowerVR, loaded host) and about 6 us (V3D).

**A suspect that did not hold.** The V3D `check_program` hashes the whole
uploaded code at every draw (`v3d/backend.rs:3065-3090`, called at `3204`).
With it returning at once the run measured 15.3 us a draw against 14.7 us:
no gain outside the 5% repeat error. It is not the cost.

## What is left inside the back ends (not measured)

Read from the code; the order below is by what is repeated per draw, not by
a measurement.

* **GC7000 `emit_draw`** (1.1 us): about 120 `state()` writes a draw, none
  skipped when equal to the previous draw's: `emit_raster_state` (1269),
  `emit_ra_state` (1726), `emit_msaa_state` (808), `emit_pe_state` (1404),
  `shader::emit_shader_state` (`vivante/shader.rs:166-380`, which rebuilds the
  `vs_input`, `vs_output` and `interp` tables from the shaders and elements),
  `emit_shader_code` (380), `emit_uniforms` (573), `emit_flushes` (1746),
  the stalls (1926, 1933).
* **PowerVR `record_draw`** (`pvr/backend.rs:6033-`): the vertex-fetch PDS
  generator (6179), the kick-USC PDS generator (6381) and the coefficient
  PDS (6412-) run at every draw, the middle one for inputs that are the same
  for every draw of the program.
* **V3D** (about 6 us, the largest): `state::job_state` (`v3d/state.rs:528`)
  rebuilds the state structs, `programs` (3096) and `texture_units` (4732,
  twice) scan the uniform streams, `v3d_job_draw` (`driver/src/v3d/v3d_job.c:950`)
  writes three uniform streams (`write_uniforms`, 354) and the shader state
  and attribute records. Its state packets are already skipped when
  unchanged (the `PACK` key rows). Jobs: a draw that closes a job pays the
  submit (`flush_job`), which the host OSAL model makes dear (list checks,
  tile state zeroed); a frame of ~200 draws crosses job boundaries. No lap
  or counter separates these, so the V3D's 6 us is not attributed.

## What Mesa does for the same work

Not checked against a Mesa tree in this session (none was at hand); this is
the design as the repository's comments cite it (`v3dx_emit.c` in
`driver/src/v3d/v3d_job.c`, etnaviv in `src/vivante/`), to confirm in
`$MESA_V3D_TREE` before building.

* Gallium drivers keep a `dirty` mask: `v3dx_emit.c` `emit_state` writes a
  packet only if its `V3D_DIRTY_*` bit is set, `etnaviv_emit.c`
  `etna_emit_state` likewise with `ETNA_DIRTY_*`. The V3D back end here has
  the equivalent (`PACK` rows); the GC7000 back end does not.
* The compiled program, its variant and the driver's derived state are
  made at bind or link and re-used on a pointer compare.
* There is no timer in the draw path.

## Ranked fixes (expected gain a draw, host; the first is measured)

| # | Fix | Where | Expected gain a draw | Risk and test |
|---|---|---|---|---|
| F0 | **Make the step laps visible and cheap on the host.** Let `osal_linux_query` add up `OSAL_QUERY_DRAW_STATS`, print a `gm2: draw-steps` line (`shim.c`, `report.py`), and answer the clock for the PowerVR and GC7000 too; and take no lap at all when no totals are wanted (below). This is the measuring tool every later fix needs. | `src/osal/linux.c:1081-1146`; `tools/hostharness/glmark2/shim.c` | None itself; makes F1 to F5 measurable | Low. |
| F1 | **No step laps unless the OSAL keeps the totals.** `Timer::start` asks once whether the OSAL keeps draw-step totals (the first `report_draw_stats` that answers `-ENOTSUP` clears a flag; the Zephyr OSAL that keeps them, `zephyr_timing.c`, leaves it set) and takes no clock read or report while it is clear. | `src/draw_steps.rs:108-170`; laps `src/gl/draw.rs:3265-4055` (6 reads), `src/vivante/backend.rs:2485-2886` (12) | **Measured: 8.4 us of the V3D host build's 14.7 us** (6 reads a draw at 1.4 us); on the PowerVR and GC7000 host builds none (no clock answered); on a board whichever one `clock_ns` costs there: the Zephyr `k_cycle_get` answer is cheap, to be measured by item F0 on the board | Low. The fake-clock tests of `draw_steps` (`draw.rs:4926`, `gl.set_clock`) keep passing if installing a clock sets the flag. This also corrects every earlier V3D host ms-a-frame (`FINDINGS-v3d-allscenes.md`, `-refract-budget.md`, the TFU tables): all are over by about 1.4 us a lap-read. |
| F2 | **Done (see "F2 done"). GC7000: emit a state group only when it differs from the frame's last draw.** Keep each group's last words in the open frame (as the V3D keeps `PACK` rows), cleared at `frame_open`/`emit_context_init` (`vivante/backend.rs:1942`); skip equal groups: raster, RA, MSAA, PE, shader state, shader code, prefetch. | `src/vivante/backend.rs:1903-1937` and the emitters above; `src/vivante/shader.rs:166`, `380`, `418` | up to **1.1 us** of the 1.1 us `emit_draw` step (about 40% of the back end's 2.9 us); more on the board (non-cacheable stores, shorter stream for the front end) | Medium to high: `emit_prefetch`'s constant-file steering and the stalls fix an order (`shader.rs:398-417`); the golden streams of `tools/vivante/cs_golden.c` must stay equal on a first draw. |
| F3 | **GC7000: build `emit_shader_state`'s tables once per (program, element layout).** | `src/vivante/shader.rs:166-380` | part of F2's 1.1 us (est. 0.2-0.4) | Medium; subsumed by F2 for a draw equal to the last. |
| F4 | **PowerVR: the kick-USC and coefficient PDS programs once per (program, variant, sample rate).** Keep their `pds_put` offsets in `ProgGpu`; only the vertex-fetch data is per draw. | `src/pvr/backend.rs:6373-6560`, `6179` | est. 0.2-0.6 us of the PowerVR's ~1.7-4 us; unmeasured (no laps in this back end) | Medium: the PDS arena is a per-frame bump (`pds_put`), a cached block must outlive the frame. Needs F0 laps in `record_draw` first. |
| F5 | **V3D: attribute the 6 us before changing anything.** Add laps (or `GM2_PERF=1` where `perf` is allowed) around `state::job_state`, `programs`, `texture_units`, `v3d_job_draw` and `flush_job`, and count jobs a frame (`gm2: v3d-frames`). | `src/v3d/backend.rs:5380-5582`, `3191`, `4891` | unknown; this is the largest back end cost (6 of 6.3 us) | None for the measurement. `src/v3d/**` is owned by the lazy-raster item until it merges. |
| - | *Withdrawn:* a once-per-program `check_program` (`v3d/backend.rs:3065`): measured no gain. The GL core's `gl_DepthRange` fill (`draw.rs:2864`) and `GlesDraw::ZERO` (`3263`): below the resolution of this method (each step is one clock read). | | 0 | |

The GL core needs no work for this scene. The next runs belong to F0, then
F1 (to take the timer out of every V3D figure), then F2 and F5.

## F2 done

`src/vivante/dedup.rs`, called from `emit_draw` (`vivante/backend.rs`).
Each of the nine state groups (vertex elements, index, streams, raster,
RA, MSAA, PE, shader wiring, program = instruction ranges and constant
file) is written into a scratch buffer and copied into the stream only
when its words differ from the ones the open stream holds last for it.
The flushes, the stalls, the occlusion counter, the texture descriptors and
the two prefetches stay outside the groups and are written at every draw.
The kept words are forgotten at every new stream (`frame_open`,
`frame_submit`), a draw taken back (`Mark::restore`), the mipmap chain, the
depth resolve, and init/shutdown.

Writing a group's words costs more on the host than storing them (the
stream is cacheable memory here), so the skip on the words alone saved
nothing: 1.59 against 1.63 us a draw in a first run. The four costly
groups (raster, PE, shader, program; together about 0.35 us of a host draw,
measured by running each writer twice) carry a key, the inputs their writer
reads; an equal key skips the writer too. A key that missed an input would
skip words that differ, so a `host-test` build writes the words on every key
hit anyway and panics if they differ (`dedup::set_verify`): the whole
GC7000 host suite (110 test binaries) runs with that check and passes. The
first draw of a stream writes every group, so the golden first-draw streams
are unchanged; `state_multi_fixp` (`vivante/cmd.rs`) now checks the room once
a command instead of once a word.

Measured with `tests/gc_state_dedup.rs`
`time_ideas_shaped_draws_with_and_without_the_skip`: glDrawElements through
the GL core and this back end over a host-memory OSAL, in `ideas`' shape (190
four-index strips a frame from one element buffer at a new offset each,
uniforms changed every 19 draws), best frame of 200, best of 5 runs, release,
this host, shared with other jobs:

| Build | us a draw | stream words a frame |
|---|---|---|
| before (old writer, every group written) | 1.227 | 48,338 |
| new writer, every group written (`set_enabled(false)`) | 1.011 | 48,338 |
| **F2: new writer and the skip** | **0.850** | **7,226** |

So 0.38 us a draw less on the host (31%): 0.22 from the writer, 0.16 from
the skip. The stream is 6.7 times shorter, which the host hardly feels and
the board should (its command buffer is non-cacheable memory, and the front
end fetches every word); that gain is the board session's to measure.

The glmark2 host harness rows (`GM2_PROFILE=1 GM2_FRAMES=200`, `ideas`,
`terrain`, `shadow`, `refract`, before and after) were not run with this
change: the session that made it could not start `tools/hostharness/glmark2/run.sh`.
They are the next host measurement of F2:

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=200 GM2_ENTRIES='^(ideas:speed=duration|terrain|shadow|refract)$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-dedup vivante
```

F3 (the shader tables once per program and element layout) is subsumed for
a draw whose shader key equals the last one's.

## What this does not explain

* The GC7000's 1.3 us outside the laps, the PowerVR back end's split, and
  the V3D's 6 us (F5).
* The boards. A lap costs there whatever the Zephyr clock query costs, and a
  command-stream write is a non-cacheable store.
* glmark2's own work between the draws: it is not in `glDrawElements`; the
  profile gives it as "in glmark2 itself" (0.96 to 1.26 ms a frame).
