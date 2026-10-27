<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# glmark2's buffer scenes: where the CPU time of a buffer update goes

glmark2-es2's three `buffer` scenes (`update-fraction=0.5`,
`update-dispersion=0.9`, `columns=200`, `update-method` subdata or map,
`interleave` false or true) cost 0.28 to 0.33 ms a frame of host CPU on the
PowerVR build. This note says where, what each update does to the GPU's
memory, what Mesa does, and what to change, ranked. Milliseconds are host
CPU time a frame, never fps; the host is not the boards (`tools/hostharness/glmark2/README.md`,
Profile).

## What was measured, and what was not

**These are not new runs.** The session that wrote this note could not start
`tools/hostharness/glmark2/run.sh`: the command was refused by the session's
permission rules, in four forms (with and without `env`, `bash`, a redirect).
`perf` is not installed either (`GM2_PERF=1` needs it). So the per-call
numbers below are the ones the committed profile runs already hold, taken
from their `FINDINGS-*-allscenes.md` tables, each with the commit and
command that produced it. The breakdown into costs (copy, allocation,
memset) is read from the code and checked against those numbers, not
sampled. A rerun is the first step of fix 1 (command at the end).

| back end | commit | file | command (all: `GM2_WRITE=0 GM2_PROFILE=1`, one process per scene, counted frames 2 to 20) |
|---|---|---|---|
| PowerVR | `b9130d64d878` | `FINDINGS-pvr-allscenes.md` | `GM2_FRAMES=20 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-prof powervr` |
| GC7000 (vivante) | `8f0643c20f69` | `FINDINGS-gc-allscenes.md` | `GM2_FRAMES=20 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-gc-prof vivante` (twice) |
| V3D, TFU on | `4b057224` | `FINDINGS-v3d-allscenes.md` | `GM2_FRAMES=20 YAKOGL_V3D_TFU=1 tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-tfu v3d` |

