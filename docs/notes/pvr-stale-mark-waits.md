<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: does glmark2 terrain wait on stale "rendered into" marks? (AM62P)

Study, no code change. Question of plan #329 (issue #1199): `v3d-terrain-copies.md`
("The finding", design 1) says a texture created in a slot that an earlier
FBO texture freed inherits that texture's `TARGET_USE` mark
(`src/gl/framebuffer.rs:96`), so `settle_texture` waits for an unrelated kick
"on every backend". Here it is measured on the PowerVR path. **Answer: the
mark is inherited, but no wait follows from it on the AM62P. Terrain pays
0 ms a frame for it in steady state and at most a few microseconds once, at
set-up.** Read at main `723a75c8`; every line number is that tree's.

## How it was run

- Host harness, PowerVR back end (`tools/hostharness/glmark2/run.sh`,
  glmark2 `22c527cb`, `GM2_WRITE=0 GM2_KICKS=1`, 6 frames a scene). `run.sh`
  runs one process per list entry, so terrain alone never sees a stale mark.
  **Stand-in for the board's order:** the harness binary it builds
  (`build/<out>/glmark2-host-powervr`) was started once with `-b :nframes=6`
  and every list entry up to terrain as its own `-b` (`--gm2-list` order: build,
  texture, shading, bump, effect2d, pulsar, desktop x2, buffer x3, ideas,
  jellyfish, terrain; 23 scenes, one process, the board's order for the
  scenes before terrain). A second run of terrain alone through `run.sh` is
  the control. A scene's duration differs from the board's (6 frames, not
  10 s); that only makes the stale kicks younger here than on the board.
- Counting needed a temporary local patch, not committed and reverted: a
  `#[track_caller]` log line in `framebuffer::texture_target_use` and in
  `backend::kick_pending`, and a second table remembering the mark a freed slot
  held until a render overwrote it ("stale" = unchanged since `free_texture`,
  `src/gl/texture.rs:5009`). Frames are cut at the window draw (`fb 0`) of
  `GM2_KICKS`. The harness's FINDINGS files were not written.
- Host kicks complete inside `submit()` (`src/osal/linux.c`), so
  `kick_pending` is always false on the host and **a wait costs 0 ms there**.
  What the host gives is which calls reach a wait test with a stale id; what
  the board pays follows from the code, below.

## Counts per terrain frame (frames 2 to 6, identical in each)

