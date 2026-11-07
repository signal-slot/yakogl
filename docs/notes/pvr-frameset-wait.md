<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P terrain: the PowerVR backend's frame-set reuse wait (a host study)

Written 2026-11-07 from the code at `e2a4e86b`. The files cited
(`src/pvr/backend.rs`, `driver/rust/src/gpu/pvr/render.rs`,
`zephyr/Kconfig`, `driver/zephyr/Kconfig`, `src/osal/zephyr.c`,
`driver/src/pvr_render.c`) are byte-identical at origin/main `80c486d2`,
so the line numbers hold for both. No code change in the tree and no board
run. It follows `pvr-hwrt-reuse.md`, which found the HWRT and kick-ring
holds at 0 in a steady terrain frame and named this wait as the one hold
terrain meets ("about 47 a frame").

**Answer.** Kick N is recorded into frame set `N mod 4` and, when its
frame opens, waits for kick N - 4, the kick that last used the set
(`backend.rs:5288-5291`). The host log confirms it for every kick. There
are 4 sets because the code builds an array of 4 (`backend.rs:271`) and
the Kconfig range stops at 4 (`zephyr/Kconfig:102`); the number is a memory
and HWRT-member budget, not a dependency of the scene. The set's memory is
a real hazard (the GPU reads the set's arenas until kick N - 4 completes),
but the depth 4 is a choice. At the board's per-kick period the wait is
not the cost: the frame is GPU-bound, a deeper pool changes nothing
in the model, and the poll adds CPU lateness without adding frame time.
The count is **49 waits plus the swap's one = 50 a frame** in the model,
not 47: the 47 of `pvr-hwrt-reuse.md` is "about" and is not reproduced.

## Which frame set each wait waits for

`frame_begin_with` (`backend.rs:5282-5292`) does, once per kick:

```
note_arena_hw(b);
b.cur = (b.cur + 1) % b.nsets;
if b.wait_fence(b.sets[b.cur].fence) != 0 { ... }
b.frame_set_activate(b.cur);
```

`sets[cur].fence` is written when the set's kick is submitted
(`backend.rs:9061`, `b.sets[b.cur].fence = fence`), so it is the last kick
that used the set (`FrameSet::fence`, `:1526-1547`). Fence ids are the
submit counter (`b.kicks` counts the same kicks), and every frame open ends
in one submit, so the set a kick takes is `kick mod 4` and the fence it
waits for is its own number minus 4.

The temporary counter below, run on the host (terrain, 20 frames,
`tools/hostharness/am62p-terrain/run.sh <GLMARK2_SRC> <out> kicks`), prints
this at every frame open from kick 101 to 260:

```
fsw: begin after kick 100 set 1 used_by 97 last 100 completed 100
fsw: begin after kick 101 set 2 used_by 98 last 101 completed 101
fsw: begin after kick 102 set 3 used_by 99 last 102 completed 102
fsw: begin after kick 103 set 0 used_by 100 last 103 completed 103
...
fsw: begin after kick 134 set 3 used_by 131 last 134 completed 134
fsw: begin after kick 135 set 0 used_by 132 last 135 completed 135
```

All 160 lines have `used_by` = (kick that opens) - 4, the set cycling
1, 2, 3, 0. The host OSAL completes every submit at once
(`src/osal/linux.c`, as `pvr-hwrt-reuse.md` says), so `completed` equals
`last` and no host wait fires (`Backend::waits` stays 0): the figures that
follow the host log are a replay of the rule, not a measurement of the
wait. The host run itself gives what the replay is fed with: 52 kicks a
frame in every steady frame (frames 1-19: "8.00 draws and 52.00 kicks
submitted a frame", `kicks.py`; frame 0 has 61), with the draws at kick
offsets fb1+2, fb2+1, fb4+1, fb5+10, fb6+9, fb4+9, fb7+10, fb0+10.

The per-frame pattern: the swap, `eglSwapBuffers` ->
`pvr_backend_finish_previous` -> `wait_fence(last_fence - 1)`
(`backend.rs:9174-9190`), drains the queue to the window's kick S. The next
frame's first three opens want S-3, S-2, S-1, all complete, so they pass
without a wait. The fourth wants S, which the GPU is still working on.
From then on every open waits. That is 52 - 3 = 49 frame-set waits, plus
the swap's one.

## The wait is the pool's depth, and the queue never drains

Scratch model, `sim.py` (outside the tree, 25 lines): kick N costs the
CPU `c` to record and the GPU `g` to complete after the kick before it;
each wait ends on the first poll step after its fence completes; the swap
waits for the last kick but one. 52 kicks a frame, 40 frames, the last 30
averaged.

| g (GPU ms a kick) | c (CPU ms a kick) | poll | frame ms | waits a frame |
|---:|---:|---|---:|---:|
| 1.6 | 0.1 | 1 ms | 83.20 | 50.0 |
| 1.6 | 0.1 | 20 us | 83.20 | 50.0 |
| 1.6 | 0.3 | 1 ms | 83.20 | 49.7 |
| 1.1 | 0.1 | 1 ms | 57.17 | 49.7 |
| 1.1 | 0.3 | 1 ms | 57.20 | 39.6 |

`g` = 1.6 is the board's 83.06 ms / 52 kicks (`tools/cts/RESULTS.md`, the
AM62P common row: terrain 83.06 ms, GL time 81.9 ms a frame);
1.1 is the queued price (`gl_test` case 216, 1.15 ms a kick, same file).
The frame is 52 x g whatever the poll: the CPU is ahead, the GPU never waits
for it. Only a pool of one set shows the poll in the frame:

| frame sets | poll 1 ms: frame ms | poll 20 us: frame ms | waits a frame |
|---:|---:|---:|---:|
| 1 | 109.20 | 88.45 | 52 |
| 2 | 83.20 | 83.20 | 52 |
| 3 | 83.20 | 83.20 | 51 |
| 4 (main) | 83.20 | 83.20 | 50 |
| 8 | 83.20 | 83.20 | 46 |

So the pool is deeper than the poll needs: with 3 kicks queued while one
is waited for (3 x 1.6 = 4.8 ms, or 3 x 1.1 = 3.3 ms) the GPU does not run
dry in a 1 ms poll quantum. This agrees with `pvr-hwrt-reuse.md`'s bound
(at most one poll quantum a four kicks, under 0.25 ms a kick, when the
queue drains).

**Is each wait a real dependency?** Two parts:

- *The hazard is real.* Set `s` holds the PDS and GENERAL arenas, the VDM
  stream, the depth-bias and scissor tables, the background/EOT PDS page
  and the clear colour (`backend.rs:1526-1547`); the next frame
  overwrites all of them (`frame_set_activate`, `:2298-2315`, then
  `b.pds.bump = 0 ... b.vdm_dw = 0`, `:5293-5299`). Kick N - 4 reads
  them until it completes, so no kick may skip the wait.
- *The depth is not.* The scene needs no more than the 4 sets because the
  GPU is the bottleneck. A mip level kick records one quad (its arena use is not measured; the
  backend report's high-water would give it), but takes the same set of about 1872 KiB as a
  window kick (the pool is sized for glmark2's largest frame, 325896 bytes
  of PDS and 516768 of GENERAL, `backend.rs:293-304`). A chain therefore
  spends a whole set per level. The pool is "too small" only in the sense
  that a set is per kick, whatever the kick weighs.

## Where the limit of 4 comes from

- `PVR_BACKEND_FRAME_SETS = 4` (`backend.rs:271`): the size of
  `Backend::sets` (`:1581`). `pvr_backend_set_frame_arenas` returns -EINVAL
  for more (`:9337-9341`); `CONFIG_YAKOGL_PVR_FRAME_SETS` is `range 1 4`,
  default 4 (`zephyr/Kconfig:99-103`); the AM62P glmark2 image sets
  nothing, so it runs 4 x (1024 KiB GENERAL + 768 KiB PDS)
  (`src/osal/zephyr.c:61-70`, `backend.rs:293-304`: "With 4 sets that is
  7488 KiB of the render pool, where 4 x (4 MiB + 1 MiB) took 20800 KiB").
- The comment at `backend.rs:1571-1581` says why the number was raised
  from 2: a Quick 3D frame is several passes (shadow maps, scene,
  effects) and with two sets a pass could not start until the one before
  last had finished.
- Past it, `pvr-hwrt-reuse.md` fixes the other three limits: the kicks in
  flight are at most `nsets` - 1 at a submit (the opens above), a size has
  `HWRT_MEMBERS` = 4 datasets (`render.rs:85`), so sets <= 4 never waits in
  `hwrt_get` (`render.rs:703-772`); `HWRT_SETS` = 32 datasets, so
  4 sizes x members <= 32 (`render.rs:74`); the geometry ring has
  `GEOM_UFO_SLOTS` = 16 (`render.rs:54`), so sets <= 16 (17 fires 27 ring
  waits a frame in that note's model).

## The expected cost of one wait on the board

A wait is `wait_fence` (`backend.rs:2327-2343`), which returns at once
when the fence is complete, else `z_wait` (`src/osal/zephyr.c:636-648`)
-> `pvr_render_wait` (`driver/src/pvr_render.c:778-783`, "frame fence") ->
`wait_for` (`render.rs:2333-2454`).

- *Poll, default build* (`WAIT_POLL_US` = 1000, `render.rs:2026-2029`; the
  Kconfig knob `YAKOGL_DRV_GPU_PVR_FINE_WAIT` is opt-in,
  `driver/zephyr/Kconfig:183-192`): a wait ends on the first whole
  millisecond after the fence moved (`pvr-fence-wait.md`, "The fine-wait
  run"). With the fence's time uniform within the millisecond, the mean
  lateness is 0.5 ms and the worst 1 ms. The CPU is idle in it: about
  50 x 0.5 = 25 ms of a 83 ms frame, none of it on the critical path
  while the queue holds 3 kicks.
- *Poll, 20 us* (`pvr-fine-wait`, the plan item `pvr-fine-wait-default`
  makes it the default, not yet merged at this commit): the mean lateness
  is 10 us, 0.5 ms over the 50 waits of a frame. The board saw terrain go 83.08 ->
  81.67 ms with it (`ce3354ac`, in `am62p-slow-terrain.md`), 1.4 ms a
  frame. The model gives 0 for a GPU-bound frame, so those 1.4 ms say that
  the frame is not entirely GPU-bound: at times the queue is shorter than
  3. Where the queue is short is after the swap, whose wait leaves only the
  window's kick in flight (above): the next frame's three free opens
  record while one kick runs, then the opens wait with the queue at 1,
  2, 3. A 1 ms lateness lands there on the frame's first waits, where the
  GPU has only one to three kicks (1.6 to 4.8 ms) of work queued. This is the one place the 1 ms
  poll can be seen in the frame, once a frame.
- *Cost per wait, as a record value:* none exists. `Backend::waits`
  (`backend.rs:2331`, `pvr_backend_counters`) is not printed by the AM62P
  glmark2 image, and `wait_for` has no counter by caller
  (`pvr-hwrt-reuse.md`, "What no record shows yet"). On the host the
  counter is 0 by construction.

## Fix designs, ranked

All are board-confirmed by terrain's ms a frame in the GM record (the common
set of `tools/cts/board-stage.sh am62p common`, or `tools/board.sh am62p
gm`); the first needs no other record. The prediction of the model is 0
change to the frame for 2 and 3, and 0-1.4 ms for 1.

1. **The 20 us poll for the frame-set wait (no frame-set change).** The
   item `pvr-fine-wait-default` makes it the default build. The frame-set
   wait is 50 of the frame's waits, so this is the whole lever on the wait
   itself. Limit: `WAIT_POLL_US` is global, the 20 us read costs a
   4-byte invalidate and read a step (`pvr-fence-wait.md` line 88); at
   most 25 reads a wait while it waits for a kick of 1.6 ms. Confirming
   record: terrain's ms a frame against the commit before it and
   `gl_test: mipchains: ... ms a frame` (case 216). A fall of 0-1.4 ms is
   the prediction; 4 ms or more would say the queue drains and a deeper
   queue (2) is worth its memory.
2. **A deeper pool: sets 4 -> 8 and `HWRT_MEMBERS` 4 -> 8, as one
   change.** `pvr-hwrt-reuse.md` candidate 2: the two must move together
   (8 sets with 4 members gives 31 HWRT waits a frame). Limits: sets <=
   members; 4 sizes x 8 members = 32 = `HWRT_SETS`, exactly the cap, so no
   fifth size is possible while terrain runs (a scene before terrain that
   leaves another size is recycled at the scene's start, `pvr-hwrt-reuse.md`
   (c)); sets <= 16. Cost: 4 more frame sets, about 7.3 MiB of the render
   pool at 1872 KiB each, and 16 more datasets, about 1.5 MiB. Code:
   `backend.rs:271`, `zephyr/Kconfig:102`, `render.rs:85`, #325's and
   #301's files. Prediction from the table above: 0 ms a frame (8 sets,
   83.20 against 83.20; 46 waits a frame against 50). It is a test of the
   hypothesis "the GPU never waits for a set". Confirming record:
   terrain's ms a frame at 8/8 against 4/4 from the same commit, within
   the run spread (0.15 ms on the other scenes, `RESULTS.md`), and the
   kick-times `frag->frag` unchanged; `"render: ... still busy"`
   (`render.rs:758`) stays 0.
3. **Small sets for the mip kicks (a second pool).** A level kick needs
   one quad, so a pool of small sets (the 16 KiB GENERAL and 8 KiB PDS
   one-draw minimums, `GEN_PER_DRAW_MAX`/`PDS_PER_DRAW_MAX`, which
   `pvr_backend_set_frame_arenas` enforces at `backend.rs:9340-9344`)
   gives a chain a deep queue for a few tens of KiB. Limits: the HWRT
   members still bound the kicks of one size in flight (a chain is
   9 kicks of one size, so d in-flight chain kicks need members >= d); a
   second ring of fences and the hazard order between the two pools (the
   window kick and a chain kick can touch the same target, the
   dependence is by fence, which already crosses sets, `backend.rs:9055-9063`)
   are new code. Its payoff is the same as 2: nothing, while the frame is
   GPU-bound. It is the cheaper form of 2 only if 2 is shown to matter.
   Confirming record: as 2, plus the backend report's arena high-water
   (`note_arena_hw`, `backend.rs:5286`, defined `:9352`) for a mip kick, a value to read
   before building it.
4. **Fewer kicks (path A/B and the single-pass chain).** Not a frame-set
   fix: each kick removed removes one open and one wait, which is
   `pvr-mip-fewer-kicks.md`'s and `am62p-pvr-mipmap-kicks.md`'s work.
   Listed because the wait count is the kick count: 52 kicks, 50 waits.

Do not start 2 or 3 before 1 is measured: the model says they gain
nothing, and a drop under 1.4 ms from 1 would confirm that the pool is
deep enough. They matter only if the board's queue is found to drain (the
fall in `case 216` of 4 ms or more).

## The temporary counter

Local only (the tree is unchanged). Apply at `src/pvr/backend.rs`, in
`frame_begin_with`, after `b.cur = (b.cur + 1) % b.nsets;`:

```diff
+    if b.kicks >= 100 && b.kicks < 260 {
+        glog!("fsw: begin after kick {} set {} used_by {} last {} completed {}", b.kicks, b.cur, b.sets[b.cur].fence, b.last_fence, b.completed_fence());
+    }
```

and run `tools/hostharness/am62p-terrain/run.sh <GLMARK2_SRC> <out>
kicks`; the lines are in `<out>/kicks/logs/powervr/0.log`. On the board the
same line (`completed` below `used_by`) counts the waits that fire: a
wait fires when `used_by > completed`.

## What is not shown

- The wait counts are a model of the code's rule, not a board count. The
  host completes kicks at once, so the 50 a frame is the replay of
  "wait for N - 4 and the swap's N - 1", not a counter that fired.
- `g` and `c` are the board's frame time per kick and an assumed 0.1-0.3 ms
  of CPU a kick, not measured per kick. A GPU time uneven across kicks
  (the 800x600 kicks cost more than the chain levels) is not in the model,
  and an uneven `g` is what could make the queue drain at the cheap chain
  kicks; the model's answer for it is the 4 ms drop above.
- The 47 of `pvr-hwrt-reuse.md` and the 50 here differ by the three free
  opens after the swap and the swap's wait; whether the board sees 47
  would need the counter above on the board.