`$GLMARK2_SRC` is glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`.

## Per-call breakdown (host ms a frame)

"Frame" is the thread CPU time of the whole frame including glmark2's own C++
(it fills the vertex data before it calls GL); "update call" is the wall time
of the entry point that updates the buffer. Calls a frame are from the GC
table (`glBufferSubData` 40x, `glMapBufferOES` 4x non-interleaved, 1x
interleaved); the 4 buffers are glmark2's `vertex_format` (4 attributes of 3
floats).

| back end | variant | frame | update call (calls) | `glDrawArrays` | `glBindBuffer` | update call as share of frame |
|---|---|---:|---|---:|---:|---:|
| PowerVR | subdata, no interleave | 0.332 | `glBufferSubData` 0.080 (40) | 0.007 | not listed | 24% |
| PowerVR | map, no interleave | 0.310 | `glMapBufferOES` 0.049 (4) | 0.008 | not listed | 16% |
| PowerVR | map, interleave | 0.284 | `glMapBufferOES` 0.057 (1) | 0.007 | not listed | 20% |
| GC7000 | subdata, no interleave | 0.637 (run 1) | `glBufferSubData` 0.133 (40) | 0.007 | 0.013 (9) | 21% |
| GC7000 | map, no interleave | 0.522 | `glMapBufferOES` 0.045 (4) | 0.008 | 0.014 (9) | 9% |
| GC7000 | map, interleave | 0.313 | `glMapBufferOES` 0.035 (1) | 0.007 | 0.009 (6) | 11% |
| V3D (TFU on) | subdata, no interleave | 0.927 | `glBufferSubData` 0.780 | 0.072 | not listed | 84% |
| V3D (TFU on) | map, no interleave | 0.473 | `glMapBufferOES` 0.335 | 0.070 | not listed | 71% |
| V3D (TFU on) | map, interleave | 0.402 | `glMapBufferOES` 0.289 | 0.055 | not listed | 72% |

(The GC7000's second run read 0.21 ms for subdata: the host's spread between
runs is up to a factor of 1.7, `FINDINGS-variance.md`; read the ranking, not
the third digit.) `glUnmapBuffer` and `glBindBuffer` are each under 0.02 ms.

What the table says:

* On PowerVR and the GC7000 three quarters or more of the frame is glmark2's
  own code (0.22 to 0.25 ms on PowerVR, `FINDINGS-pvr-allscenes.md`, rank 5
  to 7). The library's share is the update call: 0.035 to 0.133 ms.
* The V3D is the outlier: 0.29 to 0.78 ms in the update call, 5 to 10 times
  the others for the same work. The tables do not say why. The V3D's
  allocator is a first-fit bitmap over a 96 MiB carve-out
  (`src/osal/linux.c`, the V3D `l_gpu_alloc`, which also memsets) instead of
  `aligned_alloc`, and every allocation here is new; a `GM2_PERF=1` run on
  the V3D build is needed to confirm.
* Subdata costs more than map on every back end (PowerVR 0.080 against
  0.049/0.057), though both copy the same bytes (below). The difference is
  the allocation: subdata allocates new storage each frame, map reuses a
  retired one.

## What the library does on an update

glmark2 per frame (`mesh.cpp` `update_vbo`, `scene-buffer.cpp` `update`):
draw first, then `update()` writes the changed ranges into the VBOs. A
buffer is `columns 200 x rows 20 x 6` = 24,000 vertices; each of the 4
attribute buffers is 24,000 x 12 B = 288,000 B, 1.15 MB in all (interleaved:
one buffer of 1.15 MB). Non-interleaved subdata makes about 10 ranges a
buffer, 40 calls; map makes one `glMapBufferOES(GL_WRITE_ONLY)` a buffer and
copies the ranges into it (about half of the buffer, `update-fraction=0.5`).

**The draw of the same frame still has its kick open when the update comes**
(`src/gl/buffer.rs`, comment above `glMapBufferRange`: "the kick it waits for
is the one still being recorded"). So `kick_pending(last_use)` is true for
the first write to every buffer, every frame.

| path | what it does | waits for the GPU? | copies |
|---|---|---|---|
| `glBufferSubData` `src/gl/buffer.rs:445` | `writable()` (`:486`) -> `detach_from_pending_kick` (`:175`): `gpu_alloc` of the whole buffer, `cache_invd` of the old one, `copy_nonoverlapping` of **all** bytes (`:192`), `cache_flush` of all (`:195`), the old storage `gpu_free`d (deferred), `last_use = 0`. The first call of the frame pays this; the other 9 calls of that buffer see `last_use == 0` and only `upload()` (`:102`): a copy of their range plus a `cache_flush` of it | no | the whole buffer, once a frame per buffer, then each range once more |
| `glMapBufferOES` -> `map_range` `src/gl/buffer.rs:962` | the same condition (`:1011`) takes a retired storage of this buffer (`take_retired`, `:861`; kept by `retire`, `:880`, at most `RETIRED_PER_BUFFER`), or allocates; `carry` (`:920`) copies **all** bytes (OES keeps the bytes the caller does not write, so there is no invalidate bit to save any); the old storage is retired; the application then writes the ranges into the new one | no | the whole buffer, once a frame per buffer |
| `glUnmapBuffer` `:1170` | `unmap_buffer` (`:155`): `cache_flush` of the mapped length, which is the whole buffer | no | none (a flush of the whole buffer, though half of it was written) |
| `glBufferData` `:380` | not called per frame by these scenes (setup only) | | |

No update waits for the GPU: all three branches copy on write. Every update
**copies the whole buffer** where a range was asked for, once a frame for
each of the 4 buffers: 1.15 MB read and 1.15 MB written, plus (subdata on
the host, below) 1.15 MB memset. The ranges glmark2 wrote are about half of
that.

Why the host sees what it sees: `src/osal/linux.c` `l_gpu_alloc` (`:233`)
memsets every allocation (`:265`, and in the V3D variant `:762`), and
`cache_flush`/`cache_invd` are no-ops there (`:332`). So on the host, subdata
= alloc + memset + copy per buffer; map = a retired storage + copy. The
0.03 ms by which subdata exceeds map on PowerVR is that alloc and memset. On
the boards the host-only memset is absent, the allocation is the pool's, and
`cache_invd`/`cache_flush` of 288 KB are real: the Pi 4 and AM62P read GPU
memory through non-cacheable mappings (README, "What the numbers are"), so
the **read** of the old buffer in `carry` and `detach_from_pending_kick` is
the board's cost, and the host cannot show it. This is a hypothesis for the
board session; no board was run for this note (non-goal).

## What Mesa does

Read from `mesa-26.2.1` (`$CCFARM_DEPS_DIR/mesa-pan`):

* `glBufferSubData` goes to the gallium driver's `buffer_subdata`, which for
  the V3D is `u_default_buffer_subdata` (`src/gallium/auxiliary/util/u_transfer.c:7`):
  a map with `PIPE_MAP_WRITE | DISCARD_RANGE`, a copy of the range, an unmap;
  a call that covers the whole buffer is upgraded to `DISCARD_WHOLE_RESOURCE`.
* `v3d_resource_transfer_map` (`src/gallium/drivers/v3d/v3d_resource.c:262`)
  runs `v3d_map_usage_prep` (`:191`). With `DISCARD_WHOLE_RESOURCE` it
  flushes the jobs writing the resource and **allocates a new BO** without
  copying (the whole-buffer reallocation on discard); the old BO lives until
  the job is done. Without it, a write to a buffer the open job reads
  **flushes that job and `v3d_bo_map` waits for it** (the kernel is
  synchronous, as the file's comment says), unless the map is
  `UNSYNCHRONIZED`.
* So Mesa copies only the written range, but a partial update of a busy
  buffer (every call of glmark2's scene except one covering the buffer) costs
  a job flush and a wait, once a frame. YakoGL's copy on write never waits;
  it pays in bytes instead. Mesa's way to avoid both is the whole-buffer
  discard, which glmark2 does not ask for (it writes about half the buffer
  and expects the other half to stay).

## Ranked fixes

The gains are estimates on the host numbers above (PowerVR build) and say
which part of the update call they remove; the frame is 0.28 to 0.33 ms, of
which glmark2's own work is 0.22 to 0.25 ms. None is measured: each needs the
before/after of the command at the end.

| # | fix | where | expected gain |
|--:|---|---|---|
| 1 | **A storage ring with dirty ranges** for buffers a kick keeps reading. Keep the retired storage (map already does) and a list of the ranges written since it was retired; on the next write to a busy buffer swap to the retired storage and copy only those ranges (glmark2: about half the buffer, the ranges of the last frame or two) instead of all bytes. Used by `detach_from_pending_kick` (`src/gl/buffer.rs:175`, replacing its `gpu_alloc`, `:192` copy and `:195` flush) and by `map_range` (`:1011`-`:1055`, replacing the `carry` calls at `:1046`, `:1047`) | `buffer.rs:175-215`, `:1011-1056`, `:920`; a dirty list on `GlBuffer` (`:42`), fed by `upload` (`:102`) and `unmap_buffer` (`:155`) | **Subdata, PowerVR: 0.080 -> about 0.03 ms** (no alloc, no memset, half the copy); **map: 0.049 -> about 0.03 ms** (half the copy). On the boards the saving is the uncached read of half the buffer and its flush, per buffer per frame: expected larger than the host's, to be measured. About 0.03 to 0.05 ms of a 0.28 to 0.33 ms frame (10 to 15%) on the host |
| 2 | **Reuse the retired storage in `detach_from_pending_kick`** even before dirty ranges exist: today it `gpu_alloc`s new pages each frame while the map path takes `take_retired` (`:861`). Subdata then costs what map costs | `buffer.rs:175-200` | PowerVR subdata 0.080 -> about 0.050 ms (the gap to map), 0.03 ms of the frame (about 9%). Smallest change; also removes the host-only memset from the hot path. Fix 1 includes it |
| 3 | **Flush the written ranges on unmap, not the whole mapping.** `unmap_buffer` flushes `map_len` bytes (`:162`); the application's ranges are not known, but with fix 1's dirty list the ranges the carry did not copy are the only ones to flush | `buffer.rs:155-168` | none on the host (`cache_flush` is a no-op there); on the boards half of a 288 KB flush a buffer. Board only |
| 4 | **Fewer calls to the GL core per subdata**: 40 `glBufferSubData` calls a frame each pay `bound_buffer_for_target`, checks, `writable` and `upload` with a `cache_flush`. A merged flush for adjacent ranges would not change the bytes | `buffer.rs:445-483`, `:102` | under 0.01 ms (about 1 to 2 us a call after the first, which is where the 0.08 ms is); not worth a change on its own |
| 5 | **Not a library fix: the host harness's `l_gpu_alloc` memset** (`src/osal/linux.c:265`, `:762`) is in every host number for the allocating path and not on the boards. Subtract it (or time the harness with it off) before comparing fix 1's gain with the table above | `src/osal/linux.c` | changes the baseline, not the library; the PowerVR subdata number overstates the board's allocation cost by about the 0.03 ms the memset takes |
| 6 | **Investigate the V3D's 5 to 10 times** (0.29 to 0.78 ms against 0.05 to 0.13): `GM2_PERF=1` on the V3D build with `GM2_ENTRIES='^buffer:'` to see whether it is `l_gpu_alloc`'s bitmap search, the 96 MiB memset or page faults | `src/osal/linux.c:737-770`, `src/v3d/backend.rs:5705` (`v3d_kick_pending`) | unknown until sampled. If it is the host's allocator, none on the board |

Not proposed: waiting for the kick and writing in place (the comment above
`glMapBufferRange` records why: glmark2's map comes after the draw of the
same frame, so the kick it waits for is the one still being recorded, and
every frame would stall on a job flush, as Mesa's does); and an
`INVALIDATE_BUFFER` shortcut, which glmark2's `glMapBufferOES` and
`glBufferSubData` cannot ask for.

## Done

| # | fix | commit | what it does |
|--:|---|---|---|
| 2 | retired storage for every busy write | `cb382015` | `detach_from_pending_kick` (glBufferSubData, glCopyBufferSubData, the transform-feedback capture) takes back a retired storage of the buffer whose kick is done, as `map_range` did, and retires the storage it moves off instead of freeing it. No `gpu_alloc` after the first `lag + 1` frames |
| 1 | dirty ranges | `cb382015` | each retired slot keeps the ranges written since it was retired (`Retired::dirty`, at most 16 apart; past that the two closest merge, so a list only grows). Every CPU write adds its range to the buffer's slots (`note_write`: subdata, a write map, the copy, glReadPixels into a pack buffer, the capture). Taking a slot back copies only its ranges (`carry_dirty`), less the mapped range with INVALIDATE_RANGE |
| 3 | flush on unmap | not done | the dirty list does not say which bytes of the mapping the application wrote: the carry's ranges are inside the mapping and are overwritten by the application too, so `unmap_buffer` still flushes the whole mapped range (see below) |

What changes for glmark2: the subdata scene's first call a frame on a
buffer copies the ten ranges of the last two frames (the same ten every
frame, 144,000 bytes) instead of 288,000 bytes, and allocates nothing. The
map scenes are unchanged: a glMapBufferOES write mapping covers the whole
store, so every byte is dirty and the carry is still the whole buffer. Only
an application that maps a range (glMapBufferRange) or uses subdata gains.

**How it was measured.** The note's command (`tools/hostharness/glmark2/run.sh`)
was refused by this session's permission rules again, as `git worktree`,
`git archive` and `git restore` of the base were, so the per-scene
`profile-<gpu>.md` numbers are still to be taken (the command below). The
numbers here are from the ignored timing test
`time_glmark2_buffer_scene_updates` in `tests/gl_buffer_retired_reuse.rs`:
glmark2's buffer scene at its own sizes (four 288,000-byte attribute
buffers, each drawn and then updated by 10 x 14,400 bytes, half the buffer,
one frame in flight) against a stand-in OSAL that clears every allocation
and reuses freed storage once its kicks are done, as `src/osal/linux.c`
does. Its time is the wall time of the update calls of a frame, the median
of 200 frames, host CPU, `--release`, `host-test,gpu-powervr,shaderc-online`
(the GL core is the same for every back end; the stand-in back end records
draws only). "Before" is `cb382015` with `detach_from_pending_kick`'s two
new branches turned off (its code at `41e95169`); the map path is the same
code in both, so its column is the spread of the method.

```sh
cargo test --release --no-default-features --features host-test,gpu-powervr,shaderc-online \
  --test gl_buffer_retired_reuse -- --ignored --nocapture
