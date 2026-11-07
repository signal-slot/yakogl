<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GL core: which `settle_texture` drains a CPU mipmap filter needs, and the `wait_kick` alternative

Study, no code change (plan #329, item `gl-settle-drain-study`; Pi 4
candidate 4 of `pi4-slow-terrain.md`, #1209). Read at the main tip `e2a4e86b`;
every `file:line` is that tree's. Nothing was run on a board; board figures
are the cited notes'.

## Answer

1. **The drain cannot be narrowed for terrain: `wait_kick(target)` waits for
   exactly what `flush` + `finish` waits for.** The V3D runs jobs in submit
   order, two outstanding at most (`src/v3d/backend.rs:2919-2922`,
   `wait_job` `:3046`). A wait for kick K is a wait for K and everything
   before it. The drain can only be cut by a job *newer than the target's
   render* that is outstanding when the CPU filter starts. In terrain there is
   none: glmark2 renders into a target and calls `glGenerateMipmap` at once
   (`terrain-mip-usage.md`, the frame trace: `clear fb5; draw; genmip tex8`),
   and the next job samples the chain just filtered. So the window of
   "other jobs that could run" is empty.
2. **Expected gain on Pi 4 terrain: 0 ms, before and after
   `pi4-target-mark-clear`.** The two refused chains (bloom H, tilt-shift H,
   `v3d-terrain-copies.md`) wait for their own render job, which is the
   newest job. The "few ms" `pi4-slow-terrain.md` candidate 4 allowed for was
   not found, and the candidate should be dropped from the ranking. What
   remains is a no-gain, nonzero-risk change.
3. **What the mark fix changes is different:** a stale `TARGET_USE` mark makes
   `settle_texture` of a texture that is no target (a reused slot) take the
   `busy` branch and drain for nothing. That is `pi4-target-mark-clear`'s
   item, and it removes drains `wait_kick` would not.
4. **`DEPTH_TARGET` needs no lifetime fix of its own** (below). No new issue.

## `settle_texture`: what it does

`src/gl/texture.rs:900-934`. In order: `flush_clear_into` (`:865`, a pending
clear on the texture goes out), `resolve_depth` (`:698`), `ensure_storage`,
then the drain `:915-917`:

```
sampled_pending = !samples_texel_copies() && kick_pending(t.last_use)
busy = t.external || sampled_pending || kick_pending(texture_target_use(c, ti))
if busy && (flush() != 0 || finish() != 0) { drop the update }
```

then `chain_back` (`:880`, a backend-made chain returns to the storage) and
`storage_back` (`:893`, level 0 stored only in a backend's image goes into the
storage), then `stamp_texels` and `note_depth_texels_written`.

The three terms of `busy` are three different hazards:

| term | hazard | GPU access it waits for |
|---|---|---|
| `texture_target_use` pending | read-after-GPU-write (the render) and write-after-GPU-write (a render into levels 1..) | the kick that last rendered into the texture (`framebuffer.rs:96`, stamped `:196-199`, `:219`) |
| `sampled_pending` | write-after-GPU-read: a draw sampled the storage itself | `t.last_use`; only where the TMU/TPU reads the storage (`samples_texel_copies` false: PowerVR, GC7000, Mali; `src/backend.h:~620`) |
| `t.external` | an EGLImage shares the storage; any other process or context may use it | unknown kick: needs a full drain |

## Callers

Fourteen call sites besides the definition (`rg settle_texture src/gl/texture.rs`).
"CPU needs" is what must be true when `settle_texture` returns; "minimal wait"
is what a narrower wait could cover on a backend with `wait_kick` (the V3D).

| site (`texture.rs`) | call | CPU needs | minimal wait |
|---|---|---|---|
| `:2883` `tex_image_2d`, NPOT level | grow the chain (`grow_npot_chain`) | storage not in use at all: texels are copied to a new layout | the full set: target use, `last_use` (copies the storage at the relayout), external |
| `:2957` `tex_image_2d` with pixels | `glTexImage2D` upload into kept storage | no GPU write to the level (target use); no GPU read if the backend samples the storage | target use; plus `last_use` where texels are not copied |
| `:3086` `glTexSubImage2D` | CPU write of a rectangle | the same as above, and level 0 current (a sub-rectangle keeps the rendered rest: `storage_back`, `resolve_depth`) | target use (render done, then `storage_back`); `last_use` where not copied |
| `:3237` `glCopyTexImage2D`, `:3320` `glCopyTexSubImage2D` | CPU copy from the framebuffer | as `glTexSubImage2D`; the framebuffer read has its own full sync (`framebuffer.rs:2925` `gpu_idle_for_cpu`, flush + finish) | covered by the read sync, so narrowing this drain gains nothing |
| `:3778` compressed upload, `:3822` ETC1 decode, `:4025` compressed sub-image | CPU write of blocks | as `glTexSubImage2D`; ETC1 and compressed targets are never render targets | `last_use` only (no `texture_target_use`) |
| `:4213` `glGenerateMipmap`, NPOT grow | as `:2883` | as `:2883` | full set |
| **`:4298` `glGenerateMipmap`, CPU filter** | **reads level 0, writes levels 1..** | **render of level 0 done; `storage_back` done; no GPU job reading levels 1.. (storage-sampling backends)** | **target use alone on the V3D (see below); `last_use` where texels are not copied; the prefix kicks on PowerVR** |
| `:4454` `generate_mipmap_3d` | 3D CPU filter | same as `:4298`; 3D textures are not render targets in this GL core's backends | `last_use` only |
| `:5267` `glTexImage3D`, `:5411` `tex_sub_image_3d`, `:5705` `glCopyTexSubImage3D` | CPU write | as the 2D write | as the 2D write |
| `tf_capture.rs:1458` (comment) | names `settle_texture` for a capture target | not a call | n/a |

The V3D owns two more waits that settle depends on, outside the three terms:
`storage_back` → `v3d_storage_current` (`src/v3d/backend.rs:2114-2130`)
submits the open job that renders into the texture and writes level 0 back
from the render image (`rt_level0_back`); it runs after the drain, so the job
it needs is already done.

## The CPU filter (`:4298`): what it needs, step by step

`glGenerateMipmap` first offers the chain to the backend
(`backend::generate_mipmap`, `:4262-4286`): `Whole` returns with no wait at
all (the V3D's TFU path, `v3d_generate_mipmap` `:2319`, which queues the TFU
job behind the render and waits for nothing: `tests/v3d_tfu_mipmap_nowait.rs`).
The drain is reached only after a refusal (`MipChain::None`) or a `Prefix(n)`.

On the V3D the refusals terrain meets are `unknown` (TFU `layout.levels != 1`,
`pi4-slow-terrain.md`, `v3d-terrain-copies.md`): bloom H and tilt-shift H, two
a frame, plus the first frame's set-up.

- **Read-after-GPU-write of level 0.** The render job must have *completed*,
  not merely been submitted, and level 0 must be in the storage
  (`storage_back`). `kick_pending(texture_target_use)` is true until the OSAL
  fence is done (`v3d_kick_pending`, `:6325`). `wait_kick(target)` gives the
  same completion.
- **Write-after-GPU-read of levels 1...** On the V3D no pending draw reads the
  storage: draws sample tiled copies made when the draw was recorded
  (`samples_texel_copies: true`, `v3d/backend.rs:6442`), so `sampled_pending`
  is false by construction. On PowerVR and the GC7000 it is a real term:
  a previous frame's draw may still sample the old levels (`last_use`).
- **A backend still sampling the old levels.** The same, and a TFU job
  queued behind the in-flight job (`osal_v3d_tfu` queues, runs when the job in
  flight is finished) writes the *old chain's* image, not the storage the CPU
  filter writes, and that image is by construction invalidated by the
  `stamp_texels` of the CPU write: `generate_mipmap` refuses `Stamp` (`:2347`) on a
  later call.
- **Rendered-into levels 1...** `level_above_zero_attached` gates only the GPU
  offer (`:4272`); the CPU path runs for such a texture, and
  `texture_target_use` is per texture, not per level, so one mark covers
  every level.
- **The PowerVR `Prefix(n)` case.** The backend has drawn the first `n`
  levels and waited for them itself (`src/pvr/backend.rs`, `Backend::mip_cpu_tail`,
  off by default); the CPU then reads level `n` as the GPU wrote it. A
  `wait_kick(target)` would miss those newer kicks. The drain here is
  `finish()` (`pvr_backend_finish` `:9166`, `wait_fence(last_fence)`), which is
  correct; a narrower wait would have to name the last prefix kick, and the
  backend already waited for it, so the drain's only remaining cost is the
  target's own, which precedes the prefix in submit order.

## How each backend drains

| backend | `flush` / `finish` | `wait_kick` | `samples_texel_copies` |
|---|---|---|---|
| V3D | `v3d_flush` `:6197` submits the open job; `v3d_finish` `:6206` = `flush_job` + `wait_in_flight` (older, then newer job) | `v3d_wait_kick` `:6230`: kick 0 → nothing; kick not yet submitted → flush + wait all; the job in flight (or its alias) answers → `wait_in_flight`, which waits **both** jobs; the older `queued` job answers → `wait_queued` only; older than both → done | true (`:6442`) |
| PowerVR | `pvr_backend_finish` `:9166` waits the last fence; kicks run on several sets | `None` (`:9480`) → callers fall back to `finish` | false (`:9482`) |
| GC7000 | `src/vivante/backend.rs` | `None` (`:3381`) | false (`:3383`) |
| Mali | `src/mali/backend.rs` | `None` (`:3231`) | false (`:3232`) |
| host mock | `src/testutil.rs` | `None`, or `MOCK_BACKEND_WAIT_KICK` (`:1602`) which records `waited_kicks` (`:2127`) | false, or `MOCK_BACKEND_TEXEL_COPIES` (`:1607`) |

Only `v3d_wait_kick` can ever wait less than `v3d_finish`, and only in one
state: `queued` = the target's job and a newer `in_flight` job. Because the V3D
OSAL runs `queued` first, the target is then the older job. Two details of the
V3D implementation matter to any caller:

- **A kick that is the open job's alias is not flushed.** `v3d_wait_kick`
  treats `kick_id > g.kicks` as "not submitted" and flushes, but an
  `open_alias` stamp (`v3d_kick_pending` `:6325-6335`; a draw that ended the
  job it was stamped for) is `<= g.kicks` and matches neither job in flight
  yet, so `v3d_wait_kick` returns 0 with the job still open. The existing
  `settle_texture` flushes first, and any replacement must too:
  `flush()` then `wait_kick(use)`. The sync waits (`egl.rs:2583`, `es3.rs:326`,
  `:1110`) have their fence made after a flush, which is why they need not.
- **The error is the waited job's.** `finish` returns the first failure of
  the jobs it waited for; `wait_kick` the same for the jobs it waited for.
  `gpu_work_lost` counting is in `backend::counted`, not in the wait.

## What `tests/v3d_tfu_mipmap_nowait.rs` pins, and what it does not

Two tests, both the *GPU* path (the TFU), never the CPU filter:

- `the_tfu_job_is_queued_behind_the_open_render_with_no_cpu_wait` (`:828`): an
  open render job; `glGenerateMipmap` submits it and queues the TFU job behind
  its fence, no `wait()` inside the call; the TFU job runs before the sampling
  draw's job is submitted.
- `a_render_already_submitted_needs_no_wait_either` (`:871`): the render
  submitted and another job in flight after it: the TFU job is queued behind
  that job; render completed: the TFU job runs inside the call. No wait.

Not pinned anywhere: the order of a *refused* chain (the CPU-filter path), the
V3D `wait_kick` where `queued` holds the target and `in_flight` a newer job
(`src/v3d/backend_tests.rs:2829`
`wait_kick_waits_for_the_job_in_flight_and_leaves_the_open_job_open` pins the
newer-in-flight case for the sync wait only), and any `settle_texture` use of
`wait_kick` (`rg wait_kick src/gl/texture.rs` has no hit).

## Expected gain on Pi 4 terrain

| state | drain at `:915-917` | with `flush` + `wait_kick(target_use)` | gain |
|---|---|---|---|
| now (stale marks) | target job and everything before it | the same set | 0 |
| after `pi4-target-mark-clear` | the same for the 2 refused chains; no drain for a non-target | the same | 0 |

Why: at bloom H, the jobs outstanding are the terrain target's TFU job (queued
behind its render, not waited) and bloom H's own render (newest). At
tilt-shift H: the overlay's job and tilt-shift H's render (newest). Each
CPU filter's input is the job just recorded, and each next job samples its
output, so the dependency is a chain. The previous frame's last job (the
canvas draw) is older, so it was waited for by the swap
(`v3d_finish_previous` `:6258`) or finishes first by order. The GPU idles
through the ~100 ms of CPU filtering (`pi4-slow-terrain.md`) because nothing
independent exists to run, not because the drain is too wide. The way to
overlap GPU with the filter is to remove the filter (TFU or a per-level
`tfu_tile`, `v3d-tfu-raster-study`, `v3d-terrain-copies.md` candidates) or to
make the CPU loop cheaper (`mip-filter-wide`), not to narrow the wait.

