<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: is skipping the tile-state `memset` of a V3D job exact, and what it saves

Plan #329 (#1209), candidate 3 of `pi4-terrain-cpu-left.md` ("Fix
candidates, ranked", `:269-276`). This is a study: no code changed and no
board run was made. It was read from the tree at `19c7660e`. Two more
sources were used. One is the Mesa 26.2.1 gallium and Vulkan V3D drivers
(the same release as `third_party/mesa-26.2.1`, which vendors only
`src/broadcom/{cle,clif,common,qpu}`), read from an unvendored checkout of
that release; those citations are marked "Mesa 26.2.1". The other is the board
record `a4b9c551` (`pi4-terrain-board-jobs.md`). Anything taken from memory
of Mesa, the Linux kernel driver or Broadcom documents is marked
**(recalled)**.

## Summary

1. **The tile state (TSDA) is the binner's own per-tile working state, not a
   list head.** Each tile's list starts in the *tile allocation* memory.
   The renderer finds it there (`MULTICORE_RENDERING_TILE_LIST_SET_BASE` =
   tile alloc, `driver/src/v3d/v3d_job.c:1439-1442`). After a job, the TSDA
   holds that job's final per-tile state, which is not zero.
2. **Mesa does not give the binner a zeroed TSDA.** The comment at
   `src/osal/zephyr_v3d.c:1429-1430` says "Mesa allocates it fresh for every
   job". That is only half true. Mesa allocates a TSDA BO per job, but it
   takes it from its BO cache, which hands back a used, idle BO of the same
   page count with its contents unchanged (Mesa 26.2.1
   `src/gallium/drivers/v3d/v3d_job.c:568-569`,
   `v3d_bufmgr.c:119-137` and `:86-116`; Vulkan:
   `src/broadcom/vulkan/v3dv_cmd_buffer.c:501`, `v3dv_bo.c:221-238`, the
   `private` BO from the cache). Under steady load this BO is usually the
   TSDA of an earlier job of the same size, holding that job's end state.
   **So Mesa bins from a TSDA that is not zero all the time.** The
   inference is that the binner initialises the tile state itself at
   `START_TILE_BINNING`. Zeroing is therefore not needed: none of it, not
   only part of it.
3. **What zeroing costs is already measured, and it is small.** Board
   record `a4b9c551`, frame 10. Entry 1 (tilt-shift V, 800x600, 130 tiles,
   33,280 B, started by `start_pending`) has `idle` **14 us**, and that
   window contains its `memset`. Entry 2 (noise, 256x256, 16 tiles,
   4,096 B, started with the GPU idle) has `prep` **1 us**. So the rate is
   **at most about 0.45 ns/B**, which is R1's 0.36 ns/B and not R5's
   10 ns/B. Terrain's 8 jobs zero 149,504 B a frame, which is **at most
   about 0.07 ms a frame (0.2 % of 34.7 ms)**. That is under the 0.1 ms
   run-to-run spread of the frame time.
4. **Verdict: skipping is exact (by Mesa's own practice, and confirmed by a
   poison run on the board, below). The gain is at most 0.07 ms a frame,
   and no FrameTime can show it.** Of the alternatives, only "skip" and
   "move it under the previous render" are exact without hardware
   documents. Neither is worth a change for terrain alone. The cheap item
   is the inaccurate comment (`zephyr_v3d.c:1429-1430`,
   `src/v3d/backend.rs:3359-3363`).

## (a) What the binner writes into the tile state, and what is zero after a job

What the tree shows:

- The bin submit hands the binner three memory areas. They are the tile
  allocation memory (`CT0QMA`, `CT0QMS`), the tile state with its enable bit
  (`CT0QTS | CTQTSEN`) and the binner list (`CT0QBA`, `CT0QEA`). See
  `driver/src/v3d/v3d_hw.c:401-418` and the register comment "The binner
  also takes the tile allocation memory (QMA, QMS) and the tile state (QTS,
  with CTQTSEN)" (`driver/src/v3d/v3d_regs.h:266-271`, `:279-280`). Mesa's
  simulator writes the same registers and does nothing else to the TSDA
  (Mesa 26.2.1 `src/broadcom/simulator/v3dx_simulator.c:523-526`). The
  render submit (`v3d_hw.c:421-430`) does not name the tile state. It is
  binner-only memory (`v3d-bin-overlap.md:129`).