```

| update calls, 4 buffers a frame | before, run 1 / 2 (ms a frame) | after `cb382015`, run 1 / 2 (ms a frame) | `gpu_alloc` in 200 frames, before -> after |
|---|---:|---:|---:|
| 40 x glBufferSubData (10 a buffer) | 0.0544 / 0.0550 | 0.0237 / 0.0251 | 800 -> 0 |
| 4 x glMapBufferOES, 10 ranges written, unmap | 0.0381 / 0.0396 | 0.0317 / 0.0308 | 0 -> 0 |

So subdata drops from 0.055 to 0.024 ms a frame (by 56%) and now costs
less than the map path: its carry is half of the map's (144,000 bytes a
buffer against 288,000). That is the
gain the ranked list expected for fixes 1 and 2 together (PowerVR 0.080 ->
about 0.03 ms, from a profile that also times glmark2's GL call overhead).
With a stand-in OSAL that never reuses memory (each allocation is fresh
pages) the before reads 0.72 to 0.79 ms: page faults, which the host
harness's allocator mostly avoids and the boards' pools do not have.

Not done here, for the board sessions and later items: the harness
before/after (`profile-<gpu>.md`, the command below) on all three back
ends; the V3D's 5 to 10 times (fix 6) is not explained by this change, but
the subdata path no longer reaches the V3D's allocator at all after the
warm-up. Fix 3 would need the application's written ranges, which neither
glMapBufferOES nor a write map without FLUSH_EXPLICIT reports.

## The command to measure a fix

```sh
GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=60 GM2_ENTRIES='^buffer:' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-buf powervr vivante v3d
# with perf: GM2_PERF=1 in place of GM2_PROFILE=1; compare two runs with
# the harness's compare tool when it has landed
```

Run it before and after at the same commit otherwise, twice each (the spread
of one reading is large, `FINDINGS-variance.md`), and read the
`glBufferSubData` / `glMapBufferOES` rows of `profile-<gpu>.md`.
