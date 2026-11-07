<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# A reused texture slot and the DEPTH_TARGET entry

Question (left open by pi4-slow-terrain.md, "Not in this note"):
`DEPTH_TARGET` (`src/gl/framebuffer.rs:108`) has the same lifetime as
`TARGET_USE` (`:96`). A new depth texture that gets a deleted depth
texture's slot inherits the slot's `level` and `resolved`. Can that make a
depth resolve skip, or read the wrong plane?

Answer: **no on all four back ends, for correctness. Yes for cost: a stale
`TARGET_USE` stamp (not `DEPTH_TARGET`) makes the new texture wait for the
old texture's kick.** `DEPTH_TARGET` needs no clearing for the answer to
stay right, but clearing it with `TARGET_USE` is free and removes the one
case where a stale `resolved` can suppress a resolve (the kick-number
caveat below). Read from `a8e79b52`; no code changed.

## The sequence, with the code

1. Texture A, slot `k`, is rendered as a depth attachment. Each draw stamps
   it: `draw.rs:2389` calls `stamp_depth_target` (`framebuffer.rs:241`),
   which sets `TARGET_USE[ctx][k]` through `stamp_target_use` (`:196`, moved
   to the backend's kick by `restamp_draw`, `draw.rs:527`) and sets
   `DEPTH_TARGET[ctx][k].level` from the attachment (`:248`).
2. A is resolved (sampled on a CPU path, or `settle_texture`):
   `resolve_depth` (`texture.rs:698`) ends with `set_depth_resolved(c, k,
   kick)` (`:738`, `framebuffer.rs:260`). Or it never is (V3D/PowerVR
   sample the buffer, `depth_for_draw`, `texture.rs:837`, which does not
   set `resolved`).
3. `glDeleteTextures(A)`: `names::forget`, `unbind_texture`,
   `framebuffer::texture_deleted` (`texture.rs:4979`), then `free_texture`
   (`:5009`), or, for an orphan held by another framebuffer, later through
   `attachment_released` (`framebuffer.rs:1443`). `free_texture` calls
   `backend::attachment_gone(ds_object_id(serial, true))` and
   `zero_texture`. **Neither touches `TARGET_USE` or `DEPTH_TARGET`**; only
   `orphan_moved` (`framebuffer.rs:1379`) moves both rows, and zeroes the
   source entry, consistently.
4. `glGenTextures` (`texture.rs:612`) hands out the lowest unused slot, so B
   gets `k` at once; `init_texture` (`:588`) zeroes the struct and takes a
   **new** `serial = next_object_stamp()` (`:591`, `context.rs:544`, never
   reused). Tables are not touched. B inherits `TARGET_USE[k] = K1` (A's last
   render kick) and `DEPTH_TARGET[k] = {level L, resolved r}`.
   (`GlTexture`s are per share group, the tables per context slot: a second
   context of the group that rendered into A has its own stale entry for `k`
   too.)

Everything the back ends keep for a depth texture is keyed by
`ds_object_id(serial, true)` (`context.rs:554`; `draw.rs:2425`), and the
serial is new for B. So a stale slot entry can only change what the core
does *around* the backend call, never which plane the backend reads.

## What the stale entry does in the core

Let B be a depth texture (`depth_texture_rendered`, `texture.rs:745`) that
no draw has yet rendered into. Its slot says "rendered by K1, level L,
resolved r".

* **r == K1** (A was resolved at K1): `resolve_depth` and `depth_for_draw`
  return at once (`kick == target.resolved`, `texture.rs:699`, `:840`).
  Correct: B holds nothing to resolve. Once B is drawn, the draw stamps a
  new, later kick K2 (`draw_stamp`/`restamp_draw`), K2 != r, so the first
  resolve of B runs. A skip would need K2 == r (see the caveat).
* **r != K1** (A never resolved, e.g. glmark2 shadow, whose depth is
  sampled from the buffer): `resolve_depth` goes on. It does
  `if backend::kick_pending(kick) && flush()/finish()` (`:709`): **a full
  flush and finish for A's kick**, then `ensure_storage` (a deferred store
  is allocated, `:714`), then `backend::depth_resolve(id_B, ...)`
  (`:729`). The level check at `:704` only returns early when `L >=
  chain_levels` of B; otherwise L is used to size the write, but the
  backend answers 1 first (below), so no texel is written. Then
  `set_depth_resolved(c, k, K1)` (`:738`) makes the next call skip. B's
  texels are untouched, which is right: nothing rendered B.
