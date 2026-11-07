<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2 GM image: the per-scene counter line (design)

Design only. `driver/samples/zephyr/glmark2/src/main.cpp` belongs to plan
#328 for now, so nothing here is built. The need is in
`pi4-slow-terrain.md` ("Counters a board run needs"), `am62p-slow-terrain.md`
(same heading), `pvr-waited-kick.md` (step 3, "Print the counters") and
`v3d-wait-poll.md` (the "waits a frame" row).

## Where and how

One line per scene, printed right after `glmark2-steps:` in the code that
prints it (`main.cpp` ~`:1182`), from the same frame counter and the same
"scene start" snapshot the steps use (`FrameCost`, `steps0`): every
counter is read at the scene's first frame (copied into a struct) and
again at its end, and the line holds the differences, plus the mean per
frame where a rate is what the reader wants.

```
glmark2-counters: <scene info string>: frames N <name> v ...
```

The same shape for both GPUs (key/value words, no positional fields), so
the host harness and `tools/cts` parsers can read either. The V3D
exports are under `#if defined(CONFIG_YAKOGL_GPU_V3D)`, the PowerVR ones
under `CONFIG_YAKOGL_DRV_GPU_PVR`, as the includes at `:141`-`:150`; the
GC7000 image prints `glmark2-counters: <scene>: absent`.

All the exports are cumulative since init, so they are read as differences
(their doc comments say so). Each takes null for a value it is not asked
for.

## V3D (Pi 4, Pi 5): what to read