An upper bound for a scene where the call comes later: the GPU time of the
jobs recorded between the target's render and the `glGenerateMipmap`. A
scene that renders A, then B, then mipmaps A has it; none in glmark2's
terrain, shadow or refract does that.

## Minimal wait per caller, if one were made

For the callers where it is not zero already:

1. The CPU filter (`:4298`) and the 2D writes: `flush()`, then
   `wait_kick(max(texture_target_use, last_use if the backend samples the
   storage))` when `wait_kick` exists; `finish()` when it does not, or when
   `t.external`. For terrain this is the same wait; for any caller with a
   newer outstanding job it saves that job's remaining GPU time.
2. The 3D filter and compressed writes: `last_use` only.
3. The NPOT grow (`:2883`, `:4213`, `:5267`): full set; the texels move.

## Test design (if someone changes it)

Host, no board, on `MOCK_BACKEND_WAIT_KICK` + `backend_samples_texel_copies`
(`backend_waits_for_kicks` `src/testutil.rs:2049`, `backend_samples_texel_copies` `:2057`; `waited_kicks()` `:2127`,
`set_kick_pending` `:2131`). Add a mock that has both. Cases:

1. a refused chain on a target with its render pending and nothing newer:
   `waited_kicks() == [target_kick]`, no `finish`, texels filtered after.
