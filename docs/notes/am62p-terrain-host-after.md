<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P terrain: host harness at the tip, and the board values to expect

Plan #329, item `am62p-terrain-host-after` (#1199). Host only: nothing ran on
a board and no code changed. It does for the AM62P what
`pi4-terrain-host-after.md` did for the Pi 4: it reruns the host harness of
`am62p-slow-terrain.md` on a later main and predicts what the board session
`am62p-board-after` must read. Every figure is cited or marked
**(prediction)** with its arithmetic.

Ran at `d2767897` (the integration tip this item started from, 2026-11-21).
glmark2 `22c527cb` in `$CONNECTED_DEPS_DIR/glmark2-src`.

## Result

**The host frame is 51 kicks, not 52: the window's dead clear is gone
(`4255adf1`). Nothing else in terrain's frame moved on the host.** The
board's 83.06 ms (`6a7021ec`) is predicted to fall to about 80 to 82 ms: one
800x600 kick (-1.1 to -1.8 ms) and, if the image of that record lacked it,
the 20 us fence poll (-1.4 ms). Two of the commits the plan expected to
change the frame are **not on this tip** (below), so this note does not
measure them and the prediction does not include them.

## Commands

```
bash tools/hostharness/am62p-terrain/run.sh "$CONNECTED_DEPS_DIR/glmark2-src" build/am62p-terrain profile kicks
```

The perf run is not made (`perf_event_paranoid`, as in `am62p-slow-terrain.md`,
"perf (`GM2_PERF=1`): not run here"). `run.sh` wrote
`build/am62p-terrain/prof/` and `build/am62p-terrain/kicks/kicks.txt`
(not committed: `build/`).

## Kicks a frame (`kicks.py`)