* `depth_for_draw` (`:837`) for B, never rendered: `samples_rendered_depth(
  id_B, ...)` is false (no plane for the id), it falls to `resolve_depth`
  and `ensure_storage_at`; the draw samples B's texels. Right.
* `settle_texture` (`:900`): `resolve_depth` as above, and then `busy =
  ... kick_pending(texture_target_use(c, ti))` (`:916`) makes a CPU write
  into B (glTexImage2D/SubImage) flush and finish for A's kick again.
  Correct data, wasted wait. This is the same stale-mark wait that item
  pi4-target-mark-clear fixes for `TARGET_USE`. With `TARGET_USE` cleared,
  `kick == 0` and `resolve_depth` returns at `:699` before reading
  `DEPTH_TARGET` at all.
* The stale `level` is dead once B is drawn: every draw with a depth
  attachment rewrites it (`framebuffer.rs:248`), before the draw's own stamp
  can be resolved. A draw that has B attached with no depth write stamps
  nothing (`has_depth` false, `draw.rs:2388`), so the level stays stale but
  `TARGET_USE` stays too; the resolve reaches the backend, which holds no
  plane (below).
* Blit/other writers (`framebuffer.rs:1188`, `:1233`, `:1496`) stamp
  `TARGET_USE` only, never `DEPTH_TARGET`; same effect as above.

## Verdict per back end

All three ask the backend by B's id (`texture.rs:729`, `:848`); none has a
plane for it, because `free_texture` called `attachment_gone(id_A)` and B's
id is new.

* **V3D** (`src/v3d/backend.rs`): `v3d_depth_resolve` (`:4780`) finds the
  buffer with `b.owners[0] == id` (`:4785`); none for id_B, returns 1,
  texels left. `v3d_depth_sampled` (`:4210`) uses `depth_buffer_of(g, id)`
  (`:4171`); none, false. **Correct**; cost: one spurious flush/finish when
  K1 is pending, one `ensure_storage` of a deferred store.
