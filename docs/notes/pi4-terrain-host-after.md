<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 terrain: host counts around the render-into mark fix

Plan #329, item `pi4-terrain-host-after` (#1209). Host only, nothing ran on
a board. The fix is `c6b16f71` (`gl: clear a texture slot's render-into mark
when the slot is freed`, `src/gl/framebuffer.rs`); the commit before it is
`c1b7e7d9`. The tip was not fetched (`git fetch origin` is not possible
here); `c6b16f71` is the tip of the integration snapshot the work started
from.

## Result

**The host confirms candidate 1 of `pi4-slow-terrain.md` ("Fix candidates,
ranked") where its premise holds, a render-image table of 8, and shows that
on the tip the table is 16, so the fix changes no steady-state count there.**

* With `RT_IMAGES = 8` (the table the prediction was written for, and the
  Pi 4 image of `c4af4917`): terrain after the two desktop scenes, before the
  fix, makes 3 TFU chains, 2 `unknown` refusals and 3 `first-sample` copies
  a frame; after the fix 5 chains, 0 and 0, the same as terrain alone. The
  prediction ("5 TFU chains a frame", no `unknown`, no `first-sample`) holds.
* With `RT_IMAGES = 16` (`v3d-rt-evict`, `31a4cf8d`, on the tip): both
  commits give 5 chains, 0 `unknown`, 0 `first-sample` from frame 2. Terrain's
  9 targets fit with the stale marks, so the fix removes nothing a frame
  there. Its gain on the board at the tip is therefore expected to be about
  0 ms, within run-to-run spread of the 35.7 ms record of
  `pi4-slow-terrain.md`. It is a correctness fix (a stale mark sends an
  uploaded texture down the render-target paths); only frame 1 differs.

## Commands

GLMARK2_SRC is `$CONNECTED_DEPS_DIR/glmark2-src` (glmark2 source,
`22c527cb`). `before` is a detached worktree at `c1b7e7d9`, `after` the
tree at `c6b16f71`. Both with the shipped `RT_IMAGES` (16):

```
bash tools/hostharness/pi4-terrain/multiscene.sh $GLMARK2_SRC build/ms-after
bash .wt-before/tools/hostharness/pi4-terrain/multiscene.sh $GLMARK2_SRC build/ms-before
```

Both again with `RT_IMAGES` set to 8 in scratch worktrees only
(`sed -i 's/pub const RT_IMAGES: usize = 16;/pub const RT_IMAGES: usize = 8;/' src/v3d/backend.rs`;
`build/ms-before8`, `build/ms-after8`; nothing of this is committed). The
default scene list is the Pi 4's: the blur and shadow desktop scenes, then
terrain, 20 frames each, in one process; then terrain alone.

Jobs, waits and registrations a frame (the multi-scene table has no column
for them) come from `tests/v3d_terrain_jobs.rs`, which replays one steady
terrain frame on the real V3D backend with the FBO-scene-first set-up:

```
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table \
  --test v3d_terrain_jobs -- --nocapture --test-threads=1
```

## Tables

Terrain after the two desktop scenes, one process, steady frame (frame 2 to
20; every frame the same), per frame:

| table | commit | chains | unknown | plain | first-sample | image-store |
|---|---|---|---|---|---|---|
| 8 | before `c1b7e7d9` | 3 | 2 | 0 | 3 | 4 |
| 8 | after `c6b16f71` | 5 | 0 | 0 | 0 | 7 |
| 16 | before | 5 | 0 | 0 | 0 | 7 |
| 16 | after | 5 | 0 | 0 | 0 | 7 |
| 8 or 16 | terrain alone | 5 | 0 | 0 | 0 | 7 |

Frame 1 of terrain (cumulative counters' first frame):

| table | commit | chains | unknown | plain | first-sample | image-store |
|---|---|---|---|---|---|---|
| 8 | before | 1 | 13 | 1 | 9 | 1 |
| 8 | after | 1 | 13 | 3 | 7 | 1 |
| 16 | before | 1 | 13 | 1 | 9 | 1 |
| 16 | after | 1 | 13 | 3 | 7 | 1 |
| alone | either | 1 | 13 | 3 | 7 | 1 |

Frame 1 after the fix equals terrain alone's: two copies that the stale
marks sent down `first-sample` are `plain` now.

`v3d_terrain_jobs`, FBO scene first, the third settled frame, `RT_IMAGES = 8`:

| | before | after |
|---|---|---|
| V3D jobs | 9 | 9 |
| render images registered (`imaged`) | 4 | 7 |
| CPU waits (`waits` = `backend_waits`) | 9 | 9 |
| waits in the swap | 0 | 1 |
| TFU chains queued | 2 | 5 |
| `unknown` refusals | 3 | 0 |
| `first-sample` copies | 4 | 0 |
| `image-store` copies | 4 | 7 |

The clean (terrain alone) test gives the "after" column on both commits. The
CPU waits stay 9: the fix removes CPU copying, not waits. Not measured here:
GL time a frame (GM2_PROFILE); the 14.6x host figure of `pi4-slow-terrain.md`
was at `RT_IMAGES = 8` without the fix.

## Prediction against host

| prediction (`pi4-slow-terrain.md`, candidate 1) | host |
|---|---|
| 5 TFU chains a frame | 5 (table 8: 3 before; table 16: 5 both) |
| no `unknown` refusal after frame 1 | 0 (table 8: 2 before) |
| `first-sample` +0 after frame 1 | 0 (table 8: 3 before) |
| frame about 30 ms | not a host value; jobs are 9 both before and after |

## Board values to expect (`pi4-board-after`, an image from a main with `c6b16f71`)

On the tip's 16-slot table:

* `glmark2-frame:` terrain, about the 35.7 ms of the last record (the fix
  gains about 0 there). If the image had `RT_IMAGES` 8 it would be ~30-36 ms
  against ~160 ms before.
* `yakogl/v3d: TFU mipmap at frame N:` chains grow by 5 a frame from
  terrain's frame 2 (frame 10 has 5 TFU jobs); `unknown` flat after frame 1.
* `yakogl/v3d: tiled copies for draws at frame N:` `first-sample` +0 after
  terrain's frame 1. In terrain's first frame, `first-sample` 2 lower and
  `plain` 2 higher than the record before the fix (host: 9 to 7 and 1 to 3).
* `image-store` 7 a frame, 9 jobs a frame.

## Test literal

`tests/v3d_terrain_jobs.rs`, `terrain_after_an_fbo_scene_that_deletes_its_textures`,
failed on main with the shipped table of 16: its literals were the stale-mark
values of a table of 8. They now are the clean frame's (5 chains, no refusal,
7 image stores, 1 swap wait). The multi-scene harness cannot reproduce the
stale-mark thrash on the tip by itself; its "before" is only visible with a
table of 8.

## After the dead window clear (d2767897)

Plan #329, item `pi4-terrain-host-after2` (#1209). Host only, no board. Tip
`d2767897` of the integration snapshot (`git fetch origin` is not possible
here); the clear fix is `4255adf1` (`gl: a full pending clear stays on its
framebuffer across a bind to another`, `draw::park_pending_clear`).

**Answer: 8 jobs a frame on the V3D path, not 9.** The fix applies there,
and `tests/v3d_terrain_jobs.rs` pins 8 on purpose.

### Why the test passed after the merge

It did not pass by accident. `4255adf1` itself changed
`tests/v3d_terrain_jobs.rs`: both tests now assert
`(f.jobs, f.draws) == (8, 8)` and `(waits, backend_waits, swap_waits) ==
(8, 8, 1)`, with a comment that the board's 9 was the window's clear as a job
of its own, emitted when the terrain FBO was bound. The test's double does not
hide the clear: it uses the real V3D backend and counts `v3d_counters` kicks.
The value 9 is what `docs/notes/pi4-terrain-host-after.md` above and
`pi4-slow-terrain.md` recorded before the fix (their tables are the
pre-fix state and now stale on that count). `v3d-terrain-gpu.md` candidate 3
("9 jobs to 8") is therefore already in the tip; nothing is stale in the test.
The header of the first test still quotes the old 9 only as history.

### Host run at the tip

`cargo test --no-default-features --features
host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_terrain_jobs --
--nocapture --test-threads=1`:

```
test a_clean_terrain_frame ... clean: settled after 3 frame(s): Frame {
    jobs: 8, imaged: 7, draws: 8, swap_waits: 1, waits: 8, backend_waits: 8,
    tfu_queued: 5, tfu_run: 0, chains: 5, refused: [],
    copies: [("image-store", 7)] }
ok
test terrain_after_an_fbo_scene_that_deletes_its_textures ... stale: settled after 3 frame(s): Frame { (the same values) }
ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

`pi4-terrain/run.sh` (GM2_PROFILE=1, YAKOGL_V3D_TFU=1, frames 2 to 20):
"v3d: jobs 8.00 a frame, CPU waits for a job 8.00 a frame"; 8.0 draws a
frame; TFU model jobs 5.00 a frame. Waits by call: `glGenerateMipmap` 5.00
(one each, the host TFU model finishing the job in flight), `eglSwapBuffers`
2.84, `glFinish` 0.16 (the harness's occasional finish). `multiscene.sh` (blur
and shadow desktop scenes, then terrain, 20 frames, one process; and terrain
alone): 5 chains a frame from frame 2, `unknown` 0, `plain` 0, `first-sample`
0, `image-store` 7; frame 1 is 1 chain, `unknown` 13, `plain` 3,
`first-sample` 7, `image-store` 1, as before.

The job list of a frame, from `terrain_frame` (one job a draw, 8 draws):

| # | target | stores a render image |
|---|---|---|
| 1 | height map | yes |
| 2 | normal map | yes |
| 3 | terrain target (reads height, normal, specular, 3 JPEGs) | yes; then glGenerateMipmap (TFU chain 1) |
| 4 | bloom H | yes; chain 2 |
| 5 | bloom V | yes; chain 3 |
| 6 | overlay blended into the terrain target | yes; chain 4 |
| 7 | tilt-shift H | yes; chain 5 |
| 8 | window (tilt-shift V, carries the window's COLOR|DEPTH clear) | no |

`imaged` 7 is jobs 1 to 7. The window clear that was job 0 before is gone.

### Predicted board values for `pi4-board-after2`

An image from a main with `4255adf1` (and `c6b16f71`, `31a4cf8d`):

* V3D jobs a frame: **8** (was 9). The window's clear no longer submits a job
  at the terrain FBO bind. CPU waits a job: 8 on the host double; on the board
  the waits poll `completed`, so the count of waits that really block is lower
  and not predicted here; `swap_waits` 1.
* `glmark2-jobs` (the per-scene counter line of
  `glmark2-scene-counters.md`, if the image prints it): terrain `jobs` 8 a
  frame, `tfu` 5 a frame, `tfu_run` 0, `refused` 0 after frame 1,
  `image-store` 7 a frame. Over N frames the job total is 8 N (9 N before).
  The log lines: `TFU mipmap at frame 10` shows 5 chains more each frame;
  `tiled copies ... first-sample` +0 after frame 1.
* Terrain ms: a job saved is about 0.5 to 0.8 ms of GPU time
  (`v3d-terrain-gpu.md` candidate 3, confidence high), so about 35 ms against
  the 35.7 ms record (a gain within run-to-run spread: it should read 0.5 to
  1 ms lower). It is not a verdict on speed. If the jobs entry reads 9, the
  image lacks `4255adf1`; if it reads 8 and ms did not move, the clear's
  cost was small, as the 0.5 to 0.8 ms estimate says.

No stale or wrong count was found, so there is no new issue to open from
this check.