`kicks.py` summary line: `frames 1..18: 8.00 draws and 51.00 kicks submitted
a frame` (frame 0, the setup's: 61 kicks, it also makes the 512x512 specular
map's chain, as before). Every steady frame is the same line:

```text
frame 6: 8 draws, 51 kicks submitted; per draw (its fb, kicks since the draw before): fb1+1 fb2+1 fb4+1 fb5+10 fb6+9 fb4+9 fb7+10 fb0+10
```

By kind (the reading of `am62p-slow-terrain.md`, "Kicks", with the frame's
first draw now `fb1+1` where it was `fb1+2`):

| kicks | what | `kicks.py` |
|---:|---|---|
| 8 | passes: height map, normal map, terrain, bloom h, bloom v, overlay, tilt-shift h, tilt-shift v (the window's, submitted by the swap) | `fb1+1 fb2+1 fb4+1 fb5+.. fb6+.. fb4+.. fb7+.. fb0+..` |
| 43 | mip levels: chains of 9 (terrain), 8 (bloom h), 8 (bloom v), 9 (terrain again after the overlay), 9 (tilt-shift h) | `fb5+10`, `fb6+9`, `fb4+9`, `fb7+10`, `fb0+10` less the pass in each |
| 0 | the window's clear-only kick | was `fb1+2` |
| **51** | | `tests/pvr_terrain_kicks.rs` `terrain_frame_is_51_kicks_43_mip_levels_8_passes_and_no_window_clear` pins the same |

## CPU GL-call time a frame (`GM2_PROFILE=1`)

19 counted frames (2 to 20), from `build/am62p-terrain/prof/profile-powervr.md`:

> 0.331 ms wall, 0.331 ms cpu a frame; in GL and EGL calls 0.221 ms (67%), in
> glmark2 itself 0.111 ms.

| entry point | calls a frame | wall ms a frame |
|---|---:|---:|
| `glGenerateMipmap` | 5.0 | 0.117 |
| `glDrawArrays` | 8.0 | 0.070 |
| `eglSwapBuffers` | 1.0 | 0.003 |
| every other entry point | | 0.031 |

Draw steps: 152 timed draws, 8.0 a frame, 1.925 us a draw. The raw line:
`gm2: gl-frames swaps=20 counted=19 wall_ns=6298062 cpu_ns=6298047 gl_wall_ns=4189911`.

**The absolute times are not comparable to `am62p-slow-terrain.md`'s**
(0.729 ms, 0.440 ms in GL): the host that ran this was faster, and it is not
the same machine. The mix is: `glGenerateMipmap` 52.9% against 50.1% and
`glDrawArrays` 31.5% against 35.0%. The tip has no logging in a counted frame
(the log has the init line and the 512x512 chain line, both in setup).

## Host-side waits by call site

None counted: on the host every fence is complete at once, so
`Backend::waits` stays 0 (as in `am62p-slow-terrain.md`, "CPU waits on the
GPU, by call site"). The sites are unchanged by anything on the tip, so the
table there holds: the frame-set reuse wait, one a kick, 51 now
(`src/pvr/backend.rs`, `PVR_BACKEND_FRAME_SETS` = 4: kick N waits for N - 4);
the swap's `finish_previous`, 1; none in `glGenerateMipmap` with
`mip_cpu_tail` off, none in the render service, none in `settle_texture`.
The host cannot show their cost; that is the board's.

## What changed since `am62p-slow-terrain.md` (the 52-kick run)

| change | commit (on this tip?) | effect on the host frame |
|---|---|---|
| the window's dead clear kick | `4255adf1` 2026-11-18, `gl: a full pending clear stays on its framebuffer across a bind to another` (**yes**) | 52 -> 51 kicks; `fb1+2` -> `fb1+1`. The clear is parked on the window and fused into the window's own draw |
| the 20 us PowerVR fence poll by default | `1e67474f` 2026-11-09, `zephyr: the 20 us PowerVR fence poll is on by default on the AM62P` (**yes**: `CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT` `default y if SOC_AM62P54_A53`, `driver/zephyr/Kconfig:183`; `tests/pvr_fine_wait_default.rs`) | none on the host (no wait); a board effect only |
| each mip level's kick walks only that level's tiles (`pvr-mip-level-area`) | **not on this tip**: `src/pvr/backend.rs` has no per-level area; `record_mip_level` still declares level 1's size and clips the store (`mip_clip`, `:1698-1712`, `:7125-7145`) | none yet: 13,849 tiles a frame of mip kicks |
| a CPU mipmap filter of a render target waits for that target's kick (`gl-settle-wait-kick`) | **not found as a terrain change**: `settle_texture` already calls `backend::wait_kick(kick)` (`src/gl/texture.rs:690-719`, from `e320630d` 2026-11-09, `vivante: the depth resolve waits for the kick that wrote the depth`) | none: terrain neither uploads nor reads back a texture, and `mip_cpu_tail` is off |

The plan named four commits (`pvr-dead-clear-kick`, `pvr-fine-wait-default`,
`pvr-mip-level-area`, `gl-settle-wait-kick`). On `d2767897` the first two are
there and the last two are not as such; when `am62p-board-after` builds its
image from a later main, check `git log -S` for them and fold in the rows
below.

## Board values to expect (`am62p-board-after`)

Inputs, all cited: terrain 83.06 ms at `6a7021ec` (`tools/cts/RESULTS.md`,
AM62P row), 81.9 ms of it in GL calls (`am62p-slow-terrain.md`); about
1.1 ms a queued kick (`ce3354ac`; case 216: 40 kicks in 46.15 ms); the 800x600
window kick, 1.1 to 1.8 ms (`am62p-slow-terrain.md`, candidate 5); the 20 us
poll, 83.08 -> 81.67 ms (`ce3354ac`, Kconfig help text). Whether the
`6a7021ec` image already had the poll is not in the record (the Kconfig
default came 8 days later), so the range allows both.

| quantity | value | basis |
|---|---|---|
| kicks a frame | **51** host; the board counts about 51.7 (prediction) | host `kicks.py` above; the board's figure also counts setup kicks (52.7 at `ce3354ac` against the host's 52 then): +0.7 |
| terrain ms a frame, the 83.06 ms baseline | (record) | `6a7021ec` |
| after the dead clear only | 81.3 to 82.0 ms **(prediction)** | 83.06 - (1.1 to 1.8) |
| after the dead clear and the 20 us poll, if the baseline lacked the poll | 79.9 to 80.6 ms **(prediction)** | 83.06 - (1.1 to 1.8) - 1.4 |
| **range to read** | **79.9 to 82.0 ms, about 12.2 to 12.5 FPS** **(prediction)** | the two rows above; 1000 / 82.0 = 12.2, 1000 / 79.9 = 12.5. At 51 kicks x 1.1 ms = 56.1 ms of kicks plus about 20 ms of the fence stall plus 3 to 4 ms of CPU the sum is 79 to 80 ms, inside it. The record can only show -1.1 to -3.2 ms: under run-to-run spread, so the verdict is the kick count, not the ms |
| not in this range | | `pvr-mip-level-area` (0 to -3 ms, not on the tip), three levels a render (-25 ms, off), path B (+40 ms, off) |
| re-post lines | **8** printed over a terrain run, if the stall is still once a frame **(prediction)**; 0 if it is gone | `render.rs:2371` prints the first 8 re-posts and every 256th; terrain's run is about 110 to 125 frames (5802 kicks over 110 frames at `ce3354ac`), so about 1 a frame gives M of about 110 to 125: 8 lines, no 256th. The tip changes no wait that meets the 20 ms fence (`am62p-slow-terrain.md`, "Which wait meets the 20 ms fence is not in any record"), so the stall is expected to remain (prediction). The `pvr-repost-trace` text now names the call site |
| `glmark2-frame:` split, gl us a frame | about 78.7 to 80.8 ms **(prediction)** | the frame less the swap and glmark2's own: at `6a7021ec` 81.9 of 83.06 was GL, so GL = frame - 1.2 ms (host: swap 0.003 ms, glmark2 own 0.111 ms of 0.331) |
| `glmark2-frame:` split, swap us a frame | under 1 ms **(prediction)**, unless the 20 ms fence is in the swap | `finish_previous` waits for fence N - 1, which the frame-set waits have drained to 3 in flight (`am62p-slow-terrain.md`, call-site table). A swap of about 20 ms would put the stall on the swap's wait and not on a kick wait |
| `glmark2-counters` keys | **not available**: `glmark2-counters-print` is not on this tip (`driver/samples/zephyr/glmark2/src/main.cpp` has no `glmark2-counters` line; design in `docs/notes/glmark2-scene-counters.md`) | if merged by the image's commit, per that note's PowerVR table: `kicks` 51 to 52, `waits` about 51 (the frame-set waits that find the fence not complete; fewer if some are met complete) and the swap's, `reposts` about 1 a frame or 0, `wait_timeouts` 0 **(prediction)** |

## What would falsify it

* Terrain at 83 ms or more with 51 kicks: the kick price, not the count, is
  the frame, and the dead window kick cost far less than 1.1 ms (it is a
  clear that fuses into the window's draw; a kick that stores 1.9 MB
  of depth and stencil, `am62p-slow-terrain.md`).
* Re-post lines 0 and terrain still near 80 ms: the 20 ms stall was never
  once a frame at `6a7021ec`, and the frame is 51 kicks x about 1.5 ms.
* A swap of 20 ms in the `glmark2-frame:` line: the fence the re-post ends
  is met by `finish_previous`, which narrows candidate 1 of
  `am62p-slow-terrain.md` to the swap's wait.