- The 4.2 packet that configures binning has no tile-state field at all.
  `Tile Binning Mode Cfg` (code 120,
  `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:943-965`) has
  the size, the double-buffer and MSAA bits, the max bpp, the RT count and
  the two tile-allocation block sizes. VC4's packet 112 had both a "Tile
  State Data Array Address" and an "Auto-initialise Tile State Data Array"
  bit (`third_party/mesa-26.2.1/src/broadcom/cle/vc4_packet.xml:239-265`,
  bit at `:256`, address at `:263`). On V3D 4.x the address moved into the
  `CT0QTS` register and the auto-initialise choice went away.
  **(recalled)** On V3D 3.3 and later the binner always initialises the
  TSDA at `START_TILE_BINNING` (`v3d_packet.xml:431`). The VC4 kernel
  validator forced the auto-initialise bit on, so no VC4 job ever relied on
  a zeroed array either.
- Where the tile lists are. "The PTB will request the tile alloc initial
  size per tile at start of tile binning"
  (`third_party/mesa-26.2.1/src/broadcom/common/v3d_util.c:281-286`). The
  tile allocation is sized `tiles x 128 B` plus pool (`:285-311`,
  `driver/src/v3d/v3d_frame.c:118-128`). Each tile's first list block is in
  the tile allocation memory, and the RCL reads it from there with
  `TILE_LIST_INITIAL_BLOCK_SIZE` (auto-chained), `MULTICORE_RENDERING_TILE_LIST_SET_BASE`
  `= tile_alloc` and `BRANCH_TO_IMPLICIT_TILE_LIST`
  (`driver/src/v3d/v3d_job.c:1434-1442`, `:1250`;
  `v3d_packet.xml:1179-1195`, `:464-466`). So no list head lives in the
  TSDA.
- The binner list's epilogue is `FLUSH` (`driver/src/v3d/v3d_job.c:1340`;
  `v3d_packet.xml:429`). FLUSH makes the binner cap every tile list with a
  return (`driver/src/v3d/v3d_frame.c:332-334`). The caps go into the tile
  lists, which are in the tile allocation memory.

What the binner keeps in the TSDA **(recalled, not in any vendored
document)**. It is 256 B a tile on V3D 4.x
(`v3d_util.c:312`: `*tile_state_size = layers * tiles_x * tiles_y * 256`),
where VC4 used 48 B a tile (`pre-study-rpi-zero-2w.md:258`). It holds the
binner's working state for each tile:

- the current write position in that tile's list and the bytes left in the
  current block, which the binner needs in order to chain a new block from
  the pool;
- the state last written into that tile's list (the shader-state record,
  the primitive list format, index or instance bases and so on), so that
  state packets go only into the tiles a primitive touches, and only when
  they changed.

**Is any byte zero again after the job?** Nothing in the tree, the packet
XML or the binner registers clears the TSDA at the end of a job. The
`memset` at `zephyr_v3d.c:1431` is the only writer on the CPU side. A tile
that got at least one primitive or state packet ends with a write pointer
into a tile-allocation block, and that is not zero. Even a tile that got
nothing has had its list initialised (its initial block requested at the
start of binning) and capped by FLUSH. So **for this question the answer
is "no": a used TSDA is not zero after the job**. The next job's
`memset` really does write. It is not a no-op over memory that is already
zero, and "skip when already zero" (the wording of candidate 3) would never
skip. Exactness has to come from the binner ignoring the old contents, not
from the contents being zero.

## (b) Which part must be zero at the start of a bin

- **By the evidence: none of it.** Mesa's TSDA comes from the BO cache, and
  the BO is idle (`v3d_bo_wait(bo, 0, NULL)`, Mesa 26.2.1 `v3d_bufmgr.c:99-107`)
  but not cleared. The cache files a BO by its page count
  (`page_index = size / 4096 - 1`, `:89`, and the free side, `:277-312`),
  so the next TSDA of the same page count gets a former TSDA of a job of
  that size, or any other freed BO of that size (a texture, a list). The
  first TSDA of a size is a fresh kernel BO, which is zeroed
  **(recalled: shmem pages)**. Mesa's GL and Vulkan drivers run every
  conformance suite this way, so the binner cannot depend on the old
  contents. The same holds here: before the `memset` the tile state pool is
  carve-out memory that is not initialised at boot
  (`static uint8_t carve[...] __noinit`, `zephyr_v3d.c:106`). `z_gpu_alloc`
  (`:411`) does not clear it either. Only the `memset` does.
