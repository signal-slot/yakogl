<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P terrain: the PowerVR frame-set depth at 4, 6 and 8 (a host study)

Written 2026-11-23 from the code at `cc702671`. The files cited
(`src/pvr/backend.rs`, `driver/rust/src/gpu/pvr/render.rs`,
`zephyr/Kconfig`, `driver/src/pvr_render.c`, `driver/samples/zephyr/glmark2/src/main.cpp`)
are byte-identical at origin/main `91b5184e` (`git diff` of `src/pvr`,
`driver/rust/src/gpu/pvr`, `src/osal`, `zephyr`, `driver/zephyr` is empty),
so the line numbers hold for both. No code is committed, no board run. It
follows `pvr-frameset-wait.md` (the depth of 4 is a choice) and
`pvr-hwrt-reuse.md` (the HWRT members must move with it).

**Answer.** On the host the depth changes nothing a terrain frame can show
except the count of frame-set waits: **49, 47 and 45 a frame** at depths 4, 6
and 8 (the swap's one included), with the kicks the same, **51 a frame**, in
all three. The replay of the board's period puts the frame at the same
81.6 ms (51 x 1.6 ms) at every depth: the GPU is the bottleneck and a deeper
queue is not read by it. A depth of 8 costs **9024 KiB** of the 57.8 MiB
render pool (7488 KiB of frame sets and 1536 KiB of HWRT members) and
touches three limits that move together. Not worth a `Backend` option: it is
a build constant (a Kconfig number, `YAKOGL_PVR_FRAME_SETS`), memory is
sized when the backend starts, and the host says the gain is 0 ms. The one
board run that would change this answer is in "Record values".

## The three runs

Scratch copies of the tree at `cc702671` (`git worktree add --detach`
under `build/`, ignored by git, removed after the runs), patched only there:

- depth 4: unchanged, plus the counters below;
- depth 6: `PVR_BACKEND_FRAME_SETS` = 6 (`src/pvr/backend.rs:271`) and
  `HWRT_MEMBERS` = 6 (`driver/rust/src/gpu/pvr/render.rs:96`);
- depth 8: both = 8.

The counters (the two lines added; `b.kicks` is the submit counter):

```diff
 // src/pvr/backend.rs, frame_begin_with, after b.cur = (b.cur + 1) % b.nsets;
+    if b.kicks >= 100 && b.kicks < 260 { glog!("fsw: begin after kick {} set {} used_by {} last {}", b.kicks, b.cur, b.sets[b.cur].fence, b.last_fence); }
 // before the submit call (backend.rs:10140), only in the depth-4 copy
+    glog!("ksz: {} {}x{}", b.kicks + 1, b.kick.width, b.kick.height);
```

Commands, one after the other (run in parallel they share a build and
mix their results):

```
bash <copy>/tools/hostharness/am62p-terrain/run.sh <GLMARK2_SRC> <out> kicks
```

with `GLMARK2_SRC` = `$CONNECTED_DEPS_DIR/glmark2-src` (glmark2
`22c527cb`, as `tools/hostharness/glmark2/README.md`). Each run is terrain,
20 frames, `ok, 20, 161, 0, 0, 0` (161 draws, no refusal, no GL error), and
`kicks.py` prints the same line for all three: "frames 1..18: 8.00 draws and
51.00 kicks submitted a frame" (frame 0 has 61). That is one kick fewer than
`pvr-frameset-wait.md`'s 52: the dead window clear
(`pvr-dead-clear-kick`) went in since.

**What the host measures.** The log line shows the rule each depth runs:
at the open of kick N the set holds kick N - depth, for every kick from 101
to 260.

| copy | log line (the open is the line's kick + 1) | `used_by` | distance |
|---|---|---:|---:|
| depth 4 | `fsw: begin after kick 100 set 1 used_by 97` | 97 | 101 - 97 = 4 |
| depth 6 | `fsw: begin after kick 130 set 5 used_by 125` | 125 | 131 - 125 = 6 |
| depth 8 | `fsw: begin after kick 130 set 3 used_by 123` | 123 | 131 - 123 = 8 |

The sets cycle over 0..depth-1 in each log.
The host OSAL completes a submit at once (`src/osal/linux.c`), so no
host wait fires and `Backend::waits` (`backend.rs:2672-2676`, which counts
only a wait on a fence not yet complete) stays 0: a wait count a frame is a
replay of the rule, not a counter that fired.

## The replay

`build/fsim.py` (scratch, 45 lines, not committed; the model of
`pvr-frameset-wait.md` with two more holds). Kick sizes are the host log's
(`ksz:`: one 51-kick period of 128x128, 256x256, 400x300 and 800x600
targets, kicks 113-163, repeated for 40 frames, the last 30 averaged). A kick
is recorded in `c` ms of CPU, waits for kick N - depth (the frame set,
`backend.rs:6115`), for the render of the same size `members` before it
(`hwrt_get`, `render.rs:714-772`) and for kick N - 16 (the geometry ring,
`render.rs:1404-1409`), then queues behind the GPU for `g` ms. The swap waits
for the last kick but one (`pvr_backend_finish_previous`,
`backend.rs:10286-10300`). A wait ends on the first poll step after its
fence completed. Run as `python3 -I fsim.py <0.log> <depth> <members> <g>
<c> <poll>`.

| depth | members | waits a frame: frame set (+ swap 1) | HWRT waits | ring waits | frame ms (g 1.6, c 0.1) | kicks a frame |
|---:|---:|---:|---:|---:|---:|---:|
| 4 (main) | 4 | 48 (+1 = 49) | 0 | 0 | 81.60 | 51 |
| 6 | 6 | 46 (+1 = 47) | 0 | 0 | 81.60 | 51 |
| 8 | 8 | 44 (+1 = 45) | 0 | 0 | 81.60 | 51 |
| 8 | 4 (not moved) | 7 (+1) | 31 | 0 | 81.60 | 51 |

The same at a poll of 20 us (`pvr-fine-wait`, now the default build:
`render.rs:2092-2098`, `pvr-fine-wait-default`): identical counts and frame
at all three depths. At g 1.1 ms (the queued price, `gl_test` case 216) and c 0.3 ms
a kick, 1 ms poll: depth 4 has 45.2 set waits and 3.6 HWRT waits a frame,
depth 6 has 42.5 and 2.0, depth 8 has 39.8 and 0.4; the frame is 56.10 ms
for all of them. A rotation of the period (so that the swap falls after
another 800x600 kick) gives the same 81.60 and the same counts at depths 4
and 8 (rotation 25 tried). The waits fall by 2 for every 2 sets: each
extra set takes one open out of the waited range of every frame-spanning run.
The frame, the replay says, is GPU time (51 x 1.6), whatever the depth.

What the replay is not: `g` is the board's mean (83.06 ms of
`tools/cts/RESULTS.md`, the AM62P common row, over 52 kicks at the last
record, not 51); an uneven `g` (800x600 kicks dearer than chain levels) is
the one input that could make the queue drain, and the replay cannot say it.
That is the board run below.

## What a deeper pool costs, and what stops it

Per extra frame set, at the defaults (`zephyr/Kconfig:137-148`; `ARENA_GEN_BYTES`
and `ARENA_PDS_BYTES`, `backend.rs:339-340`; the set's VDM stream and
tables in `FrameSet`, `backend.rs:1572`): 1024 KiB GENERAL + 768 KiB PDS +
the 64 KiB VDM stream and small tables, about **1872 KiB**
(`pvr-hwrt-reuse.md:184-191`; 4 sets are 7488 KiB, `zephyr/Kconfig:147`).
Per extra HWRT member (`pvr-hwrt-reuse.md:184-190`): about **96 KiB**
(TPC 16, VHeap 8, region headers 64, macrotile 4, MList 4 KiB, 384 bytes of
firmware heap), per size; terrain runs four sizes (128x128, 256x256, 400x300,
800x600 in the host log), so one extra member is 384 KiB.

| depth | extra frame sets | extra HWRT members x 4 sizes | extra bytes | share of the 57.8 MiB pool (`pvr-render-pool.md:16-18`) |
|---:|---:|---:|---:|---:|
| 4 | 0 | 0 | 0 | 0 |
| 6 | 2 x 1872 = 3744 KiB | 2 x 384 = 768 KiB | 4512 KiB (4.4 MiB) | 7.6 % |
| 8 | 4 x 1872 = 7488 KiB | 4 x 384 = 1536 KiB | 9024 KiB (8.8 MiB) | 15.2 % |

The AM62P render pool is shared with the other scenes' textures
and framebuffers (`pvr-render-pool.md`: terrain, then shadow and refract
failed with "render pool exhausted" when the sets were 20800 KiB), so the
price is paid in room for the largest scene, not in terrain alone. The set
arenas are 2.0 and 2.4 times glmark2's largest frame
(`backend.rs:330-340`), so smaller sets are the cheaper way to a deeper pool
(`pvr-frameset-wait.md`, candidate 3), not a bigger one.

What each limit is, at origin/main:

| limit | value | where | reached at |
|---|---:|---|---|
| sets built | array of 4 | `PVR_BACKEND_FRAME_SETS`, `backend.rs:271`; `sets: [FrameSet; ..]`, `:1627` | 5 |
| `pvr_backend_set_frame_arenas` | -EINVAL above the constant | `backend.rs:10452` | 5 |
| Kconfig range | `range 1 4` | `zephyr/Kconfig:140` | 5 |
| HWRT members of a size | `HWRT_MEMBERS` = 4 | `render.rs:96`; `hwrt_get` waits "HWRT member in flight" when `2 * (pairs + 1) > HWRT_MEMBERS`, `:756-765` | **any depth above members** (the last row of the table: 31 waits a frame at 8 sets with 4 members) |
| HWRT datasets | `HWRT_SETS` = 32 | `render.rs:85`; `hwrt_get` scans `:728` | 4 sizes x 8 members = 32: the cap, no fifth size |
| geometry UFO ring | `GEOM_UFO_SLOTS` = 16 | `render.rs:55`, throttle `:1402-1409` | 17 |
| frame-set fence | one `i64` a set | `FrameSet::fence`, `backend.rs:1572-1600` | none |

The firmware's own limit on a deeper queue (the KCCB/CCCB depth,
`CCCB_SIZE`) was not measured here: the host has no firmware, and the
`pvr-hwrt-reuse.md` model puts the first hold, the ring's, at 17 kicks.
`HWRT_MEMBERS` counts members as pairs (`2 * (pairs + 1)`), so it stays
even: 6 and 8 are valid values, 5 and 7 are not. The depth-6 copy and the
depth-8 copy built and ran terrain to the same 161 draws, no GL error.