* **PowerVR** (`src/pvr/backend.rs`): `pvr_backend_depth_resolve` (`:4908`)
  returns -EBUSY only if the open frame draws into id_B (`:4930`, not the
  case before B's first draw); otherwise it looks up `s.owners[DEPTH] == id`,
  and the parked list by id, finds nothing, returns 1.
  `pvr_depth_sampled` (`:4812`) goes through `depth_buffer_sampleable`
  (`:4784`) by id: none, false. `pvr_backend_attachment_gone` (`:4749`)
  undefined A's planes and flushed A's open frame. **Correct**, same cost.
* **Vivante** (`src/vivante/`): `gc_depth_resolve` (`backend.rs:2995`) ->
  `ds_cache::resolve_depth` (`ds_cache.rs:562`): no entry with `key.id ==
  id_B`, nothing parked, returns 1 (`:585`); `gone` (`ds_cache.rs:410`)
  undefined A's planes. The core never uses `depth_for_draw`'s buffer path
  here (`backend::samples_rendered_depth` is false without v3d/powervr
  features), the draw samples the texels. **Correct**, same cost.
* **Backend without the entry** (`backend::depth_resolve`, `backend.rs:2048`
  answers 1): texels as they are. **Correct.**

## The one thing that could go wrong: the kick number

`resolved == kick` is a "this kick's depth is in the texels" test across
the slot's whole life. A skip of a real resolve of B needs a stamp K2 of
B's draw that equals `r` left by A. Kick ids are the backend's monotonic
counter and A's K1 was flushed (a resolve flushes, `texture.rs:709`; the
sampled path flushes, `:848`) before B can exist, so a later draw is in a
later kick. I found no path where it repeats, except one not examined: a
backend re-init or a test mock that restarts `kick_id()` from 1 while the
process (and these static tables) lives on. `tests/` reset the tables with
`framebuffer.rs:3762` only in the in-crate test setup. If a back end ever
restarts the counter, `resolved` from before is a skip (stale data left in
B's texels, the depth pass never resolved) and `TARGET_USE` is a wrong
wait. Clearing both on free (below) removes it.

## Must DEPTH_TARGET also be cleared?

Not for correctness today (verdicts above). Yes in the same place
pi4-target-mark-clear clears `TARGET_USE`: the free of the slot
(`free_texture`, also reached from `attachment_released`) and the hand-out
(`glGenTextures`/the bind that makes a far name's slot, `init_texture`).
Reasons: (1) it is one line next to the first, `orphan_moved` already
treats the two rows as one (`framebuffer.rs:1389`-`:1402`), a fix that
clears one and not the other leaves them disagreeing on the same slot
(`TARGET_USE == 0` with `resolved != 0`: harmless, `kick == 0` returns
first; `TARGET_USE != 0` with `DEPTH_TARGET` zero: harmless too, but a
reader has to prove it, as this note does); (2) it closes the kick-counter
caveat. The fix must clear the row of **every context slot** of the share
group, not only the current one (the tables are `[ctx][slot]`, the texture
is per group). Do not clear only in `glGenTextures`: `free_texture` is
reached from `attachment_released` too and the entry is wasted state until
the slot is handed out.

## Host test that would show the defect (the wait)

Nothing wrong shows in the data; the observable defect is the wait. In a
`src/gl` in-crate test with the mock backend (`src/testutil.rs`;
`depth_resolves()` records `flushes`/`finishes`, `set_kick_pending`,
`finishes()`):

1. `glGenTextures` A; `glTexImage2D` A as `GL_DEPTH_COMPONENT` (R32F depth,
   64x64); FBO with A at `GL_DEPTH_ATTACHMENT`; draw (mock `kick_id()`
   returns K1). `set_kick_pending(K1, true)`.
2. `glDeleteTextures(1, &A)`; `glGenTextures(1, &B)` -> assert `B == A`
   (same slot).
3. `glTexImage2D` B as a depth texture, other size (32x32) and
   `glTexSubImage2D` into it.
4. Assert `finishes()` did not rise and no `depth_resolves()` entry for
   id_B. **Today it fails**: the SubImage goes through `settle_texture`,
   `kick_pending(K1)` is true, the mock counts a flush and a finish, and
   `resolve_depth` calls `depth_resolve` for B's serial. After
   pi4-target-mark-clear (with the `DEPTH_TARGET` reset) it passes.
5. Second assertion, same test, for the data: B's texels read back what
   the SubImage wrote (and, for the sampling path, `glReadPixels` of a draw
   that samples B equals the CPU data), with and without the fix: the
   answer must not change.
6. The counter variant of the caveat: make the mock's `kick_id()` return
   `r` for B's first render (pending), then sample B on a CPU path and
   assert exactly one `depth_resolves()` entry for id_B. This passes today
   only if `resolved != r`; it would fail with a restarted counter.

Per-back-end repeat of 1-5 against the real backends, with the real
planes: tests/depth_resolve_v3d.rs, tests/depth_resolve_pvr_backend.rs and
tests/gc_depth_resolve.rs already drive A's render and resolve; the delete,
regen, size-change steps add on top, asserting `v3d`'s `depth_resolves`
counter / PowerVR's -EBUSY-free resolve stays 0 and the B texels are
untouched.

## Not examined

* The kick-id restart caveat on real boards (only read from the code).
* Cost on the boards: how often glmark2 deletes and re-gens a depth texture
  (shadow/refract make theirs once); the stale-stamp cost in terrain is
  colour textures, see pi4-slow-terrain.md.