2. the same with a newer kick N pending (set_kick_pending(N)): the filter
   proceeds, `N` still pending (the point of the change).
3. the open-alias target stamp: the kick is flushed first (a mock that
   fails a `wait_kick` of an unsubmitted kick), the regression of the
   `v3d_wait_kick` detail above.
4. a backend with `samples_texel_copies` false and a pending `last_use`: waits
   for it.
5. `t.external`: `finish` as now.
6. `tests/es2_report_guard.rs` unchanged; bytes of the chain equal the old
   path's (`tests/gl_mip_cpu_filter_pin.rs`, item `mip-filter-pin`).
7. V3D side with the real backend, in the style of `tests/v3d_tfu_mipmap_nowait.rs`:
   refuse the chain (a size the layout refuses), run the platform's fake OSAL;
   events show `submit(target)`, `wait(target)`, no wait of a later job
   submitted before the call.

The ordering risk is item 3 and a `queued`/`in_flight` swap: waiting for the
`queued` job while the target is the `in_flight` one.

## `DEPTH_TARGET`: does it need the lifetime fix of `TARGET_USE`?

**No correctness fix; no new issue.** `DEPTH_TARGET` (`framebuffer.rs:108`) is
the pair `{level, resolved}` per slot, never cleared at slot reuse or free
(only moved by `orphan_moved` `:1389-1397` and zeroed in tests `:3763`). Its
readers are `resolve_depth` (`texture.rs:698-739`) and `depth_for_draw`
(`:836`), and both test `texture_target_use` first:

