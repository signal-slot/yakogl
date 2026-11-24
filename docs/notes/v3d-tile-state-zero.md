# V3D: is skipping the tile-state memset exact, and what it saves (study)

Plan #329 (#1209), candidate 3 of `pi4-terrain-cpu-left.md` ("Fix candidates,
ranked"). A study: no code changed, no board run. Statements marked
**(recalled)** are Mesa or kernel behaviour remembered, not read in this
tree; a source check or a board dump must confirm them before code relies on
them.

## What the code does today

- `start_job` zeroes the job's tile state, `memset(ts, 0, k->tile_state_size)`
  (`src/osal/zephyr_v3d.c:1431`), on the **Normal non-cacheable** carve-out
  (`:235`, `K_MEM_ARM_NORMAL_NC`), just before `v3d_submit_bin` (`:1457`). The
  comment at `:1429-1430` gives the reason: Mesa allocates the array fresh per
  job. The V3D 7.1 OSAL and the host stand-in do the same
  (`src/osal/zephyr_v3d71.c:677`, `src/osal/linux.c:913`).
- `start_job` runs from two places: `z_submit` when nothing is in flight
  (`:1415`) and `start_pending` (`:1466-1478`), called when the previous job
  ends (`finish_inflight`, `:1316`, after the render and the cache clean
  `v3d_caches_clean` at `:1302`) or fails (`:1215`). In both the GPU is
  **idle**, so the memset is exposed time.
- The size is the job's own, `tiles_x * tiles_y * 256`
  (`src/v3d/backend.rs:3361-3375`, `driver/src/v3d/v3d_frame.c:127`), passed in
  the kick (`src/v3d/backend.rs:3390-3410`). The pool is sized for the 4-RT 32x32
  maximum: 4 MiB at `MAX_TARGET` (`src/v3d/backend_tests.rs:1423`, `src/v3d/backend.rs:6427`,
  allocated `:6418`).
- Terrain's sizes (`v3d-terrain-gpu.md:101-111`): 800x600 at 64x64 tiles is
  13 x 10 = 130 tiles = **33,280 B**; 256x256 is 16 tiles = **4,096 B**.
  The 182,784 B a frame in `pi4-terrain-cpu-left.md:178` is 5 x 33,280 +
  4 x 4,096: the **9-job** frame. With the dead window clear gone (8 jobs) it
  is 4 x 33,280 + 4 x 4,096 = **149,504 B**. The candidate's expected gain
  should use that: **0.054 ms (R1, 0.36 ns/B) to 1.5 ms (R5, 10 ns/B)**, not
  0.07 to 1.8.

## (a) What the binner writes, and whether any of it is zero afterwards

In the tree:

- The binner is given the tile state address and enable at submit:
  `CT0QTS = CTQTSEN | tile_state` (`driver/src/v3d/v3d_hw.c:416`), beside the
  tile allocation memory `CT0QMA` (`:414`). The render start
  (`v3d_submit_render`, `:421-428`) gets only the RCL, no tile state address.
  The render finds its tile lists through the RCL's
  `MULTICORE_RENDERING_TILE_LIST_SET_BASE`
  (`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:1179-1183`), which
  points into the tile **allocation** memory, and the initial block size
  (`:955-963`, `:984-992`) the binner and RCL must agree on. So the tile state
  belongs to the binner alone; the render does not read it.
- The XML has **no** packet or field describing the tile state layout. Nothing
  under `third_party/mesa-26.2.1/src/broadcom/cle/` or in this tree says which
  of the 256 bytes a tile the binner reads first or writes last.
  `pi4-v3d-bringup.md:139` and `v3d71-clear-frame.md:144` give only the size,
  `tiles * 256`.

Recalled (not in the tree):

- The array is the binner's per-tile state: where each tile's primitive list
  currently ends in the tile allocation blocks, and related counters. The
  hardware needs it zero at the start of a bin; Mesa gets that by allocating a
  fresh BO per job (`v3d_job.c`, `alloc_tile_state`) whose pages the kernel
  hands out zeroed. `v3d-swap-wait.md:96-97` says the same.
- A tile that received primitives ends with a non-zero entry. A tile nothing
  touched may stay zero.

Answer: **no byte can be assumed zero again after a job.** Terrain's
full-screen jobs touch all 130 tiles of a target. A skip "because the state is
already zero" would be exact only for tiles the binner never wrote, and nothing
in the tree tells the OSAL which those are.

## (b) What must be zero at the start of a bin

Not provable from the tree. Two readings:

1. **All of it** (today, and Mesa): safe.
2. **Only the per-tile list heads** (recalled guess: a small head of the 256 B).
   Sizes at terrain's targets for an assumed head, arithmetic only:

   | head per tile | 800x600 (130 tiles) | 256x256 (16 tiles) | frame, 8 jobs |
   |---:|---:|---:|---:|
   | 256 B (today) | 33,280 | 4,096 | 149,504 |
   | 32 B | 4,160 | 512 | 18,688 |
   | 16 B | 2,080 | 256 | 9,344 |

   The head size is the unknown, so this is **not exact until a board dump or a
   source check names it**. A strided store run on non-cacheable memory is not
   cheaper per byte than a long one, so the gain is below the byte ratio.

## (c) Exact alternatives

1. **Zero only the needed part.** Exact only with the head layout (b). A wrong
   head size means the binner reads stale list pointers: a hang or garbage, not
   a slow frame.