- **"Only the per-tile list heads" is not a choice on this GPU.** The
  heads are in the tile allocation memory, which the binner writes at the
  start of binning ((a)). Nothing of the TSDA's 256 B a tile is documented
  in the tree as needing a known value.
- **Sizes at terrain's targets.** The memset is sized to the job's own
  tiling, not to the pool (`job_tile_state_bytes`,
  `src/v3d/backend.rs:3367-3372`, passed as `kick.tile_state_size` at
  `:3392`, `:3407`). The pool itself is sized for `MAX_TARGET` (4096)
  square at 4 RTs (`backend.rs:447`, `:6418-6427`). The tile size comes from
  `v3d_frame_tiling_gen` (`driver/src/v3d/v3d_frame.c:64-95`: RGBA8, one RT,
  no MSAA, so 64x64). Terrain's targets are listed in
  `v3d-terrain-gpu.md:101-111`.

| target | tiles | tile state bytes | jobs a frame |
|---|---:|---:|---:|
| 800x600 (window, terrain target, tilt-shift H) | 13 x 10 = 130 | 33,280 | 4 (window, terrain, overlay, tilt-shift H) |
| 256x256 (height map, normal map, bloom H, bloom V) | 4 x 4 = 16 | 4,096 | 4 |
| **frame, 8 jobs** (after `4255adf1`, the dead window clear) | 584 | **149,504** | 8 |
| frame, 9 jobs (before it; the 182,784 B of `v3d-frame-cpu-model.md:133`) | 714 | 182,784 | 9 |

If only VC4's 48 B a tile mattered (recalled, and not known to hold on
4.2), the 800x600 job would need 6,240 B in 130 strided pieces. That is not
an exact option without a 4.2 document.

## (c) Exact alternatives

| # | alternative | exact? | what it needs |
|---|---|---|---|
| C0 | **Skip the `memset`** (`zephyr_v3d.c:1431`) | Yes, by Mesa's practice ((b)). Confirm it once on the board with a **poison fill** instead of a plain skip: write `0xA5` instead of `0`, so that a binner that did read the old state would fail every job, not only some | One line, and a knob for the poison run |
| C1 | Zero only the part needed | Already done at the size level. The job's own tiles are zeroed, not the 1 MiB pool (`backend.rs:3359-3366`; this took glue's DEMO15 `prep` from 307 to 76 us). Below a tile: not exact without a document | Nothing more |
| C2 | Zero it with the GPU: a TFU job or a clear job | Exact if done, but **no gain**. The TFU converts a source image (it would need a zero source as large as the area, which is read again), and it runs on the same serial path, with the GPU idle between jobs (`tfu_queue_run`, `zephyr_v3d.c:1311`). A clear job is a whole bin and render, at 10 to 60 us of fixed cost a job (`v3d-wait-poll.md:22`, `:131`), more than the 14 us `memset` it replaces | New job kinds, for a loss |
| C3 | Zero it while the previous job renders | Exact if the binner is done with the TSDA at FLDONE. It is binner-only memory ((a)), and the render starts only after FLDONE (`v3d_isr`, render submit `v3d_hw.c:421-430`). The place to do it is in `z_submit`, when a job is parked (`:1406-1413`) and the running job's bin is over (`!s.bin_pending`), or at FLDONE. Not in the ISR: up to 15 us there | The parked job's tile state address (the same single pool) |
| C4 | Two tile-state areas, alternated | Exact (the same as C3, with the area never shared). It is half of `v3d-bin-overlap.md` candidate 1 (`:204-236`, "a second tile-alloc and tile-state area"). It adds a second pool (the pool is sized for `MAX_TARGET` at 4 RTs, `backend.rs:6418-6427`), and on its own it saves no more than C3 | Backend pool and kick changes |

C0 removes the cost. C3 and C4 move it off the GPU-idle path, so the CPU
still pays it but the GPU no longer waits. C1 is done, and C2 costs more
than it saves.