- `kick == 0` returns before `level` or `resolved` is read (`:701`, `:840`).
  Once `pi4-target-mark-clear` clears `TARGET_USE` on free and on hand-out,
  a reused slot's `level` and `resolved` are unreachable until the next
  render.
- The next render goes through `stamp_depth_target` (`framebuffer.rs:241-250`,
  from `draw.rs:2389`), which writes `level` and a new `TARGET_USE` together.
  `resolved` stays stale, but it is compared only for equality with that new
  kick (`kick == target.resolved`), and `resolved` was set from a kick that had
  been submitted and waited for (`:722` flush + finish, `:738`). Kick ids only
  grow (`v3d_kick_id` `:6312` = `kicks + 1`), so a later stamp cannot
  equal it (short of a 2^32 wrap).
- The backend's depth plane is keyed by `ds_object_id(t.serial, true)`
  (`context.rs:554`), and `serial` is a fresh `next_object_stamp` per texture object
  (`texture.rs:591`). A stale slot cannot read another texture's plane: the
  backend answers 1 for an id it does not hold (`:728`).

What a stale `TARGET_USE` does today for a *new depth texture* in the slot is
the cost, not an error: `resolve_depth` finds `kick != 0`, passes
`depth_texture_rendered`, may wait for the old kick (`:719`) and allocates the
deferred store (`:726`), then resolves nothing (rc 1) and sets `resolved`.
That is the same spurious drain as the colour case and the clear of
`TARGET_USE` removes it. For tidiness the same function may reset
`DEPTH_TARGET[ctx][ti]` to `{0, 0}` (as `orphan_moved` does for the moved-from
slot, `:1396`); it is not required. Test, for that item: delete a rendered,
unresolved depth texture, create another in its slot, `glTexSubImage2D` it,
and see no `kick_pending` query or wait for the old kick and no
`ensure_storage` before the upload's own.

## Limits

- No board run; the Pi 4 job order is from `terrain-mip-usage.md`'s host
  trace and `pi4-slow-terrain.md`'s description, not a new trace.
- `v3d_wait_kick`'s two-job reading is from the code, not a run.
- The AM62P CPU tail (`Backend::mip_cpu_tail`) was read, not measured; it is
  `pvr-tail-cacheable-study`'s subject.
- Row numbers of `texture.rs` may move when `mip-filter-wide` and
  `pi4-target-mark-clear` land.