2. **Zero it with the GPU (TFU or clear job).** The TFU turns a texture into a
   texture (`v3d-tfu-order.md`): it needs a zero source as large as the target
   and its own fixed cost. A clear render job costs a bin and a render submit,
   milliseconds of fixed cost (`v3d-terrain-gpu.md:313`), to save at most
   1.5 ms. **Not worth building.**
3. **Zero it while the previous job renders, same area.** The previous job's
   bin is done at FLDONE (`src/osal/zephyr_v3d.c:1030-1058`, `bin_pending` cleared at
   `:1050`) and the render does not read the array (above). At `z_submit`
   with `s.inflight && !s.bin_pending` the waiting kick (`:1408-1413`) already
   has its size. **Hazard (recalled):** the binner's writes may sit in the V3D L2
   cache until `v3d_caches_clean` (`:1302`, after the render); a CPU zero
   before that clean can be overwritten by the write-back, leaving the next bin a
   dirty array and no error. A same-area zero is exact only after the clean,
   when the GPU is idle again: no gain. **Rejected.**
4. **Two areas alternated.** Job N bins with area A; area B (used by N-1,
   already cleaned) is zeroed by the CPU while N bins and renders; N+1 uses B.
   A late write-back of A cannot land on B, so it is exact under the recalled
   hazard. Cost: a second pool (4 MiB at `MAX_TARGET`, or sized to the largest
   job seen) from `V3D_HEAP_DATA`, and the kick naming the area. It hides the
   memset of every job that is submitted while a predecessor is in flight. The
   frame's first job has none (the swap waited, the GPU is idle) and still
   pays its 33,280 B unless the other area was zeroed right after the previous
   frame's last clean, at the size seen last frame.

## (d) Expected gain, and the board value that confirms it

Rates R1 to R5 of `v3d-frame-cpu-model.md`, as `pi4-terrain-cpu-left.md:176-188`,
recomputed for 149,504 B.

| alternative | exact | gain a frame |
|---|---|---:|
| head only (16 to 32 B) | only with the layout | up to 0.05 to 1.4 ms |
| GPU zero | exact | negative: do not build |
| overlap, same area | no (recalled hazard) | do not build |
| two areas, zero while N runs | exact | 0.04 to 1.3 ms (116,224 B, all but the first job's 33,280 B, at 0.36 to 10 ns/B) |

Board value, no new code, from the ring `osal_v3d_recent_jobs_us`
(`src/osal/zephyr_v3d.c:740-760`):

- **`prep` of the frame's first job**, whose submit finds no job running:
  `c0` is taken in `z_submit` (`:1398`) and `prep = c_bin - c0` (`:1434`), so it
  is the memset plus a few instructions.
- For a job that started from `start_pending`, `prep` is queue time
  (`pi4-terrain-cpu-left.md`, answer 2) and does not count; its **`idle`**
  (previous GPU end to this bin submit, `:740-745`) contains the memset.
- Under 0.1 ms for a 130-tile job (33,280 B, 3 ns/B or better) closes the
  candidate. 0.2 ms or more means R3 to R5 and the two-area form is worth
  0.5 to 1.3 ms a frame. The sum of `idle` over the 8 jobs bounds the gain from
  above.

## (e) Files, plans, host test design

| change | files | plan |
|---|---|---|
| kick names the area; OSAL zeroes the other | `src/osal/zephyr_v3d.c` (`z_submit` `:1376-1420`, `start_job` `:1423-1459`), `src/v3d/kick.rs:31-32` | `src/v3d/**` is #308's: order after #308 releases it; the OSAL file is in no listed plan |
| second pool, area choice | `src/v3d/backend.rs:3361-3410`, `:6385-6430` | #308 |
| a new kick field, if any | `driver/src/v3d/v3d.h`, `v3d_kick.h` | `driver/src/v3d/**` is #321's |
| 7.1 OSAL mirror | `src/osal/zephyr_v3d71.c:657-689` | Pi 5 work (#321) |
| host stand-in | `src/osal/linux.c:913` | whoever changes the kick |

Host test design (the host cannot show a speed-up; it can show exactness):

1. **Kick contract**, `src/v3d/backend_tests.rs` beside `:1472-1474` and
   `:4593`: two consecutive jobs name alternating `tile_state` areas, each
   `tile_state_size` as `job_tile_state_bytes` gives.
2. **Zero at bin start**, in the stand-in OSAL (`src/osal/linux.c:913`): poison the area
   with `0xA5` after each job to model a dirty array; before the bin assert the
   area named by the kick is all zero. A skip that leaves a byte fails.
3. **Hazard model**: inject a late store into the previous job's area after the
   next job's overlap zero. The same-area design fails it; the two-area design
   passes. This pins why the same area is rejected.
4. **Picture**: `tests/es2_report_guard.rs` unchanged, and the terrain job
   sequence (`v3d-terrain-job-pin`) renders the same bytes with and without.
5. **Head-only** has no host test until a board dump (read `tile_state` after a
   job, which bytes of each 256 B are non-zero) names the layout.

## Verdict for the ranking

The simple skip is **not exact**: terrain leaves all 130 tiles of a big job
non-zero. Only the two-area form is exact, worth 0.04 to 1.3 ms a frame, with
a second 4 MiB pool, behind #308's release of `src/v3d/**`. Measure `prep` of
the first job and `idle` first; under 0.1 ms per 130-tile job, close the
candidate. The frame is GPU-bound (28.4 of 34.7 ms is `wait_job`), so this is a
last-millisecond candidate, not a lever.
