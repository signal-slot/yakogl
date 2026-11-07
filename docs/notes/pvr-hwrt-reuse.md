<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P terrain: the HWRT and kick-ring holds in the PowerVR mipmap chain

Written 2026-11-07 from the code at `d83e25b5`. The files cited are
byte-identical at main `723a75c8`, so the line numbers hold for both. No code
change and no board run. This is the lead that `pvr-tq-mipmap.md` ("What the
host spends between kicks today") leaves open: does a terrain frame's
mipmap chain wait on the host in `hwrt_get` (an HWRT dataset in flight) or
in the geometry UFO ring, and what does it cost?

**Answer: at main, neither hold fires in a steady terrain frame. HWRT
waits: 0. Ring waits: 0. HWRT re-initialisations: 0.** The backend's
frame-set ring holds every kick first, and it is smaller than both limits.
More HWRT members, a larger ring or per-size keying would gain nothing on
their own. The HWRT limit was a real cost until `ce3354ac`; it becomes one
again if the frame sets grow past the HWRT members. The one host hold
terrain's kicks meet is the frame-set reuse wait. It is ranked at the end
with the record that would confirm it.

The waited-for kicks of path B and the 20 ms re-post are
`pvr-waited-kick.md`'s, not repeated here.

## The three host limits on a kick, in the order a kick meets them

1. **Frame-set reuse (the backend).** Kick N is recorded into frame set
   `N mod nsets`. Opening that frame waits for the fence of the kick that
   last used the set, kick N - nsets (`src/pvr/backend.rs:5283-5291`; the
   fence is stored at `:9061`). `nsets` is `CONFIG_YAKOGL_PVR_FRAME_SETS`,
   default 4 and at most 4 (`zephyr/Kconfig:99-103`;
   `PVR_BACKEND_FRAME_SETS` = 4, `backend.rs:271`, enforced at
   `:9339-9341`). The AM62P glmark2 image does not set it
   (`driver/samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.conf`,
   `prj.conf`). The wait is `wait_fence` (`backend.rs:2327-2342`) ->
   `z_wait` (`src/osal/zephyr.c:636-648`) -> `pvr_render_wait`
   (`driver/src/pvr_render.c:778-783`, `who` = "frame fence") ->
   `wait_for` (`driver/rust/src/gpu/pvr/render.rs:2327`). So when kick N
   reaches `submit`, kick N - 4 has completed and at most three kicks
   (N-3 .. N-1) are in flight.
2. **HWRT member in flight.** `submit` -> `hwrt_get` (`render.rs:1389`,
   `:703-772`). A size has at most `HWRT_MEMBERS` = 4 datasets, built in
   pairs (`render.rs:85`). The next member of each of the size's pairs is
   handed out once its last kick has completed (`:730-735`). A third and
   fourth member are built while every member seen is in flight and the
   size is below 4 (`:744-750`). Only then does it wait for the oldest
   member's last kick (`:751-770`, `wait_for(.., "HWRT member in flight")`).
3. **Geometry UFO ring throttle.** `GEOM_UFO_SLOTS` = 16 (`render.rs:54`).
   Kick N waits for kick N - 15 when that kick has not completed
   (`render.rs:1392-1399`).

A fourth path re-initialises a dataset instead of waiting: **pair recycle**.
When the size has no pair and none of the 16 pairs (`HWRT_SETS` = 32,
`render.rs:74`) is free, the least recently kicked pair is waited for and
rebuilt for the new size (`render.rs:773-865`, then the buffers and the
firmware structures at `:867-1000`). A member handed out again on the fast
path (`:734`, `:770`) returns before any of that, so it keeps its buffers,
region headers and firmware HWRT data. The free lists are shared by every
pair and are never touched here (`:813-816`).

## Terrain's kicks a frame, by target size

From glmark2's source (the tree the host harness builds,
`tools/hostharness/glmark2/README.md`; `src/scene-terrain.cpp:59-165`,
`:332-338`, `scene-terrain/*.cpp`), with its
defaults bloom and tilt-shift on. Each line below is one kick
(`record_mip_level`: one kick a level, at level 1's size for every level of
the chain, `backend.rs:6124-6131`, `:6151-6152`, `:6268-6269`):

