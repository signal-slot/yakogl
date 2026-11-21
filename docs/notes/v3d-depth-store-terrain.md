<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: can the terrain target's depth store and load be skipped exactly?

Study (item v3d-depth-store-study), candidate 4 of `v3d-terrain-gpu.md`
("Fix candidates after the fix"). Measured on the host at main `975a6792`
(the figures below were counted with a scratch print, reverted; no code
changed). The source files named were read at `975a6792`; between it and
origin/main `2be14a44` nothing in `src/v3d`, `src/gl/draw.rs` or
`src/gl/framebuffer.rs` moved that this note reads.

## Verdict

**No exact rule exists that the backend can apply when job 6 is built.** One
store is dead (job 6's), but the fact that makes it dead, "the next use of
the buffer is a full clear", is only known a frame later, after the job is
submitted. Job 3's store and job 6's load are *needed*: terrain's overlay is
depth-tested against them. What can be exact is a lookahead the V3D backend
does not have, and the gain it would buy is at most 0.43 ms, likely under
0.2. Recommendation: do not build it; close candidate 4 as "no exact rule,
bounded gain".

## Host counts (20 frames, `GM2_ENTRIES='^terrain$'`, `YAKOGL_V3D_TFU=1`)

Command: `tools/hostharness/glmark2/run.sh <GLMARK2_SRC> <out> v3d` with the
env of `tools/hostharness/pi4-terrain/run.sh` (`GM2_PROFILE=1`, `GM2_FRAMES=20`),
plus a scratch `fprintf` at the top of `emit_tile_list`
(`driver/src/v3d/v3d_job.c:1134`) printing the target size, `zs_address` and
`zs_address && zs_load` / `zs_store` (the two `if (c->zs_address && ...)`
packets at `:1224` and `:1269`). Counted lines, per entry's 20 frames (the
three further window stores are scene setup, before frame 1):

| job (see `v3d-terrain-gpu.md`) | target | depth buffer | loads | stores | lines in 20 frames |
|---|---|---|---:|---:|---:|
| 1, 2, 4, 5 | 256x256 maps, bloom | none (`zs_addr=0`) | 0 | 0 | 80 |
| 3 | terrain target 800x600 | `3642000` | 0 | **1** | 20 |
| 6 | terrain target again (overlay) | `3642000` | **1** | **1** | 20 |
| 7 | tilt H 800x600 | none | 0 | 0 | 20 |
| 8 | window 800x600 | `14ac000` | 0 | 0 | 20 |

A frame: **2 depth stores, 1 depth load**, 8 jobs (job 0 is gone, the dead
window clear fix). Bytes: the backend keeps depth as D24S8, 4 bytes a pixel
(`zs_layout`, `src/v3d/backend.rs`, `cpp` 4; glmark2's `DEPTH_COMPONENT16`
becomes D24S8), 800x600 x 4 = 1,920,000 B per packet: stores 3.84 MB, load
1.92 MB, **5.76 MB a frame**, the number `v3d-terrain-gpu.md` gives.

## Why the stores and the load are there

glmark2 (checkout `22c527cb`, `src/scene-terrain/`):

- Only the terrain target has a depth renderbuffer
  (`scene-terrain.cpp:82`, `setup_offscreen(screen_res, true)`;
  `base-renderer.cpp:149` `GL_DEPTH_COMPONENT16`). Nothing samples it (it is
  a renderbuffer; no `OES_depth_texture`, no `glReadPixels` of depth, no
  blit) and the CPU never reads it.
- Job 3 (`TerrainRenderer::render`, `terrain-renderer.cpp:43-57`): clears
  colour and depth, draws the terrain with depth test on
  (`BaseRenderer::make_current`, `base-renderer.cpp:64-81`, test on for any
  target with a depth renderbuffer, mask on). Depth ends up in the buffer;
  job 3 must store it **only if a later job reads it**.
- Jobs 4, 5 draw to other targets (bloom H, bloom V). They bind a different
  framebuffer: no clear or draw touches the terrain target's depth.
- Job 6 (`OverlayRenderer::render`, `overlay-renderer.cpp:72-90`):
  `target_renderer_.make_current()` re-binds the terrain target, **no
  `glClear`** (`overlay-renderer.cpp` has none; `grep clear` of the three
  renderers finds only `texture-renderer.cpp:57` and `terrain-renderer.cpp:47`),
  depth test still on, quad at z = 0 (`terrain-texture.vert`:
  `gl_Position = vec4(position, 1.0)` with the grid's z 0; window depth 0.5,
  `GL_LESS`). So the overlay fragments pass only where the terrain's stored
  depth is greater than 0.5. **Job 6's depth test reads job 3's depth: the
  load, and job 3's store, are required.** Whether the test ever fails
  depends on the scene's data (terrain nearer than mid-range); it can, and
  the backend cannot know it, so it cannot drop the load either.
- Job 6's store writes depth the overlay may have changed (the test passes
  and `glDepthMask(GL_TRUE)` writes z 0.5). After job 6, the next use of the
  buffer is job 3 of the next frame: `glClear(COLOR|DEPTH)`. So job 6's
  1.92 MB store is dead, and only it.
- Jobs 7, 8: other targets (the window's depth is its own, cleared, not
  stored: `v3d_surface_swapped`, `backend.rs:5494`).

