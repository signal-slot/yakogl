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