| Step | Target | Kicks |
|---|---|---:|
| height map, normal map | 256x256 | 2 |
| terrain (offscreen, depth) | 800x600 | 1 |
| its chain, levels 1-9 | 400x300 | 9 |
| bloom horizontal | 256x256 | 1 |
| its chain, levels 1-8 | 128x128 | 8 |
| bloom vertical | 256x256 | 1 |
| its chain, levels 1-8 | 128x128 | 8 |
| overlay into the terrain target | 800x600 | 1 |
| terrain's chain again | 400x300 | 9 |
| tilt-shift horizontal | 800x600 | 1 |
| its chain | 400x300 | 9 |
| tilt-shift vertical (the window) | 800x600 | 1 |
| one more kick before the next frame's height map (host run below) | taken as 800x600 | 1 |
| **total** | 4 sizes | **52** |

The host harness confirms the order and the count:
`GM2_KICKS=1 GM2_WRITE=0 GM2_FRAMES=4 GM2_ENTRIES='^terrain$'
tools/hostharness/glmark2/run.sh <glmark2> build/gm2-hwrt powervr` at
`d83e25b5`. Its `gm2: draw` lines step through framebuffers
1, 2, 4, 5, 6, 4, 7, 0 a frame, at kick ids 136, 137, 138, 148, 157, 166,
176, 186, then 188 for the next frame's first draw. So a frame is 52 kicks:
9 between terrain and bloom, 8 after each bloom pass, 9 after the overlay
and after tilt-shift, and one kick (187) after the window's draw that holds
no glmark2 draw. The 43 chain kicks also match the "43 of its 53 kicks a
frame" counted on the board (`backend.rs:6213-6215`;
`am62p-pvr-mipmap-kicks.md`, "The cost", 53 at `d3b8d4b2`). The extra kick's
size is not printed. The model takes it as 800x600; the counts at main do
not depend on it (the argument under the table holds for any size). The 512x512 specular map and
its chain (level 1 256x256) are rendered in the scene's setup only
(`scene-terrain.cpp:300`).

## The counts: a model of the three limits

The glmark2 host harness cannot measure these holds. Its OSAL
(`src/osal/linux.c`) completes every submit at once and never runs
`driver/rust/src/gpu/pvr/render.rs`. The counts therefore come from a
replay of the code above: `hwrt_get`'s loop line for line (`:716-772`, the
recycle `:779-792`), the throttle at `:1394-1398`, and the frame-set rule
from limit 1, fed with the 52 kicks of the table for 20 frames. The case
taken is the worst one for the holds: the GPU slower than the CPU, so at
each submit only the kicks before N - nsets have completed. The board
supports this case: GL time is 81.9 of terrain's 83.06 ms a frame
(`tools/cts/RESULTS.md:274-276`), so the CPU spends the frame waiting. The
model is a scratch script outside the tree. Its rule is in this paragraph,
and `render.rs`'s own mock test pins the first frame's behaviour:
`a_size_with_every_member_in_flight_takes_a_second_pair`, `:3958-3985`.

Counts a steady frame (in brackets: the first frame of the scene):

| frame sets | HWRT_MEMBERS | (a) HWRT waits | (b) ring waits | (c) builds / recycles |
|---:|---:|---:|---:|---|
| **4 (main)** | **4 (main)** | **0** | **0** | **0 / 0** (12 / 0) |
| 4 | 2 (before `ce3354ac`) | 37 | 0 | 0 / 0 (8 / 0) |
| 5 | 4 | 23 | 0 | 0 / 0 |
| 6 or 8 | 4 | 31 | 0 | 0 / 0 |
| 8 | 8 | 0 | 0 | 0 / 0 (20 / 0) |
| 12 | 12 | 0 | 0 | 0 / 0 |
| 16 | 16 | 10 | 0 | 0 / 0 |
| 17 | 16 | 10 | 27 | 0 / 0 |

Why main has none:

- **(a) HWRT.** At each submit at most three kicks are in flight (limit 1),
  and a size has four members. So at least one member's last kick has
  completed. If that member is the newer one of its pair, the pair's older
  member is complete too. Either way, the member `hwrt_get` looks at first
  in that pair (`next`, the older one, `:730`) is free. The code says the
  same in `render.rs:76-79`: "a size with four members never waits for one
  of its own renders before the back end's frame-set reuse would wait
  anyway". The waits come back as soon as the frame sets exceed the members
  (rows 5-8 with 4 members). They also come back when 4 sizes x members
  exceeds the 32 datasets of `HWRT_SETS` (row 16/16: the pairs run out, and
  `hs.is_none()` at `:750` sends the size to the wait).
- **(b) Ring.** Kick N - 15 completed long before kick N - 4 did. The ring
  binds only with 17 or more frame sets.
- **(c) Re-initialisation.** Terrain uses 4 sizes (256x256, 800x600,
  400x300, 128x128), at most 2 pairs each: 8 of the 16 pairs. No steady
  frame builds or recycles a set. The first frame builds 12 members
  (`:867-1000`: buffer allocation, about 350 bytes of firmware structures
  written and flushed, no firmware round trip with `hwrt_cleanup` off, its
  default, `driver/src/pvr_render.c:454`). A size that a scene before
  terrain left in the pool may be recycled once at the scene's start
  (least recently kicked, so long complete). That cost is per scene, not
  per frame. A free set of the same size is always reused as it is: the
  member's buffers are kept when they are big enough (`:880-912`), and its
  HWRT data is rewritten in place, not cleared (`:959-966`, `hwrt_reinit`
  off).

## What each hold costs