For terrain: **job 6's load needs job 3's depth** (no clear between the two
passes; the only clears of the target are job 3's own), and the dead store is
job 6's.

## The rules considered, and what breaks each

The store decision is made in `flush_job` (`src/v3d/backend.rs:2951-2995`):
`zs_store = (z_written || s_written || ...) && !zs_discard`, `zs_load =
z_stored && !z_cleared`. `zs_discard` is set by one thing today,
`v3d_surface_swapped` (`:5494`: a window's swap, EGL 1.5 3.10.1). The
decision is final at job end: the job's tile list is built there
(`v3d_job_end`, `:2990`) and the job goes to the GPU.

1. **Skip the store if the next use is a full clear or the deletion
   (lookahead).** True for job 6 (next use: frame N+1's job 3 clear), but job
   6 ends when job 7's draw binds another framebuffer, ~1 frame before the
   next use. Nothing is known then. To know it, job 6 would have to be held
   unsubmitted until the target's next bind/clear: job 7 samples job 6's
   colour, so holding it stalls the frame. **Not possible, not exact.**
2. **Defer the decision to the next bind, store later.** The store is a
   tile-list packet at the end of each tile; the tile buffer is gone after
   the job. There is no separate "store later" step in the hardware. The
   one exact variant is to *re-run* the job on a later use that turns out to
   need the depth: an unbounded replay of the draws, which the backend does
   not keep. **No.**
3. **`EXT_discard_framebuffer` / `glInvalidateFramebuffer`.** The GL core
   validates and ignores it (`invalidate_framebuffer`,
   `src/gl/framebuffer.rs:3793-3830`: no state change, "Nothing to discard");
   the backend has no entry for it (`src/backend.h` has no discard call).
   glmark2 never calls it. A hook would be exact for an application that
   calls it (the spec makes the contents undefined) but it changes nothing
   for terrain, and it is ES 3.0 / the extension, not an ES 2.0 answer. Out
   of scope for this candidate; a separate feature if wanted.
4. **A later job of the same frame loads it.** That is job 6, and it is why
   job 3's store stays. A counter-rule "store only if a later job of the
   frame loads it" is the same lookahead as 1 for job 3 (jobs 4, 5 are
   between) and says "store" for terrain anyway.
5. **Merge jobs 3 and 6** (depth stays on chip): job 6 needs bloom V's
   output, which needs bloom H, which needs job 3's colour. Impossible.

**The GL case that breaks any guess for job 6's store**: the same frame as
terrain up to job 6, then another draw to the terrain target with depth test
and no `glClear` (a second overlay, a particle pass), or `glClear(COLOR)` only
(depth kept, ES 2.0 4.2.3: only the named buffers are cleared). A backend that
skipped job 6's store would let that draw test against stale depth from job
3 (or the previous frame): different pixels, no error. This is the
"observable by a later draw without a clear" of `v3d-terrain-gpu.md`.
`tests/es2_report_guard.rs` would not see it; a host test of the guess
would, and would fail.

## What an exact rule would take, if the gain justified it

A *retroactive* rule only: let job 6's store go to memory as today, and make
the next frame's job 3 start without loading (it already does: its depth is
cleared, `zs_load = z_stored && !z_cleared` is false). So the *load* is
already skipped where it is dead; **the store is the only dead part, and it
cannot be known at job 6**. The structure would be a one-frame-late rewrite
of the job (resubmitting job 6's RCL without the store), which wastes the
saved time on the first run and cannot be exact for a second consumer.

Files, if someone builds a lookahead anyway (a "this target's depth is dead
after this job" bit supplied by the GL core, which would need to see the next
frame):

| file | what | held by |
|---|---|---|
| `src/gl/framebuffer.rs` / `src/gl/draw.rs` (job end and settle paths, bind) | the future-use knowledge; the call into the backend | #325 and #310 (draw.rs); #325 (framebuffer.rs) |
| `src/backend.rs`, `src/backend.h` | a new hook | #325, #310 |
| `src/v3d/backend.rs` (`flush_job` `zs_store`, `:2993`) | honour it | #308 |
| `driver/src/v3d/v3d_job.c` (`emit_tile_list` `:1269`) | none: already driven by `zs_store` | #308 / #321 |

## Expected gain and the board value that confirms it

Bytes: 1.92 MB of 18.3 MB of TLB traffic a frame (`v3d-terrain-gpu.md`,
"Per frame"), 10 percent. At the TFU's rate (4.4 GB/s equivalent, the
note's own conversion) 0.43 ms; the 130-tile job 6 is sample-bound
(1 fetch of bloom V, blend) so the store hides under the shading: **expected
0.1 to 0.4 ms a frame of a 35.7 ms frame (the `pi4-slow-terrain.md` figure),
under 1 percent.** Far below the noise of one board run (the
`glmark2-frame:` numbers move more than that between runs).

Board record to confirm it, from the `glmark2-jobs:` line of the terrain
scene (`print_jobs`, `driver/samples/zephyr/glmark2/src/main.cpp:1255`),
job 6's `render` ms (the entry with the terrain target loading and storing
colour): **a skipped store would lower it by at most 0.43 ms and leave
job 3's unchanged**. If instead job 6's `render` minus job 3's `render`
already sits near 0 after accounting for tile count (both 130 tiles), the
TLB store is hidden and the whole candidate is worth nothing. A cheaper
pre-check, no code: compare job 6 `render` with job 7's (130 tiles, one
colour store, no depth, 9 taps); the difference bounds what the depth
load plus store (3.84 MB through the TLB) can cost.

## Not done

- No board run (non-goal). The 0.43 ms is a bytes ratio, not a measurement.
- Whether the overlay's depth test ever fails in the real scene was not
  counted: the stand-in's rasteriser does not report fragment kills. The
  verdict does not depend on it (the backend cannot know).
- The quad's z of 0 is read from the vertex shader and `make_grid`'s
  default; the grid call is `make_grid(1, 1, 2.0, 2.0, 0)` (last argument
  the z), `overlay-renderer.cpp:99`.