| what | sequence run | terrain alone |
|---|---:|---:|
| bindings whose `target_kick` is non-zero only because of a stale mark | **2** (slots 4 and 6, marks 182 and 183, both left by `desktop effect=shadow`) | 0 |
| reads of a stale mark by `texture_target_use` | 6 (`src/gl/draw.rs:3952` x2, `src/gl/texture.rs:838` x2, `:699` x2) | 0 |
| `kick_pending` calls with a stale id | **0** | 0 |
| `kick_pending` calls with any non-zero id | **0** | 0 |
| CPU waits on the GPU through the GL core (`flush`/`finish` after a mark test) | **0** | 0 |
| reads of a live mark (terrain's own FBO textures) | 30 | 30 |

Terrain has 10 textures: 7 FBO targets (height, normal, specular, terrain,
bloom H, bloom V, tilt-shift H; slots 1, 2, 3, 7, 8, 9, 10) and 3 JPEG uploads
(slots 4, 5, 6). Slots 4 and 6 come back stale from the desktop scene; slot 5
was never an FBO texture. That is the same "2 of the 3 uploads" as on the Pi 4.
A stale mark survives `ideas` and `jellyfish` (neither renders into a texture)
and is freed again, still stale, when terrain ends.

Call sites of the six reads in a frame, all benign on the PowerVR:

- `src/gl/draw.rs:3952`, `texture_target_use` while building each sampler
  binding, into `GlesTextureBind::target_kick` (`:4015`). The PowerVR back end
  never reads that field for a 2D texture (`src/pvr/backend.rs` has no read of
  `target_kick`); the draw reads it only for the native cube, 2D array and 3D
  copies (`draw.rs:4049`, `:4079`, `:4115`), which need `sh.native_cube`
  bits terrain's shaders do not set.
- `src/gl/texture.rs:838` (`depth_for_draw`) and `:699` (`resolve_depth`): both
  return before any wait, because `depth_texture_rendered(t)` is false for a
  colour texture (`:706`; `depth_for_draw` then falls to `resolve_depth` and `ensure_storage_at`).

## Counts at set-up (terrain's first frame, once)

9 `kick_pending` calls with a stale id, all `false` on the host:

| call site | calls | slots (stale mark) |
|---|---:|---|
| `src/gl/texture.rs:2092-2093` `note_target_use`, from `tex_image_2d` (`:2728`, glTexImage2D) | 7 | 1 (209), 3 (181), 4 (182), 6 (183), 7 (174), 8 (184), 10 (185) |
| `src/gl/texture.rs:916` `settle_texture`, from glGenerateMipmap's NPOT branch (`:4213`) | 2 | 7 (174), 10 (185): the two 800x600 textures |

Terrain alone makes none of these calls (no mark yet).

## What the AM62P pays for one

`backend::kick_pending` (`src/backend.rs:1796`) is `pvr_backend_kick_pending`
(`src/pvr/backend.rs:9428`): `id != 0 && id > completed_fence()`. A stale
id costs one `completed()`: `z_completed` (`src/osal/zephyr.c:624`) ->
`pvr_render_completed` -> `render::completed` (`driver/rust/src/gpu/pvr/render.rs:1664`),
one 4-byte cache invalidate and read of the fragment UFO, plus
`drain_pending_frees`. A few microseconds at most; no polling loop, none of the
1 ms `wait_for` steps of `pvr-fence-wait.md`.

Whether it is a **fence wait on a retired kick or a real wait on an in-flight
one** is decided by the age of the id. The fragment UFO is one in-order
timeline; the swap waits for the previous frame
(`finish_previous`, `pvr-fence-wait.md`), so at most about 2 kicks are in
flight at any moment (plus a mipmap chain's kicks inside a frame). A stale mark
is the last render into a slot by an earlier scene. Between that scene's last
frame and terrain's set-up lie whole scenes (here the marks 182-185 are about 60
kicks before terrain's first kick at ~241; on the board a scene runs
seconds). So `id <= completed` always: **a retired kick, `kick_pending` false,
no `flush()`/`finish()`**. The only way to hit the real wait is a stale kick of
the scene directly before terrain with no CPU time between (a hand-made list
`desktop, terrain` still has terrain's shader compiles and JPEG decodes, many
ms, before `texture.rs:2093`).

Expected cost on the AM62P:

| | per frame | once, at set-up |
|---|---:|---:|
| 9 `completed()` reads | 0 | 9 x a few us, **< 0.1 ms** |
| bound if a stale kick were in flight | n/a (0 calls a frame) | `flush()` + `finish()`: the 1 ms-quantised wait for <= 2 kicks of ~1.8 ms, **<= 4 ms** (not reached, see above) |
| 2 stale bindings a frame | 2 array reads, ns | |

So **0 ms a frame (bound: 0 calls, so no error bar is needed), < 0.1 ms in the
whole run.** Terrain's 83.06 ms (`tools/cts/RESULTS.md`, `6a7021ec`) has nothing
of it. The waits that make that frame are elsewhere: the backend's own
(`wait_fence` in `mip_chain_gpu`, `finish_previous` at the swap), not a
GL-core mark test; the live-mark tests above never reach `kick_pending` in a
frame either (30 reads, 0 calls).

## Does `pi4-target-mark-clear` remove them?

Yes, by construction (not run: the fix is a running item): with the mark zero
at free or at hand-out, `texture_target_use` returns 0, the two stale bindings
carry `target_kick == 0`, and the 9 set-up calls become `kick_pending(0)`,
which `src/pvr/backend.rs:9432` answers false by its `kick_id != 0` guard before
reading the fence. The host log above shows the stale reads are the only
difference between the two runs (live reads: 30 a frame in both). Removing them
changes no PowerVR wait, because there was none.

## Board record value that confirms it

The AM62P glmark2 record (`records/am62p/common-<commit>/...-GM-.../`, the
console log's `terrain` per-scene ms/frame and the `RESULTS.md` Common-set
row, 83.06 ms at `6a7021ec`): **no line moves.** Terrain stays at 83.06 ms
within the run-to-run spread, and shadow (10.62) and refract (35.09) too, in a
board run built from a main with `pi4-target-mark-clear` against one without.
A change of terrain's ms by more than the spread would contradict this note and
point at a wait this study did not see. The only counter that can differ is on
the Pi 4 (`first-sample`, `unknown`, TFU chains), not on the AM62P.

## Not found

No defect other than the stale mark itself (harmless here). Left for the other
items of plan #329: where the 83 ms goes (`am62p-slow-terrain.md`, the mipmap
kicks, `am62p-pvr-mipmap-kicks.md`).