| word in the line | export (`src/v3d/backend.rs`) | difference of | answers |
|---|---|---|---|
| `jobs`, `waits` | `v3d_counters(kicks, waits)` (`:6355`) | both | jobs a frame; CPU waits a frame (`v3d-wait-poll.md`: 9 and 9 on terrain) |
| `copy.<name>` x16 | `v3d_copy_counters(u32[V3D_COPY_REASONS = 16])` (`:2269`) | each reason, only the non-zero printed; names `COPY_REASON_NAMES` (`:2224`): plain, r32f, first-sample, cpu-write, relayout, not-stored, no-chain, layout, depth-refused, upload, arena, stamp-replaced, stale-given-back, image-store, level0-back, chain-back | why a draw's binding was tiled (candidate 1, 3 of `pi4-slow-terrain.md`) |
| `mip.refused.<reason>`, `mip.queued`, `mip.full` | `v3d_mip_counters(refused[V3D_MIP_REFUSALS], queued, queue_full)` (`:5034`) | each | glGenerateMipmap on the TFU or on the CPU, and why |
| `rt.live`, `rt.bytes` | `v3d_rt_images` (`:4990`) first two | not a difference: the value at the scene's end | render images held |
| `rt.direct`, `rt.cpuwrites`, `rt.nomem`, `rt.chains`, `rt.back` | `v3d_rt_images` rest (`direct`, `cpu_writes`, `no_memory`, `chains`, `chains_returned`) | each | draw bindings sampled from a render image with no copy; samples sent back to the copy path; images not made; TFU chains; chains written back |
| `tiled.n`, `tiled.bytes`, `tiled.hot` | `v3d_tiled_evictions` (`:5104`): per cause, entries then bytes | each | eviction of tiled copies (candidate 2's cousin: `rt_register` evictions are not in it, see below) |
| `tile.ns[3]`, `tile.untimed` | `v3d_tiling_time(ns[3], untimed)` (`:5157`) | each | CPU time in the store into the tiled layout (upload, kept copy, arena); print `untimed` always: while non-zero the times are short and are not a measurement |
| `tfu.jobs`, `tfu.failed` | `v3d_tfu_counters` (`:5176`) | each | tiled copies made by the TFU, asks that fell back to the CPU |
| `tex.n`, `tex.bytes` | `v3d_texture_counters` (`:6278`) | each | textures tiled by the CPU and bytes |
| `smp.packed`, `smp.reused` | `v3d_sampler_counters` (`:5254`) | each | sampler records |

The user's list in `pi4-slow-terrain.md` is `v3d_copy_counters`,
`v3d_mip_counters`, `v3d_rt_images`, `v3d_counters` and `v3d_tiling_time`;
the four last rows are the other exports of the same file that the same
line can carry at no cost, and are optional (print them in a second
`glmark2-counters2:` line if one line gets too long for the UART).

### Not exported, so not in the line

`pi4-slow-terrain.md` asks for three more that no export gives. Each needs
a small counter in `src/v3d/backend.rs` (a plan on V3D, not this item):

- the render-image eviction count (`rt_register`): a new `v3d_rt_images`
  argument, or `v3d_rt_evictions`;
- the CPU wait time: a clock lap in `wait_job` (`:3057`) summed in
  `V3dState`, exported as `v3d_wait_time(ns, n)`; the line gets
  `wait.ns` and `wait.n`;
- the CPU mipmap filter's time and bytes: in the GL core, not V3D, so it
  reaches the line through the OSAL (`osal_draw_stats` or a new step).

## PowerVR (AM62P): what to read

| word | export | difference of | answers |
|---|---|---|---|
| `kicks`, `waits` | `pvr_backend_counters(kicks, waits)` (`src/pvr/backend.rs:9200`, declared in `pvr_backend.h`) | both | kicks a frame (5 chains x 9 + the frame's own: 52.7 on the kick-times build) and CPU waits a frame |
| `draws`, `tiles` | `pvr_backend_workload(draws, tiles)` (`:9217`, `pvr_backend.h:44`) | both | objects and 16x16 tiles covered: geometry, pixels or per-kick overhead |
| `reposts` | `pvr_render_reposts` (`pvr_render.h:250`, an `extern uint32_t`) | difference | kicks re-posted after the 20 ms fence; the count `am62p-slow-terrain.md` wants for the once-a-frame stall (the GM record had it only in running form from `render.rs:2371-2381`) |
| `fl.grow`, `fl.served`, `fl.refused`, `fl.oom`, `fl.empty`, `fl.filled`, `fl.reposts`, `fl.stop`, `fl.partial` | `pvr_render_fl_counters(uint32_t[9])` (`pvr_render.h:225`) | words 0-6 and 8 (word 7 is a stop cause, not a count: print the last value) | the same re-post count from the driver (word 6), and partial renders |
| `wait_timeouts` | `pvr_backend_wait_timeouts()` (`:9269`) | difference | waits that gave up |
| `depth_sample.*` | `pvr_backend_depth_sample_stats()` (`:4834`, a Rust-only `pub fn`, no C export) | needs an `extern "C"` wrapper first | not for terrain; leave out |

`pvr_render_reposts` and `pvr_render_fl_counters` word 6 are the same count
read twice; print one (`reposts`) and keep word 6 out, so the line does not
carry a number that can disagree with itself.

What the PowerVR exports cannot give, and what the AM62P note asked for:
which wait meets a re-post (frame set, swap or finish) and the last
posted fence beside the waited one. That is `wait_fence`'s callers and the
OSAL's wait name (`pvr_render.c:783`): driver and library code, not a
line in `main.cpp`. A counter by call site is the design: three `u32`
(`waits_frame_set`, `waits_swap`, `waits_finish`) in the backend beside
`waits`, exported with `pvr_backend_counters`'s successor.

## What the line must not do

- Read a counter inside the timed frames: read once at the scene's first
  frame and once at its end, as `FrameCost` does for the steps, so the
  line costs no frame time.
- Print a line for a scene the benchmark did not run: `frames 0` prints
  nothing, as the steps line returns on `c.frames == 0` (`:1177`).
- Reset a counter: the exports are the library's, other readers (the
  host harness's `gm2: v3d-frames` line, `shim.c`) use the same ones.
- Change an ES 2.0 answer: it only reads.

## Order of work

1. `main.cpp`'s line for V3D and PowerVR as tabled, after plan #328
   releases the file (small, rebased on the current main).
2. The three missing V3D counters and the PowerVR wait call sites, each
   its own change with a host test (`v3d_counters`-style: a call, read the
   difference).
3. A host check: `tools/hostharness/glmark2` already prints a per-frame
   `gm2: v3d-frames` line; the board line has the same words, so the
   harness and the board can be diffed for a scene.

Check against the board record: terrain on the Pi 4 should show `jobs 9`
and `waits 9` a frame (`v3d-wait-poll.md`), the AM62P `kicks` about 52 a
frame and `reposts` a frame near 0 or 1 (`pvr-waited-kick.md`).
