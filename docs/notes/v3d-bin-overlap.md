<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: can job N+1's bin overlap job N's render, and when does a waiting job start

Study for plan #329 (#1209), item v3d-bin-overlap-study. It answers
candidates 1 and 2 of `pi4-terrain-cpu-left.md` ("Fix candidates, ranked",
`docs/notes/pi4-terrain-cpu-left.md:249-268`). No code was changed and no board
was run. Tree: written at `93eb0f0a`; every line reference was rechecked
at main `7eb634e6` (`src/osal/zephyr_v3d.c` and the cited notes were
unchanged between the two, so only `src/v3d/backend.rs` lines moved). Mesa and Linux-kernel behaviour
written from memory is marked **(recalled)**. It was not checked against a
source tree here; only the packet XML under `third_party/mesa-26.2.1` was read.

## Answer in short

1. **(a) A waiting job starts only when the CPU is inside the OSAL.** That
   means a `wait()`, a `completed()`, the next submit, a free or allocation
   poll, or a TFU run. It never starts from the interrupt. `v3d_isr` handles
   only FLDONE and OUTOMEM, and FRDONE is never unmasked. **Candidate 2 is
   not moot.** As the backend works today, though, it is worth little:
   terrain's CPU is already inside `wait_job` when the running job ends.
2. **(b) The hardware allows the overlap.** The CLE has two threads: CT0
   bins and CT1 renders, each with its own queue registers. The Linux driver
   (recalled) runs bin and render as separate scheduler queues. Each job has
   its own tile-alloc and tile-state BO, and the overflow memory is tied to
   the job's render.
3. **(c) What yakogl needs:**
   - a second tile-alloc and tile-state area, chosen by the backend, because
     the RCL embeds the tile-alloc address. That is 6,823,936 B at the
     current worst-case sizing, or 586,240 B for terrain's largest job.
   - overflow blocks owned per job, not reset at every bin start.
   - a "bin may not overlap" bit in the kick.
   - per-stage in-flight records.
   - the render of N+1 starts only after N's FRDONE, cache clean and queued
     TFU jobs.
4. **(d) Expected gain at terrain is small: about 0.1 to 0.7 ms a frame from
   the bins, plus up to 0.07 to 1.5 ms of tile-state `memset` moved under
   the GPU.** Job 3's large bin (131,072 triangles) **cannot** overlap job
   2's render, because its coordinate shader samples the textures jobs 1
   and 2 render. That removes most of the 1 to 4 ms that
   `pi4-terrain-cpu-left.md:252` assumed. The board value that confirms it
   is the ring's per-job `bin` and `prep` (see (d)). No note in the tree
   gives a per-job bin time yet.