| Hold | file:line | Count a terrain frame (main) | ms a frame | Source of the ms |
|---|---|---:|---:|---|
| HWRT member in flight | `render.rs:749-772` | 0 | 0 | the count. When it fired (2 members, 37 a frame) it cost about 7.9 ms a frame, about 0.21 ms a wait: terrain went 90.94 -> 83.06 ms between `86d0b482` and `6a7021ec`, the first board run since `ce3354ac`, while the other 28 scenes moved by 0.15 ms or less (`tools/cts/RESULTS.md:274-278`; the attribution assumes that commit is the only one that moved terrain) |
| Geometry UFO ring throttle | `render.rs:1392-1399` | 0 | 0 | the count |
| HWRT pair recycle / re-init | `render.rs:773-1000` | 0 (12 builds in the scene's first frame) | 0 | the count. A build is CPU writes of a few hundred bytes, microseconds (no firmware command with `hwrt_cleanup` off) |
| *Frame-set reuse (backend)* | `backend.rs:5288-5291` | about 47 (every kick once four are queued) | at most 0.25 ms a kick when the queue drains, otherwise 0; not measured | see below |

The frame-set wait is the binding host hold, and its cost is the 1 ms poll.
Without `pvr-fine-wait`, `wait_for` reads the fragment UFO once a
millisecond (`render.rs:2018-2028`, `:2333-2453`). So a wait ends up to
1 ms after its fence moves (`pvr-fence-wait.md`). While it waits, three
kicks are still queued. At the queued price of about 1.15 ms a kick
(gl_test case 216: 40 kicks in 46.15 ms, `RESULTS.md:300-302`), that is
about 3.5 ms of work, more than the 1 ms poll, so the GPU should not run
dry and the wait costs CPU time, not frame time. A drained queue would show
as at most one poll quantum per four kicks: under 0.25 ms a kick, under
11 ms over terrain's 43 chain kicks. That is an upper bound from the rule,
not a measurement. This hold does not explain the 0.96 ms a kick spent
outside the hardware (`pvr-tq-mipmap.md`, "The cost being attacked"):
at the poll's worst it accounts for a quarter of it.

## Fix candidates, ranked

Memory below is per dataset (`render.rs:867-912`, the same at all four of
terrain's sizes, which are all under the minimums): TPC 16 KiB, VHeap
8 KiB, region headers 64 KiB and macrotile 4 KiB from the GENERAL heap,
the MList 4 KiB from the PM-protected heap, and 128 + 256 bytes of firmware
heap (`HWRTDATA_COMMON` 88 and `HWRTDATA` 256 bytes, each padded to its own
128-byte line, `:52`, `:410-412`). That makes about 96 KiB a member,
allocated only when a size first uses it. A frame set is about 1872 KiB of
the render pool at the defaults (`zephyr/Kconfig:104-116`: 4 sets are
7488 KiB).

1. **Measure the frame-set wait's poll first (no code).** This is arm 0 of
   `pvr-tq-mipmap.md` ("The smallest board experiment"): gl_test case 216
   and glmark2 terrain on two images of one main commit, one with
   `CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT=y`. It costs no memory and is a
   board session. Confirming record value: `gl_test: mipchains: ... ms a
   frame` and terrain's ms a frame in the GM record. The prediction is a
   drop under 1 ms (the queue does not drain). A drop of 4 ms or more on
   case 216 (0.1 ms a kick of 40) would say the queue drains, and then
   fine wait (or a shorter poll for the frame-set wait alone) is the fix.
2. **A deeper queue, as one change: frame sets 4 -> 8 and `HWRT_MEMBERS`
   4 -> 8.** The two must move together. 8 sets with 4 members gives 31
   HWRT waits a frame, each costing a poll quantum (the model above, and
   the `ce3354ac` history). The ring of 16 and the 32 datasets
   (4 sizes x 8) are enough up to 8 sets. Memory: 4 more frame sets, about
   7.3 MiB of the render pool (the pool budget is `pvr-render-pool.md`'s),
   plus 4 more members for each of terrain's 4 sizes, 16 x 96 KiB, about
   1.5 MiB. Code: `PVR_BACKEND_FRAME_SETS` and the Kconfig range
   (`backend.rs:271`, `zephyr/Kconfig:102`) and `render.rs:85`. These are
   #325's and #301's files. It helps only if the firmware or the GPU
   overlaps more than three queued kicks. The kick-times run says
   fragment phases are already back to back (`pvr-fence-wait.md`, "The
   kick-times run"). So the predicted gain is small, unknown in size.
   Confirming record value: case 216's ms a frame and terrain's ms a frame
   at 8/8 against 4/4 from the same commit. The pvr-kick-times line's
   `frag->frag` should fall if the deeper queue helps, and the
   `"render: ... still busy"` count (`render.rs:758`) should stay 0.
3. **More HWRT members, keying sets by size, or a larger ring, alone: no
   gain on terrain at main.** All three holds count 0. Sets are already
   keyed by (width, height, samples) (`:719`), and the chain already uses
   one size for all its levels (`backend.rs:6124-6131`). A larger ring
   (`GEOM_UFO_SLOTS` 16 -> 32) costs 64 bytes of firmware heap but moves the
   timeline word that follows the slots (`render.rs:1425`) and the
   kick-times ring (`kick_times.rs`, the `KT_RING` test at
   `render.rs:3663`). It is needed only past 16 frame sets. Confirming
   record value for "no gain": terrain's ms a frame with `HWRT_MEMBERS` = 8
   against 4, within the run-to-run spread (0.15 ms on the other scenes of
   `RESULTS.md:277-278`).

## What no record shows yet

- No board record counts `wait_for` calls by caller. `hwrt_busy_reuse`
  (`render.rs:754`) counts only waits that timed out, and the backend's
  `waits` (`backend.rs:2331`, `pvr_backend_counters`) is not printed by
  the AM62P glmark2 image (`driver/samples/zephyr/glmark2/src/main.cpp`
  prints the GC7000's waits only). So the zeros above come from the code
  and the model, not from a count on the board. A counter per `who`, for
  example a per-caller entry in `wait_for`'s `kick_times::wait_begins`,
  would turn them into a record value. That is code in
  `driver/rust/src/gpu/pvr/`, #301's.
- The host run counts 52 kicks a frame and the board counted 53 at
  `d3b8d4b2`. The board's extra kick was not identified. The size of
  kick 187 and the window's sample count are not in the `GM2_KICKS` lines,
  so the model takes 800x600 and one sample for both. None of the zeros at
  main depends on them.