## (d) Expected gain, and the board value that confirms it

**Where the `memset` sits.** Every `start_job` runs with the GPU idle. In
`z_submit` the GPU is idle at a submit that finds nothing in flight
(`:1415`). In `start_pending` it runs right after the previous job's end
(`finish_inflight`, `:1311-1316`). So the whole `memset` is GPU-idle time,
in two stamps of the ring (`osal_v3d_recent_jobs_us`, `zephyr_v3d.c:786`;
fields in `v3d-jobs-readout.md:16-27`):

- **`prep`** (`:1433-1434`) holds it for a job submitted with the GPU idle.
  For a parked job, `prep` counts from the submit and is mostly queue time
  (`pi4-terrain-cpu-left.md:20-29`).
- **`idle`** (`note_job_done`, `:769`, `:778`: this job's bin submit less
  the previous job's GPU end) holds it for a parked job that
  `start_pending` started. It reads 0 when a TFU job run since moved
  `recent_prev_end` past the bin submit (`:1360`, "stamps cross", `:744`),
  which is why entries 4 to 8 read 0.

**What board record `a4b9c551` already reads**
(`pi4-terrain-board-jobs.md:47`, `:52-54`; the order of the entries is in
`v3d-blur-jobs.md:85-95`):

| entry | job | tiles, bytes | started by | stamp holding the `memset` | value | bound on the `memset` |
|---|---|---|---|---|---:|---|
| 1 | tilt-shift V into the window, 800x600 | 130, 33,280 | `start_pending` (prep 3609 = queue) | `idle` | 14 us | **under 15 us**: at most 0.45 ns/B |
| 2 | noise (height map), 256x256 | 16, 4,096 | `z_submit`, GPU idle (`idle` 80) | `prep` | 1 us | under 2 us |
| 3 | normal map, 256x256 | 16, 4,096 | `start_pending` | `idle` | 1 us | under 2 us |

The values are rounded down (`k_cyc_to_us_floor32`,
`v3d-jobs-readout.md:12-13`), so a printed 14 means less than 15. Each
window also holds `complete()`, `note_job_done` and the stamps, so the
bounds are upper bounds. At most 0.45 ns/B agrees with R1, 0.36 ns/B
(`v3d-frame-cpu-model.md:108`). It is about 20 times better than R5, which
the earlier "up to 1.8 ms" rested on.

**Expected gain a frame** (149,504 B; 4 jobs at 33,280 and 4 at 4,096):

| alternative | GPU-idle time removed a frame | CPU time removed | confirms on the board |
|---|---|---|---|
| C0 skip | 4 x under 15 + 4 x under 2 = **under 0.07 ms** (about 0.054 at R1) | the same | entry 1's `idle` 14 drops to 2 or less, entries 2 and 3 read 0 or 1, and the other columns stay the same. FrameTime moves less than its 0.1 ms spread, so it is not the reader |
| C3 overlap | the same, under 0.07 ms | none (it moves) | the same `idle` drop |
| C4 two areas | the same, under 0.07 ms | none | the same `idle` drop, and more only together with `v3d-bin-overlap.md` candidate 1, which that note bounds at about 0.29 ms (`pi4-terrain-board-jobs.md:84-92`) |
| C1, C2 | 0 (C1 is done; C2 adds job cost) | 0 | - |

**The value named for this item** ("`prep` of a job that starts with the
GPU idle") is entry 2's 1 us, which was already read. It closes the
question in the way `pi4-terrain-cpu-left.md:272-275` set out: "under 0.1
ms a job closes the question". Entry 2 is a 16-tile job, though. Entry 1's
`idle` of 14 us is the reading for a full 800x600 job, and it closes the
question too. **Candidate 3 is a gain of at most 0.07 ms a frame; this
agrees with `pi4-terrain-board-jobs.md:105-109` ("refuted").**

If C0 is made anyway (for example together with the Pi 5 OSAL), the board
check is a correctness check more than a speed check. Run the 33 glmark2
scenes and gl_test with the poison fill: every scene runs to its record
with the same pictures, and there are no MMU faults and no hung jobs
(`diag_mmu_faults`, `hung_refused`, the "render did not end" line,
`zephyr_v3d.c:1291-1300`). After that, `idle` drops on entry 1.

## (e) Files, the plan that holds each, and a host test design

| file:line | what | holder |
|---|---|---|
| `src/osal/zephyr_v3d.c:1429-1431` | the `memset`, and the comment "Mesa allocates it fresh" (inaccurate, see (b)) | No other plan lists `src/osal/zephyr_v3d.c` (`v3d-pending-look` of this plan changed it). A C0, C3 or poison knob would be #329's, rebased on main |
| `src/osal/zephyr_v3d71.c:666`, `:686` | the same `memset` in the Pi 5 (V3D 7.1) OSAL | #321's Pi 5 work. 7.1 sizes its tiles itself (`v3d_packet.xml:967-993`), and the same Mesa evidence applies (the cache code is shared). Leave it to #321 |
| `src/v3d/backend.rs:3359-3372`, `:3392-3407`, `:6418-6448` | `job_tile_state_bytes` (the size), its comment "The OSAL zeroes this much ... Mesa ... per job", the kick, and the single pool | #308 (`src/v3d/**`). Correcting the comment, and C4's second pool, wait for #308 to release the file |
| `driver/src/v3d/v3d_hw.c:401-418`, `v3d_job.c:366-377`, `:1340`, `:1434-1442` | the bin submit, the bin prologue and epilogue, and the RCL's tile list base | #321 (`driver/src/v3d/**`). No change needed by any alternative |
| `docs/notes/pi4-terrain-cpu-left.md:269-276`, `pi4-slow-terrain.md` ranking | candidate 3 as written ("jobs whose state is already zero") | Not edited here (non-goal). `issue-1209-final` fills the ranking |

**Host test design.** No host build reaches `zephyr_v3d.c`
(`tools/v3d/osal_frees_check.c:4-5`), and the host stand-in's jobs complete
at submit without a binner (`src/osal/linux.c`, `pi4-terrain-cpu-left.md:191-193`).
So a host test can pin the OSAL's decision and the sizes, but not the
binner's behaviour. That stays the board's poison run. The shape, after
`tools/osal/pending_frees_check.c` and `tools/v3d/osal_frees_check.c`:

1. Move the decision into a header, `src/osal/v3d_tile_state.h`, the way
   `v3d_frees.h` is shared (`zephyr_v3d.c:82`). It would be one function
   `v3d_tile_state_prepare(uint8_t *ts, uint32_t bytes, int mode)` with
   mode zero (as now), skip, or poison (`0xA5`). `start_job` calls it
   (`zephyr_v3d.c:1431`), and a build-time `CONFIG_`/`-D` knob picks the
   mode. Zero is the default until the board poison run passes.
2. `tools/osal/v3d_tile_state_check.c` (`tools/osal/*`, the `osal` scope of
   `tools/check.sh:252`) pins:
   - zero mode writes exactly `bytes` zeros and not a byte past them (a
     guard canary after the area);
   - skip mode writes nothing (a canary fill stays whole);
   - poison mode writes `bytes` of `0xA5` and no more;
   - the sizes at terrain's targets, through `v3d_frame_tiling_bpp`
     (`driver/src/v3d/v3d_frame.c:50`, compiled on the host as other checks
     do). 800x600 RGBA8 one RT gives 130 tiles and 33,280 B, 256x256 gives
     16 tiles and 4,096 B, and the 8-job frame gives 149,504 B. At 4 RTs and
     4096x4096 the size equals the pool `v3d_frame_alloc_sizes` gives
     (`:118-128`), so no job outgrows it (`backend.rs:3393-3397`).
3. Registering the row needs `tools/check.sh`, which #325 and #321 are
   changing. The row waits for a free place, or goes in as one small
   rebased commit.
4. ES 2.0 answers cannot move. The OSAL is not in the host build, and the
   default mode is unchanged, so `tests/es2_report_guard.rs` is unaffected.

## What this note does not settle

- The TSDA's exact byte layout on 4.2 and the binner's initialisation are
  recalled, not documented in the tree. The evidence in the tree is
  indirect: Mesa's cache reuse and the missing auto-initialise field. The
  poison run is the direct test.
- One record (`a4b9c551`) and frame 10 only. The bound rests on entry 1's
  `idle`, a single reading.