5. **(e) Files and owners:**
   - `src/osal/zephyr_v3d.c` (no plan named as its holder).
   - `src/v3d/backend.rs` and `src/v3d/kick.rs` (plan #308).
   - `driver/src/v3d/v3d_kick.h` and `v3d_hw.c` only if their shared
     definitions change (plan #321).
   - `driver/samples/zephyr/glmark2/src/main.cpp` to print `bin` and `prep`
     (plan #328).
   - A host test needs `zephyr_v3d.c` built on the host for the first time,
     on the `hw_mock_test.c` register model given a clock.

## (a) When a waiting job starts today

The OSAL holds at most two jobs: one in flight and one waiting in `s.pending`
(`src/osal/zephyr_v3d.c:145-150`, header comment `:24-30`).

- **Submit.** If a job is already waiting, the running one is finished first
  (`z_submit`, `:1395-1396`, which calls `finish_inflight(false)`). If a job
  is in flight, the new kick is copied into the slot and the call returns
  (`:1406-1413`, comment "the binner and the tile memory belong to the job
  running"). Otherwise it starts at once (`start_job`, `:1415`).
- **The ISR starts only a render, never a job.** `v3d_isr` (`:1010-1052`)
  reads `V3D_CTL_0_INT_STS` masked to `V3D_BIN_IRQS` = FLDONE | OUTOMEM
  (`:170`, `:1014`).
  - OUTOMEM: it gives an overflow block (`:1020-1031`).
  - FLDONE: it records the pool and calls `v3d_submit_render` (`:1032-1050`).
  - It never touches `s.pending` and never calls `start_pending`.
  - FRDONE is not in the mask and has no handler: "FRDONE has no interrupt"
    (`:1273`, `:1463-1465`). Only FLDONE and OUTOMEM are ever unmasked
    (`:1454-1456`). The IRQ is connected once (`:261`).
- **`start_pending` (`:1466-1478`) has two callers, both on CPU paths:**
  - `finish_inflight` (`:1316`), after FRDONE was waited for or polled
    (`:1279`), the cache clean (`:1302`), the queued TFU jobs (`:1311`) and
    the fence's completion (`:1313`).
  - `bin_finish`'s failure path (`:1215`), after a failed bin is recovered.
- **`finish_inflight` is reached only from CPU paths:**
  - `z_wait` (`:1514-1524`)
  - `z_completed` (`:1486-1487`, only when `inflight_ready` reads FRDONE up,
    `:1228-1240`)
  - `z_submit` (`:1396`)
  - `finish_all` (`:1322-1326`), from `osal_v3d_tfu` (`:1344`)
  - the frees poll `poll_completed` (`:395-399`) and its wait `wait_pending`
    (`:331-333`, through `z_wait`)

So **the event that starts a waiting job is the CPU noticing FRDONE**: in a
wait, a poll, a submit or an allocation. The ISR covers only the bin to render
step of the job already in flight.

**Candidate 2 of `pi4-terrain-cpu-left.md` is therefore not moot. On terrain
its ceiling is the `idle` the ring records, and it is probably small.** The
backend waits for a job in the GL call that ends a job (8 waits a frame,
`pi4-slow-terrain.md:554`; `wait_job`, `src/v3d/backend.rs:3162-3174`).
That wait is `z_wait`, which spins on FRDONE (`v3d_wait`, `:1279`) and then
starts the pending job at `:1316`. With 28.4 ms of the 34.7 ms inside
`wait_job` (`pi4-slow-terrain.md:563-565`), the CPU is in the wait when most
renders end, so the pending job starts within the poll latency. The window
candidate 2 would close is a render that ends while the CPU is recording,
outside any wait. The ring measures that window as `idle` (`:769`, `:778`;
printed by `print_jobs`, `driver/samples/zephyr/glmark2/src/main.cpp:1402-1420`).

Starting the waiting job from the ISR needs three things:
- FRDONE unmasked.
- The clean (`v3d_caches_clean`, two bounded register polls,
  `driver/src/v3d/v3d_hw.c:377-389`) moved to the ISR or made unnecessary.
- The queued TFU jobs (`tfu_queue_run`, `:1111-1140`, each one polled,
  `tfu_now`, `:1354-1374`) run from an interrupt as well.

Neither the clean nor the TFU poll belongs in an ISR, so the ISR could start
only a job that has no TFU behind its predecessor. On terrain, five of the
eight jobs have TFU behind them (`v3d-terrain-gpu.md:101-111`: jobs 3, 6 and 7
an 800x600 chain, jobs 4 and 5 a 256x256 chain). Only jobs 2 (after 1), 3
(after 2) and 1 (after 8, but past the swap's wait) follow a job with none.

## (b) What V3D 4.2 hardware and Mesa/Linux allow

### The registers (from the tree's headers)

`driver/src/v3d/v3d_regs.h:251-293`. The header comment at `:267-272` says: "thread 0
bins, thread 1 renders. Writing a thread's end address (QEA) starts it at its
base (QBA)".

| register | offset | role |
|---|---|---|
| `V3D_CLE_0_CT0QBA` / `CT0QEA` | `0x160` / `0x168` (`:281`, `:283`) | binner list base and end; the QEA write starts the bin |
| `V3D_CLE_0_CT1QBA` / `CT1QEA` | `0x164` / `0x16c` (`:282`, `:284`) | render list base and end; the QEA write starts the render |
| `V3D_CLE_0_CT0QMA` / `CT0QMS` | `0x170` / `0x174` (`:285-286`) | tile-allocation memory, address and size (binner only) |
| `V3D_CLE_0_CT0QTS` (+ `CTQTSEN`) | `0x15c` (`:279-280`) | tile state data array address (binner only) |
| `V3D_PTB_0_BPCA` / `BPCS` | `0x300` / `0x304` (`:290-291`) | where the binner is allocating now |
| `V3D_PTB_0_BPOA` / `BPOS` | `0x308` / `0x30c` (`:292-293`) | the next overflow block it takes on OUTOMEM |
| `V3D_CTL_0_INT_STS` FLDONE / FRDONE / OUTOMEM | bits 1 / 0 / 2 (`:256-258`) | bin done, render done, binner out of memory |
| `V3D_CTL_0_INT_MSK_SET` / `MSK_CLR` | `0x60` / `0x64` (`:253-254`) | interrupt masks |

The bin and render threads have separate base and end registers, and the
binner alone takes QMA, QMS and QTS (`v3d_submit_bin`,
`driver/src/v3d/v3d_hw.c:401-419`; `v3d_submit_render` writes only CT1QBA and
CT1QEA after a cache invalidate, `:421-430`). Programming CT0 while CT1 runs
is therefore a matter of which memory each one points at.

### What the packets fix

- The binner writes per-tile lists into the tile-allocation memory, starting
  with an initial block per tile. Its block sizes are fields of **Tile
  Binning Mode Cfg** (packet 120, V3D ≤ 4.2): `tile allocation block size`
  and `tile allocation initial block size`
  (`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:943-965`).
  yakogl sets them to 128 B initial and 64 B chained
  (`driver/src/v3d/v3d_frame.c:154-155`, `:164-165`).
- The renderer reads those lists from the address in **Multicore Rendering
  Tile List Set Base** (packet 123, field `address`, 26 bits at bit 6,
  `v3d_packet.xml:1179-1182`). yakogl writes `f->tile_alloc` there
  (`v3d_frame.c:269-272`). **The tile-alloc address is baked into each
  job's RCL**, so job N's render reads job N's tile lists until FRDONE. A
  bin of N+1 into the same area would overwrite them while N renders.
- **Tile List Initial Block Size** (packet 126, `v3d_packet.xml:1187`) is
  emitted in the RCL with auto-chained lists (`v3d_frame.c:264-267`), so
  the render follows chains into the overflow blocks. Those blocks must
  stay untouched until the render that walks them is over.
- **Increment Semaphore** and **Wait on Semaphore** (packets 7 and 8,
  `v3d_packet.xml:432-433`) exist. yakogl emits neither: no `SEMAPHORE` in
  `driver/src/v3d/*.c`. Bin-to-render order comes from FLDONE, as in the
  Linux driver.

### What the Linux v3d driver and Mesa do (recalled)

- **Recalled:** the kernel (`drivers/gpu/drm/v3d`) splits a
  `DRM_IOCTL_V3D_SUBMIT_CL` into a bin job and a render job. They go on
  separate `drm_sched` queues (`V3D_BIN`, `V3D_RENDER`; there are also
  `V3D_TFU`, `V3D_CSD` and `V3D_CACHE_CLEAN`).
  - The render job depends on its own bin job's done fence, and each queue
    runs one hardware job at a time.
  - Bin job N+1 is therefore free to start as soon as bin N is over, while
    render N still runs, unless it carries an in-fence (`in_sync_bcl`).
  - The render queue starts render N+1 after render N's FRDONE interrupt
    and bin N+1's FLDONE.
  - FRDONE, FLDONE and OUTOMEM are all unmasked; `v3d_irq` signals the
    fences.
- **Recalled:** `v3d_bin_job_run` writes CT0QMA/QMS (from the submit's
  `qma`/`qms`), CT0QTS with CTQTSEN and the submit's `qts`, then CT0QBA and
  CT0QEA. It also invalidates the caches at the bin's start, as yakogl does
  (`v3d_hw.c:413`), while a render may be running. `v3d_render_job_run`
  writes CT1QBA and CT1QEA.
- **Recalled:** Mesa (gallium `v3d`, `v3d_job.c` `v3d_job_submit`)
  allocates **a tile-alloc BO and a tile-state BO per job**, sized by
  `v3d_tile_alloc_sizes` (`*tile_state_size = layers * tiles_x * tiles_y *
  256`, quoted in the tree at `src/v3d/backend.rs:3342-3348`). Every bin has
  its own memory, and overlap needs no extra rule.
- **Recalled:** the binner overflow pool is the kernel's. On OUTOMEM,
  `v3d_overflow_mem_work` allocates a 256 KiB BO and writes BPOA/BPOS. The
  previous overflow BO is attached to the **bin job's render**, so the
  memory lives until that render ends. The OSAL copies the 256 KiB size
  (`zephyr_v3d.c:110-112`; `zephyr/Kconfig:272-280` "as the Linux driver
  does").
- **Recalled:** when a job's binning must see an earlier job's results,
  Mesa gives the bin an in-sync on the previous job's out-sync
  (`in_sync_bcl`). Examples: transform-feedback output read by a later
  draw, or a vertex or coordinate shader sampling a texture the previous
  job rendered. Otherwise only the render waits (`in_sync_rcl`). The exact
  condition Mesa tests was not checked here.

## (c) What yakogl would need

### 1. A second tile-alloc and tile-state area, chosen by the backend

The backend allocates **one** tile-alloc and **one** tile-state area at init
(`src/v3d/backend.rs:1203-1206`, sized at `:6399-6412`). Every kick names
them (`:3384-3391`, `tile_alloc: g.tile_alloc.gpu`, `tile_state:
g.tile_state.gpu`), and every RCL embeds the tile-alloc address (`:2704`;
`v3d_frame.c:269-272`). The OSAL cannot swap areas behind the backend's
back. The backend must pick area A or B per job and write it into both the
kick and the RCL, the way it already alternates the list arenas (`arenas` and
`spare`, `backend.rs:3111-3119`; `holder`, `:3129-3134`). The OSAL's kick
already carries both addresses (`src/v3d/kick.rs:29-32`).

Sizes, from `v3d_frame_alloc_sizes` (`driver/src/v3d/v3d_frame.c:118-128`):
`tiles x 128` B rounded to 4 KiB, plus 8,192 B, plus `min(tiles x 128 x
draws / 2, 524,288)` rounded to a page. Tile state is `tiles x 256` B. The
draw budget is 1,024 (`zephyr/Kconfig:259-262`; `DEFAULT_DRAWS_PER_JOB`,
`backend.rs:61`).

| sizing | tiles | tile alloc B | tile state B | second area, B |
|---|---:|---:|---:|---:|
| as today: `MAX_TARGET` 4096 at 4 RTs, 32x32 tiles (`backend.rs:447`, `:6399-6411`) | 16,384 | 2,629,632 | 4,194,304 | **6,823,936** |
| terrain 800x600, 1 RT 32 bpp, 64x64 tiles (130 tiles, `v3d-terrain-gpu.md:103-111`) | 130 | 552,960 | 33,280 | **586,240** |
| terrain 256x256 targets (jobs 1, 2, 4, 5) | 16 | 536,576 | 4,096 | 540,672 |

- The Pi 4 GM image's carve-out is 96 MiB
  (`driver/samples/zephyr/glmark2/boards/rpi_4b.conf:12`; Kconfig default
  48 MiB, `zephyr/Kconfig:371-374`). A second worst-case area is 6.5 MiB of
  it, or 13.6 % of the default 48 MiB.
- A per-job size is possible (Mesa's way) but needs an allocation per job.
  The two fixed areas are simpler and cost the 6.8 MiB once.
- Either way `job_tile_state_bytes` (`backend.rs:3349-3354`) keeps sizing
  the `memset`.
- **Side effect worth having:** the tile state of N+1 can be zeroed at its
  submit, into the idle area, while N renders. Today it is zeroed in
  `start_job` (`zephyr_v3d.c:1431`), which a waiting job reaches only after
  N is finished, so the GPU is idle during it. This moves candidate 3's
  `memset` (`pi4-terrain-cpu-left.md:269-276`) off the critical path for
  every overlapped job.

### 2. The binner overflow pool

- The pool is `CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS` blocks (default 2,
  `zephyr/Kconfig:272-276`) of 256 KiB, taken at init (`zephyr_v3d.c:269-286`).
- `start_job` resets `s.ov.used = 0` at every bin start (`:1450`), and the
  ISR hands out `s.ov.blocks[s.ov.used++]` (`:1021-1025`).
- With overlap, bin N+1 would get the same blocks that render N is still
  walking through its chained tile lists (packet 126, above).
- **Needed:** each job owns the blocks its bin took until its render
  completes, as the kernel ties them to the render (recalled). Two simple
  ways:
  - split the pool in two halves, one per area, so each half has one block
    by default; or
  - keep a free list that `finish_inflight` refills at the job's completion
    (`:1313`).
- A bin that finds its half dry still fails by name (`-ENOMEM`, `:1026-1029`).
- The `s.bin` pool record (`:1035-1046`) stays per bin.

### 3. Per-stage in-flight state and the ISR

- **One in-flight record today.** `s.if_*`, `s.bin_pending` and `s.bin_rc`
  (`:135-144`) describe the single job in flight.
- **Two records needed.** Overlap needs a **binning** record (bin N+1: its
  fence, `if_c_bin`, `if_c_fldone`, RCL range, tile pool and overflow
  share) and a **rendering** record (render N: fence, `if_c_render`, lists
  for the failure line).
- **The ISR at FLDONE of N+1 must not start render N+1 while render N runs.**
  Today it calls `v3d_submit_render` unconditionally (`:1048`). With
  overlap, FLDONE of N+1 only marks "bin done, render ready". The render
  starts at the earliest of:
  - `finish_inflight(N)` right after `tfu_queue_run` (`:1311`), in place of
    `start_pending` at `:1316`; or
  - the ISR, when render N is already over **and** has no TFU behind it.
    That needs FRDONE unmasked, see (a).
- **`bin_finish` failure path (`:1160-1222`).** A failed bin resets the GPU
  (`recover`, `:1054-1069`), which kills a render in flight. Both jobs
  then complete with the error, in fence order.
- **`inflight_ready` (`:1228-1240`)** must test FRDONE for the rendering
  record and the bin status for the binning record separately.

### 4. When a bin may overlap: a kick bit

The OSAL cannot know whether bin N+1 reads what render N writes. The backend
does. Terrain's case: job 3's coordinate shader samples twice
(`v3d-terrain-gpu.md:194`, "2 LINEAR (`terrain.vert:40,45`), run twice: the
coordinate shader at binning and the vertex shader at render"). The textures
are the height and normal maps rendered by jobs 1 and 2
(`v3d-terrain-gpu.md:104-106`; which sampler reads which map is recalled from
glmark2's `terrain.vert`, not in the tree).

So the kick needs a bit, for example in `V3dKick::pad` (`kick.rs:33`, always
0 today, `backend.rs:3391`). The backend sets it when any draw's vertex stage
samples a texture, or the job uses transform feedback, or anything else a
bin reads that an outstanding job writes. The conservative first rule:
overlap only when **no** vertex or coordinate shader of the job samples a
texture. The OSAL then holds the bin as today when the bit is set.

### 5. The order of fences

Per job N, today (`finish_inflight`, `:1254-1318`):
FRDONE → `v3d_caches_clean` (`:1302`) → `tfu_queue_run(N)` (`:1311`) →
`complete(N)` (`:1313`) → start of the waiting job (`:1316`).

With overlap:

1. **bin N+1** may start at submit when the area and overflow share it
   needs are free and its kick allows it. It is independent of N's
   FRDONE, clean and TFU.
2. **render N+1** starts only after all of render N's FRDONE, N's clean,
   N's queued TFU jobs (`tfu_queue_run`; the TFU must not run beside a
   render, `zephyr_v3d.c:1341-1343`, and render N+1 may sample the chain),
   and bin N+1's FLDONE.
3. **fence N** completes as today, before N+1's render starts; the fence
   order is unchanged (`complete`, `:1144-1151`, keeps the last two results
   for `z_wait`, `:1525-1527`).
4. **area A** (N's) is free again at `complete(N)`. A bin of N+2 into A may
   start then. The backend's `holder` check (`backend.rs:3129-3134`) already
   models "the outstanding job that holds this set".
5. **The TFU and a running binner.** `osal_v3d_tfu` forbids the TFU only
   beside a render (`:1341-1343`). A bin that samples no texture (the
   kick bit clear) touches neither the TMU nor the L2T, so it may run
   beside a queued TFU job. A bin with the bit set never overlaps anything.
   This rule is an inference from the comment at `:1341-1343`; no board run
   checked it.
6. **`osal_v3d_tfu` itself** (`:1328-1348`) calls `finish_all`, which today
   ends both jobs. With overlap it must also end a bin in flight (or let it
   finish) before running a lone TFU job.

### 6. The TFU queue and the recent-jobs ring

- **`tfu_q`** (`:1097-1103`) tags each TFU job with the fence it follows:
  the newest, waiting or running (`:1610-1616`). That stays correct. The
  queue drains at render N's end (`:1311`) and gates render N+1 (step 2).
  It does not gate bin N+1.
- **The ring** (`note_job_done`, `:759-784`):
  - `idle` is `s.if_c_bin - recent_prev_end`, clamped to 0 when negative
    (`:769`, `:778`). An overlapped bin starts before the previous job's
    end, so its `idle` reads 0, as the clamp already handles.
  - `bin` (`if_c_fldone - if_c_bin`, `:1174`, `:1269`) and `prep`
    (`c_bin - c0`, `:1434`) need per-stage stamps (item 3). Otherwise the
    second job overwrites the first's.
  - A useful new field is the wait between FLDONE and the render's start
    (`if_c_render - if_c_fldone`), which overlap makes non-zero. It says
    how much of the bin was hidden.
  - `render_late` (`:1275-1276`) is unchanged.

## (d) Expected gain at terrain

**No note in `docs/notes` gives a per-job bin time.** pi4-terrain-cpu-left
lists binning as "1 to 5, of which job 3 most; estimate; not measured, no
record has it" (`pi4-terrain-cpu-left.md:212`). The board records do
print one figure, the **last** completed job's `bin` and `prep` per frame on
the `glmark2-frame` line (`main.cpp:1366-1367`, summed at `:644-646`).
pi4-slow-terrain quotes only that `prep` (2.56 ms,
`pi4-slow-terrain.md:521`), not the `bin`. The records are gitignored.

**The estimate and its basis.** Terrain's 8 jobs a frame
(`v3d-terrain-gpu.md:103-111`, job 0 gone since `4255adf1`):

| job | bin content | may overlap the previous render? | basis |
|---|---|---|---|
| 1 height map | one quad | no: the frame's first job, submitted after the swap's wait (every swap waited for, `pi4-slow-terrain.md:554`) | |
| 2 normal map | one quad, no vertex texture | yes, with render 1 | `v3d-terrain-gpu.md:105` |
| 3 terrain | 131,072 triangles, **coordinate shader samples the maps of jobs 1 and 2** | **no** | `v3d-terrain-gpu.md:106`, `:194` |
| 4, 5 bloom | one quad each | yes (with render 3 and its TFU chain A, see (c) 5) | `:107-108` |
| 6 overlay | one quad | yes | `:109` |
| 7 tilt-shift H | one quad | yes | `:110` |
| 8 window | one quad | yes | `:111` |

- **The bins that can hide** are six one-quad bins (jobs 2, 4 to 8). Each
  is the CLE start, the binner's initial block writes for the job's tiles
  (16 or 130), and the FLDONE interrupt's latency before the render
  starts. With nothing in the tree to measure, the estimate is **20 to
  100 µs each, 0.1 to 0.6 ms a frame**.
- **Plus the `memset`.** The tile-state `memset` of those six jobs moves
  under the GPU (item c.1). That is up to 0.07 to 1.5 ms of the 0.07 to
  1.8 ms `pi4-terrain-cpu-left.md:178` gives for all eight jobs.
- **Total: 0.2 to 2 ms a frame, most likely under 1 ms.** The 1 to 4 ms of
  `pi4-terrain-cpu-left.md:252` assumed job 3's bin hides. It cannot
  without a reorder.
- **Job 3's bin** is the large one. 393,216 vertices go through the
  coordinate shader (`pi4-terrain-cpu-left.md:36-38`), and it may well be
  1 to 3 ms. Hiding it would need a bin that does not wait for job 2. One
  example is a terrain job whose coordinate shader reads only the height
  map, which job 1 renders. That is not this fix.

**The board values that confirm it** come from the ring
`osal_v3d_recent_jobs_us` (`zephyr_v3d.c:786-806`). It carries `bin` and
`prep` per job (`:772-773`; struct `src/osal/zephyr_v3d.h:117-123`).
`print_jobs` (`main.cpp:1402-1420`) prints only `total`, `render`, `idle`
and `tfu`. So the first step is a print change: add ` bin B prep P` to
each `glmark2-jobs` entry. Then read frame 10 of terrain:

- **Before the fix:** the sum of `bin` over jobs 2 and 4 to 8 is the
  ceiling of the bin part of the gain. Job 3's `bin` says what a reorder
  could add. Each overlappable job's `prep` (only the `memset` when the GPU
  is otherwise idle, `pi4-terrain-cpu-left.md:20-28`) is the ceiling of the
  `memset` part.
  - If the six `bin`s sum to under 0.2 ms and the `prep`s show the
    `memset` under 0.1 ms a job, the overlap is not worth its 6.8 MiB and
    candidate 1 should drop below early Z (`v3d-early-z.md:220`).
- **After the fix:**
  - `glmark2-frame` terrain `frame` and `glmark2-counters` `wait.ns` a
    frame fall by about that sum.
  - The `bin` of jobs 2 and 4 to 8 is unchanged, but the gap between their
    render start and the previous job's end goes to the clean plus the TFU
    alone.
  - `idle` stays 0.
  - The job count (8.003 a frame, `pi4-slow-terrain.md:554`) and the
    render sum are unchanged.

## (e) Files a fix would touch, owners, and a host test

### Files and owners

| file | what changes | held by |
|---|---|---|
| `src/osal/zephyr_v3d.c` | two-stage in-flight records, the ISR's FLDONE rule, per-job overflow shares, `start_job` split into bin and render, the fence order of (c) 5, ring stamps | not in any plan's list in the plan text; plan #321 holds `driver/src/v3d/**` and `driver/zephyr/lib/board/**`, not `src/osal` |
| `src/osal/zephyr_v3d.h` | ring struct field (FLDONE-to-render wait), if added | same as above |
| `src/v3d/backend.rs` | second tile area (alloc at `:6399-6412`), per-job choice written into the kick (`:3384-3391`) and the RCL (`:2704`), the overlap bit from the job's shaders | **plan #308** (`src/v3d/**`) |
| `src/v3d/kick.rs`, `driver/src/v3d/v3d_kick.h` | the flag in `pad` | #308 (Rust side); **plan #321** (`driver/src/v3d/**`) for the C header |
| `driver/src/v3d/v3d_hw.c` | none needed: `v3d_submit_bin` and `v3d_submit_render` take addresses already (`:401-430`) | #321 |
| `driver/samples/zephyr/glmark2/src/main.cpp` | `bin` and `prep` in `print_jobs` (the confirming print; can land first, alone) | **plan #328** (`driver/samples/zephyr/glmark2/**`) |
| `zephyr/Kconfig` | the overflow split or a block count per area (`:272-280`) | not named in the plan text |

### Host test design

The tree has no host build of `zephyr_v3d.c` at all. `tools/v3d/osal_frees_check.c`
and `osal_granules_check.c` test the headers it uses (`v3d_frees.h`,
`v3d_granules.h`), and `tools/v3d/hw_mock_test.c` tests only
`driver/src/v3d/v3d_hw.c` (`hw_mock_test.c:1-21`; run by `tools/check.sh:2675-2682`).
The register model in `hw_mock_test.c` completes a job **at the register
write**:

- FLDONE is set when CT0QEA is written (`hw_mock_test.c:205-217`).
- FRDONE is set when CT1QEA is written (`:226-227`).
- Time is a counter that `mock_busy_wait` advances (`:232-235`).

That cannot show an overlap. A test of this fix needs:

1. **A host build of `zephyr_v3d.c`** on `tools/v3d/hostshim/zephyr`,
   which today has `kernel.h`, `logging` and `sys`. Grow it with the
   following, or move the job-queue logic (`z_submit`, `start_job`,
   `finish_inflight`, `v3d_isr`, `start_pending`, `tfu_queue_run`,
   `note_job_done`) into a header the host compiles alone, as the frees
   rule was (`v3d_frees.h`). The second is less code to stub and is the
   recommended path.
   - `k_cycle_get_32` and `k_cyc_to_us_floor32` on the model clock
   - `irq_lock` / `irq_unlock`
   - `IRQ_CONNECT` / `irq_enable`, recording the handler
   - `device_map`, `k_mem_map_phys_bare` and `k_mem_phys_addr` over a
     host buffer
   - `barrier_dmem_fence_full`, `printk`, `snprintk`
2. **A model with two independent CLE threads on a clock.**
   - A CT0QEA write starts a bin of configured length (µs) that sets FLDONE
     when the clock passes its end, or OUTOMEM after `oom_blocks`.
   - A CT1QEA write starts a render that sets FRDONE at its end.
   - CT0 and CT1 each keep their own busy flag and the QMA/QTS/tile-list
     base they were started with.
   - A `mock_busy_wait` or `k_busy_wait` call advances the clock and,
     when a bit with an unmasked interrupt rises, calls the recorded ISR.
     This is the step that makes the ISR's start-render path testable.
   - The TFU (`TFU_CS`) is modelled the same way, with a length.
3. **Checks the model can make:**
   - **No tile-memory sharing.** A CT0QEA write while CT1 runs must carry a
     CT0QMA and CT0QTS different from the tile-list base the running RCL
     reads. The model can take that base from the kick's area pairing.
   - **No render beside a render or a TFU.** A CT1QEA write while CT1 runs
     or `TFU_CS` is busy is a failure.
   - **Render N+1 after N's clean and TFU.** The CT1QEA of N+1 comes after
     N's `L2TCACTL` clean writes and N's TFU jobs, in the write log
     `wlog` (`hw_mock_test.c:68-76`).
   - **Overflow ownership.** A BPOA given to bin N+1 is never a block that
     job N's bin took, until N completed.
   - **The kick bit holds the bin.** With the bit set, CT0QEA of N+1 comes
     after `complete(N)`. This is today's order byte for byte, so the same
     test with every bit set pins that nothing changed.
   - **Timing.** With bins of 50 µs and renders of 1 ms over eight jobs,
     the model's end time falls by 7 x 50 µs when bits are clear and is
     unchanged when they are set. The ring's `idle` is 0 for overlapped
     jobs and `bin` is 50 µs for each.
   - **Fence order.** `z_wait` results stay as the last-two rule
     (`zephyr_v3d.c:1525-1527`).
4. **On the Rust side**, a `src/v3d/backend_tests.rs` case (plan #308).
   - Terrain's kick bits: set for job 3, clear for the quad jobs. The
     `v3d-terrain-job-pin` test harness already runs terrain on the V3D
     stand-in.
   - The two areas alternate per arena set, and each RCL's Tile List Set
     Base equals its kick's `tile_alloc`.
   - ES 2.0 answers stay the same (`tests/es2_report_guard.rs`), because
     the host OSAL (`src/osal/linux.c`) completes jobs at submit and sees
     no overlap.

## Limits

- Every gain figure in (d) is an estimate: the per-job `bin` was never read
  on the board. The confirming print is a one-line change in plan #328's
  file.
- The Mesa and kernel details are recalled, not read from a source tree.
- Which sampler of `terrain.vert` reads which map is recalled from
  glmark2's source. The tree only says the coordinate shader makes two
  texture fetches (`v3d-terrain-gpu.md:194`). If neither fetch reads job
  2's output, job 3's bin could overlap render 2, and the gain could reach
  the 1 to 4 ms of `pi4-terrain-cpu-left.md`.
- That a cache invalidate at a bin's start (`v3d_hw.c:413`) is harmless to
  a running render is recalled from the kernel doing the same. The board
  has not run it.