## Record values a board run would read

One image per depth from one main commit with `CONFIG_YAKOGL_PVR_FRAME_SETS`
and `HWRT_MEMBERS` changed (the Kconfig range and the member count move
together in a scratch build), `tools/cts/board-stage.sh am62p common`, or
`tools/board.sh am62p gm`, terrain in the GM record:

- **`glmark2-counters` line** (`main.cpp:1336-1340`, fields `kicks`, `waits`,
  `reposts`, `wait_timeouts`, printed from `pvr_backend_counters` and
  `pvr_render_reposts`, `main.cpp:304-316`): `kicks` stays 51 a frame
  (the counts divided by the scene's `frames`). `waits` counts the waits that
  fired (`backend.rs:2672-2676`): at most 49, 47, 45 a frame at depths 4, 6,
  8 (the host's rule), and less when a set is already complete by the open.
  A `waits` that falls by 2 and 4 a frame while the frame time does not is
  the host's prediction.
- **FrameTime** (terrain's ms a frame in the GM record): 83.06 ms at the last
  record, and the same within the run spread (0.15 ms on the other scenes,
  `RESULTS.md`) at 6 and 8. A fall of 4 ms or more at 6 or 8 says the queue
  drains at the cheap chain kicks, and a depth is worth a Kconfig default
  of 6.
- **The 20 ms re-post line**
  (`render: fence N (who) not moved at 20 ms, re-posted K kick(s) (M so
  far) site=S last_posted=L`, `render.rs:2450-2461`, rate limited to the
  first 8 and every 256th) and **`pvr_render_reposts`**
  (`driver/src/pvr_render.c:369`, the `reposts` field of the counters): no
  line, `reposts` 0, at every depth. A frame-set wait is for the kick d
  behind, at most 8 x 1.6 = 12.8 ms of GPU work, under the 20 ms; a line
  with `site=` the frame-set wait at depth 8 is the firmware stall of
  `pvr-fence-wait.md`, not the depth.
- **HWRT hold:** `"render: WxH set N still busy at seq ..."` (`render.rs:769`, "HWRT member in
  flight") and `hwrt_busy_reuse` (`render.rs:355`) stay 0 when the members
  move with the sets, and show 31 a frame (the replay's number) when only
  the sets move.
- **Pool:** "render pool exhausted" must not appear (a failed run, not a
  number); "pool owner backend frame arenas high-water" and the pool summary
  (`pvr-render-pool.md:104-119`) show the 9024 KiB.

## Is it worth a `Backend` option?

No.

1. The host says 0 ms of frame (51 x 1.6 at every depth) and 2 and 4 waits
   less. A wait count is not time on a GPU-bound frame (`pvr-frameset-wait.md`).
2. The depth is the size of an array and of the render pool
   (`PVR_BACKEND_FRAME_SETS`, `Backend::sets`, `ArenaCfg::sets`); a `Backend`
   option (`Backend::mip_cpu_tail`-style, runtime) would still have to build
   the sets at the largest depth, so it saves no memory, and it is `nsets`
   that already is the runtime choice (`pvr_backend_set_frame_arenas`,
   `backend.rs:10440`, from the Kconfig). The knob exists for 1 to 4.
3. Going above 4 is a build-time change of three constants and one
   Kconfig range, and it must move `HWRT_MEMBERS` with it: a runtime option
   that left them apart would wait 31 times a frame (the last table row).
4. If the board's `waits` run says the queue drains (a 4 ms or more drop
   at 6 or 8), the order is: small sets for the chain kicks
   (`pvr-frameset-wait.md`, candidate 3, a few tens of KiB a set), then
   6 sets with 6 members (4512 KiB), and the Kconfig range `1 6` as the
   change. Do not raise it before the board shows the drop.

## What is not shown

- The waits are a replay of the code's rule on the host's kick list; the host
  fires none (`Backend::waits` 0). A board run reads the real count.
- `HWRT_MEMBERS` was moved in the scratch copies, but the host OSAL does not
  run the HWRT code (`src/osal/linux.c` has no dataset), so the HWRT waits
  (0, and 31 at 8/4) come from the replay of `hwrt_get`'s rule and the
  host's sizes; `render.rs:4196-4220` is the driver test of the rule.
- The firmware's queue limit is not measured, and the memory of the extra
  members is the figure of `pvr-hwrt-reuse.md`, not remeasured.
