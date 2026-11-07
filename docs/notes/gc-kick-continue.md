<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: a render-target change that does not end the kick (design)

Plan #328, item kick-continue-design (issue #1204). This is the step after
mip-one-kick. `gc-slow-stream-diff.md` finding 2 (b) asks for it: today a
render-target change ends the GC kick, so terrain has about 50 more kicks a
frame than Mesa, refract 12 and shadow 2. This note lists every place in the
code that assumes one target per kick. It gives the changes file:line by
file:line, the host tests that prove them, the kicks a frame each scene
should then have, and how the step fits with resolve-own-kick, mip-one-kick
and gc-frames-ring.

Host only. No code was changed and no board was run. Code is cited at
origin/main `c707598d` (2026-11-07, after tiled-copy-rs). Mesa is the
26.2.1 tree in the deps cache (`mesa-pan/mesa-26.2.1`).

## Summary

- At a kick boundary the stream gets the OSAL's epilogue and prologue
  (`driver/rust/src/gpu/vivante/cmdbuf.rs:157-190`), our context init
  (`src/vivante/backend.rs:2194-2227`), a dedup invalidation, the
  occlusion-query suspend and, for a 4x target, the RS resolve. Of all that,
  a target change needs only one thing: a colour and depth cache flush
  followed by an RA-to-PE stall. That is Mesa's own rule
  (`etnaviv_emit.c:274-291`: `ETNA_DIRTY_FRAMEBUFFER` adds `COLOR | DEPTH`
  to the flush, then the RA-PE stall). `emit_flushes`
  (`backend.rs:1798-1803`) already writes it at every draw, with `TEXTURE`,
  `TEXTUREVS` and a TS flush on top. So the change is mainly the removal of
  one condition (`backend.rs:2912`). Most of the work is in the places that
  learn the kick's fence later than they do today: the mipmap chain's fence,
  and the MSAA resolve.
- After this step, `frame_submit` runs only at:
  - a CPU wait (`finish`, glFlush, the GL core's flush before a CPU access);
  - a full stream or arena (`backend.rs:2908-2914`, `:2966-2984`);
  - the swap;
  - a change of target away from a 4x target whose scratch has to be
    resolved (decision D2 below).
- Expected kicks a frame (harness, `gc-slow-host-counts.md` method):

  | scene | today | after mip-one-kick | after kick-continue |
  |---|---:|---:|---:|
  | terrain | 52 | about 9 | **1** |
  | shadow | 3 | 3 | **2** |
  | refract | 13 | about 3 | **2** |

  Shadow and refract keep a second kick because of the depth resolve, until
  depth-sample-gc lands. After depth-sample-gc, and after the GL-core change
  C9 below (gated on the X2 depth-TPU session), shadow and refract go to 1.
  Mesa has 2 for terrain and 1 each for shadow and refract
  (`gc-slow-stream-diff.md` "terrain", "shadow", "refract").
- Two existing ordering defects sit on the same lines. The fix for each is
  part of this design (C3, C4):
  - The MSAA destination is set before the target-change submit
    (`backend.rs:2853-2866` against `:2912-2913`). A change from one 4x
    target to another resolves the first target's samples into the second
    target, and the second target is then never resolved.
  - `ds_cache::select` stamps the surface with `gc_kick_id()` (`:2839`)
    before the kick may be submitted (`:2913`, `:2968`). A draw that lands
    in the next kick leaves its depth surface stamped with the kick before.
    This is the shape the GL core fixed with `restamp_draw`
    (`src/gl/draw.rs:508-520`).

## What a boundary writes, and what a target change needs

| written at a kick boundary | where | does a target change inside one stream need it? |
|---|---|---|
| OSAL epilogue: `FLUSH DEPTH\|COLOR\|SHADER_L1`, TS flush, FE-PE semaphore and stall, EVENT | `cmdbuf.rs:168-185` | The flush and the TS flush: yes. The next draw's `emit_flushes` (`backend.rs:1799-1802`) writes both, plus `TEXTURE` and `TEXTUREVS`, then the RA-PE stall. The semaphore and the EVENT: no. Mesa writes neither at a framebuffer change. |
| OSAL prologue: `FLUSH DEPTH\|COLOR`, FE-PE semaphore and stall, PIPE_SELECT 3D | `cmdbuf.rs:157-162` | No. The pipe is already 3D, and the flush is the one above. |
| context init: pipe, API mode, PA and RA constants, descriptor flush, VS icache invalidate, attribute array cleared | `backend.rs:2194-2227` | No. Nothing in it depends on the target. A shader or descriptor change between two draws to one target is already handled within one stream (dedup's program group, the per-slot `NTE_DESCRIPTOR_INVALIDATE`, `texture.rs:639-660`). |
| `dedup::invalidate` | `frame_open` `:2261`, `frame_submit` `:2296` | No. The registers keep their values within the stream. The PE group's key holds the target's address, size and pitch (`pe_key` `:2040-2043`), the program group's its height (`program_key` `:2095`, for the frag-coord constants), and a 64-bit target's PE writer runs every time (`pe_key` returns false, `:1996-1998`). So the new target's words are written whenever they differ. |
| `oq_suspend` | `:2300` | No. An occlusion query counts samples whatever the target. The counter stays armed across the change, as it does across draws today. |
| 4x resolve (`emit_resolve`) | `:2308-2311` | Yes, at a change away from a 4x target. See D2. |
| flip study (test knob) | `:2312-2314` | It copies `g.target_*`, which becomes the last target of the kick. The study only reads the window, which is the last target before a swap. No change. |

Mesa does the same: terrain's golden stream holds 9 targets and 43 mipmap
levels in 2 submits (`gc-slow-stream-diff.md` "terrain"). The barrier
between a render and a later sample of that texture is
`FLUSH TEXTURE`/`TEXTUREVS` (`etnaviv_emit.c:266-269`), and `emit_flushes`
writes it at every draw. Between a render and an RS read of it (the tiled
copy), the barrier is `FLUSH COLOR|DEPTH` followed by the RA-PE stall. That
is `emit_rs` (`backend.rs:961-962`), as in `etnaviv_rs.c:845-850`.

## Every one-target-per-kick assumption

"Today" is what the code does at `c707598d`. "After" is what the place does
under this design. The C numbers name the changes in the next section.

| # | place | assumption | after |
|---|---|---|---|
| A1 | `src/vivante/backend.rs:2899-2915` (`draw_one`): `gl.target_gpu != g.target_gpu` in the submit condition (`:2912`) | a new target ends the kick | removed (C1) |
| A2 | `backend.rs:130-133` (`target_gpu`, `target_w`, `target_h`: "a change closes the frame"); set only by `frame_open` (`:2265-2267`) | the frame's target is fixed for the kick | they become the last target recorded, updated at a change (C1) |
| A3 | `backend.rs:69-77` (the `GC_FRAMES` comment: "a full stream, a target change or glFlush closes a kick early") | target changes make several kicks a frame | the comment loses "a target change" (C1). The slot count is discussed under gc-frames-ring. |
| A4 | `backend.rs:2305-2311` (`frame_submit` resolves `ms_dest_gpu` at the end of the stream); `:144-159` (one scratch `msaa` and one `ms_dest_*`); `ensure_msaa` `:803-818`; `emit_resolve` `:1027-1043` | a kick has at most one 4x target, so one scratch and a resolve at the end are enough | a change away from a 4x target still submits, so the resolve stays at the end of the kick. That means the change point (D2, C2). |
| A5 | `backend.rs:2853-2866`, the 4x destination set **before** the frame decision `:2909-2915`; `Mark::restore_ms_dest` `:3094-3099` | none intended. With A1 this is the ordering defect named in the summary. | `ms_dest` is set after the frame decision (C3) |
| A6 | `backend.rs:2834-2848` (`ds_cache::select(.., gc_kick_id(), ..)`), `ds_cache.rs:384` (`used_by = kick` at select) | the kick open at select is the kick the draw lands in. That fails when `:2913` or `:2968` submits in between. | `used_by` is stamped after recording, in `ds_cache::drawn` (`backend.rs:2990`, `ds_cache.rs:397-405`) (C4) |
| A7 | `ds_cache.rs`: `victim` `:642-660`, `gone` `:410-427`, `seed_depth` `:497-513`, `copy_plane` `:435-460`, `resolve_depth` `:590-592` (`-EBUSY`) | per-kick fences: a surface is free once the kick that used it has completed | still right. Kicks are longer, so a surface stays pending until the swap or a CPU wait. `Victim::AllPending` (`:302-305`) now needs more than `ENTRIES` depth/stencil pairs in one swap-frame, not in one target's pass. That is the dEQP shape to watch (B2). |
| A8 | `src/vivante/tiled_copy.rs:478-482` (`refresh`: a render into the texture in a pending kick is finished first, unless the RS can tile it); `emit_pending` `:433-449`; `busy` `:624-626`; `victim` and `lru_idle` `:598-620` | the render that wrote a texture is in an earlier kick, or in the open one only by accident | right as it stands. A render in the open kick is the normal case now: an RS-fit texture (`rs_fits` `:397-409`, terrain's maps) gets its RS copy in the stream after the render, behind `emit_rs`'s flush and stall. Any other texture still calls `finish()`, which submits. `tests/gc_tiled_copy_reuse.rs:526` ("the render and the draw are two kicks") becomes one kick (T3). |
| A9 | `src/vivante/mipmap.rs:266-269` (comment and `dedup::invalidate`: "Every level is a stream of its own (a new target)"), `:270-277` (a level per `record_level`, each a new target, so each level submits through A1), `:278-281` (`gc_flush` at the chain's end), `:282` (`dedup::invalidate`), `:284` (`mip_fence = last_submitted_fence()`), `:285` (`note_chain(.., gc_kick_id() - 1)`); `backend.rs:168-171` (`mip_fence`), `:2416-2421` (`last_submitted_fence`, used only here); `gc_return_mipmap_chain` `mipmap.rs:352-365` | the chain is submitted before `glGenerateMipmap` returns, so its last fence is known | the chain stays in the open kick. `mip_fence` becomes `mip_kick`, a kick id, and the reader waits through one kick-id wait helper (C5). |
| A10 | dedup invalidation points: `frame_open` `:2261`, `frame_submit` `:2296`, `mipmap.rs:269` and `:282`, `gc_depth_resolve` `:3017`, `Mark::restore` `:3091` | each stream starts with every group forgotten, and the chain's kicks are streams of their own | only the stream ends and `Mark::restore` stay. `mipmap.rs:269` and `:282` go (C5). The words are compared anyway, so keeping them would only re-emit the groups. `gc_depth_resolve`'s stays: the GL core submits before a CPU resolve (`src/gl/texture.rs:709`). |
| A11 | `oq_suspend` at `frame_submit` `:2300`; `oq().on = 0` in `frame_open` `:2259` | per stream | unchanged. The armed counter now crosses target changes, which is right for GL. |
| A12 | stream and arena limits: `GC_CMD_WORDS` `:81` (128 K words), `GC_ARENA_BYTES` `:82` (1 MiB a slot); `full` `:2908-2914`; the overflow re-record `:2964-2984`; `ensure_extra` `:3026-3045` | a kick is one target's pass, so these limits are seldom reached | unchanged code. A kick now holds a whole swap-frame: terrain about 19 K words and 17 KiB of arena a frame today, refract 4.6 K, shadow 1.5 K (`gc-slow-stream-diff.md`, `gc-slow-host-counts.md`). All three fit with room to spare. Slint's 66 K words fit too. A frame that does not fit splits as it does today. |
| A13 | `src/vivante/wide_target.rs:33-35` and `prepare` `:155-166` ("before the first draw of a kick into it", `tile_in` when `e.kick == 0`); `bind` (`backend.rs:1672`, `gc_kick_id()`); `retire` `:272-294` | each entry is keyed by the kick that rendered into it | right as it stands. A, B, A in one kick finds A's shadow with `e.kick` set to the open kick, so no `tile_in` is needed and none is done. A 64-bit texture sampled in the kick that rendered it goes through `tiled_copy::refresh` -> `finish()` (not `rs_fits`), which submits and retires as today. |
| A14 | OSAL deferred frees: `src/osal/zephyr_gc.c:178-196` (a free is held for the open frame), `src/osal/gc_frees.h:97-117` (`fence = submitted + 1` while a frame is open), `src/osal/pending_frees.h:124-134` (128 entries; `-EBUSY`, "unsubmitted", when every held block waits for the open frame) | a kick ends often, so the open frame's held frees are few | unchanged code. The frame stays open for a whole swap-frame, so an application that frees many blocks between two swaps, with no CPU wait, can fill the queue with open-frame entries. Watch it (B2). |
| A15 | GL core: `texture.rs:709` (`resolve_depth`: flush, then `finish`), `texture.rs:850` (`depth_for_draw`: flush when the depth kick is pending), `buffer.rs:181` and `:438`, `texture.rs:1036` (a CPU write to an object whose `last_use` kick is pending: flush and finish, or orphan) | the kick that last used an object is often already submitted | unchanged code. More CPU writes now find their kick still open, which costs a submit and a wait. The glmark2 scenes write no buffer or texture between draws of a frame (harness: no wait at those call sites). `texture.rs:850` is C9. |
| A16 | `kick_pending` `:3288-3307`, `gc_kick_id` `:3269-3271`, `gc_finish_previous` `:3245-3258`, `finish` `:3213-3226`, `restamp_draw` `src/gl/draw.rs:508-520` | by kick id, not by target | unchanged. A kick id lives longer in the ring because there are fewer kicks. |
| A17 | `emit_context_init` `:2194-2227`, `emit_flushes` `:1798-1803`, `emit_draw`'s FE-PE stall `:2170` and RA-PE stall `:2185` | none tied to the target | unchanged. The proof that they are enough is Mesa's rule (above) and the board steps B1 and B3. |

## Decisions

- **D1. No cap on target changes a kick.** A cap (say 64 changes) would
  bound the open frame's held frees (A14) and its pending surfaces (A7). But
  terrain alone has 52 changes a frame (9 passes and 43 levels), and Mesa
  has no cap. Leave it out. If the dEQP gate shows the frees queue refusing
  "unsubmitted" blocks (the OSAL's close line, `pending_frees.h:153-161`) or
  `AllPending`, add a constant `GC_KICK_TARGETS_MAX` at `backend.rs:81`.
  When it is reached, the change submits.
- **D2. A change away from a 4x target still submits.** The scratch surface
  is one (`backend.rs:146`), and today it is resolved at the end of the
  stream (`:2308-2311`). Resolving it in the stream at the change instead
  means three things. It needs room kept for the RS block, which
  `frame_submit` already gets. It needs a fix for the case where the resolve
  itself overflows inside `Mark`'s window. And it fixes nothing that is
  measured: 4x is off in every build (`GC_MSAA_ENABLE` false, `:388`), and
  only gl_test case 16 turns it on. So at a change, if `g.ms_dest_gpu != 0`,
  `frame_submit()` runs as today, and the resolve lands at the change. The
  in-stream resolve is a later step, once 4x is used. The plan's "the
  `ms_dest_gpu` resolve has to be emitted at the target change" is met this
  way.
- **D3. The chain's wait is by kick id.** Waiting by kick id needs a submit
  when the kick is still the open one, and a fence lookup in the ring
  otherwise. resolve-own-kick needs the same thing (it waits for the kick
  that rendered the depth). There is one helper, `wait_kick(kick_id) -> i32`,
  in `backend.rs` next to `kick_pending` (`:3288`). It does this:
  - if `kick_id` is the open kick (`kick_id == g.kick_id + 1 && g.recording`),
    it calls `frame_submit()`;
  - it looks for the slot with `f.kick_id == kick_id` and calls
    `wait_frame(i)` (`:3197-3206`, counted in `waits`);
  - it answers 0 when no slot holds the kick (too old, so already complete).

  If resolve-own-kick lands first with its own lookup, C5 replaces that
  lookup with this helper.

## The changes, file:line

All lines are at `c707598d`. mip-one-kick, resolve-own-kick and resolve-wide
are not on main yet, so their lines will move. Each change says what it
applies to.

- **C1. The kick no longer ends at a target change.** `backend.rs:2912`
  becomes `if g.recording && (g.frames[g.cur].w.overflow || full ||
  (gl.target_gpu != g.target_gpu && g.ms_dest_gpu != 0))`. The last term is
  D2. After the `frame_open` at `:2916-2918`, set `g.target_gpu`,
  `g.target_w` and `g.target_h` from `gl` on every draw. `frame_open`'s
  writes at `:2265-2267` can stay; they are the same values. Rewrite the
  comments at `:130` ("the target of the last draw recorded; a change ends
  the kick only when a 4x target is left"), `:72-76` (A3) and `:2899-2900`.
- **C2. MSAA.** Nothing beyond C1's last term and C3. `frame_submit`'s
  resolve (`:2305-2311`) stays as it is. Add one comment at `:2305`: the
  resolve runs at the end of the stream, and a change away from the 4x
  target ends the stream (D2).
- **C3. The 4x destination is set after the frame decision.** Move
  `:2853-2866` (`ms_via_scratch`: `ensure_msaa` and `ms_dest_*`) to after
  `:2922` (`clock.lap(Step::GcFrame)`). At that point the open frame has
  already been submitted with its own resolve if the change left a 4x
  target. The submits that can come earlier (`ds_cache::select`'s `finish`
  `:2834`, `tiled_copy::prepare` `:2882`, `wide_target::prepare` `:2890`)
  then still resolve the previous destination, which is what the comment at
  `:2826-2828` wants. `Mark::take` (`:3064-3077`) runs after the move, so
  `restore_ms_dest` (`:3096-3099`) still puts back the new destination
  after the overflow re-record's submit (`:2967-2976`).
- **C4. The depth surface is stamped with the kick that holds the draw.**
  `ds_cache::drawn` (`ds_cache.rs:397`) takes `kick: u32` and sets
  `e.used_by = kick`. The call at `backend.rs:2990` passes `gc_kick_id()`,
  after recording, as `tiled_copy::emit_pending` and `wide_target::bind`
  already do. `select` keeps its stamp at `:384`, so a draw refused after
  select still protects the surface it may have filled (seed or copy). The
  file is resolve-wide's while that item runs. Land C4 after it.
- **C5. The mipmap chain stays in the open kick.** Applies on top of
  mip-one-kick.
  - mip-one-kick will need a way past A1 for the levels (a flag, or a state
    that the target check skips). Delete it: C1 removes the check.
  - Delete `mipmap.rs:278-281` (`gc_flush` at the chain's end) and the
    `dedup::invalidate` calls at `:269` and `:282`.
  - `backend.rs:168-171`: `mip_fence: i64` becomes `mip_kick: u32`, the
    kick that holds the chain's last level. `mipmap.rs:284` becomes
    `g.mip_kick = backend::gc_kick_id()`, and `:285`
    `note_chain(t.base_gpu, backend::gc_kick_id())`, the open kick, not
    `- 1`.
  - `gc_return_mipmap_chain` (`mipmap.rs:352-365`) becomes: when
    `kick_pending(g.mip_kick)`, call `wait_kick(g.mip_kick)` (D3), with the
    log line as now. The fences are ordered, so the newest chain's kick
    still covers every chain before it.
  - `last_submitted_fence` (`backend.rs:2416-2421`) has no caller left.
    Delete it.
  - The error paths at `:272-275` (`gc_finish` before the CPU filter) stay.
  - Update the comments at `:266-268`, `:291-293` ("one kick per level,
    every kick submitted before this returns") and `tiled_copy.rs:473-475`.
- **C6. `wait_kick`** (D3) in `backend.rs` after `kick_pending`
  (`:3288-3307`). It is also the body for `YakoglBackend::wait_kick`
  (`:3405`, `None` today, "until this backend waits for one kick"). Fill
  that entry in only when resolve-own-kick does. The sync objects then stop
  calling `finish()` (`src/gl/es3.rs:326`, `:1110`, `src/gl/egl.rs:2583`).
  That is a separate change with its own tests.
- **C7. Comments that state the old rule.** `backend.rs:2123`
  (`emit_context_init` "at the start of a stream" stays right),
  `dedup.rs:28-33` (drop "the mipmap chain's kicks"),
  `tests/gc_state_dedup.rs:17` (same).
- **C8. Tests that pin the old rule** change with C1 and C5. They are listed
  as T-items below. No test outside these files asserts a kick at a target
  change. The other files' kick counts are per glFlush or glFinish
  (`tests/gc_npot_repeat.rs:423`, `tests/es3_int_rt_mrt_gc.rs:351`).
- **C9 (later, gated). The GL core submits the depth kick only when the
  backend reads on the CPU.** `src/gl/texture.rs:850` flushes when the
  depth texture's kick is pending, so that `samples_rendered_depth` can be
  asked. Under depth-sample-gc's answer (a), the GC7000 samples the depth
  in the stream, so the flush only splits the kick. Change: flush only when
  the backend does not sample in the stream. Gate it on the X2 depth-TPU
  session's check 7 (`gc-depth-sample-wire.md` "(a).10"): that check is
  exactly whether the texture unit sees the PE's depth with only
  `emit_flushes` in between. It is a GL-core change in plan #310's area, so
  keep it to that one line. This takes shadow and refract from 2 kicks to 1.

## Host tests

Each test names its deliberate break: the line that, put back, makes it
fail. In `src/vivante/backend_tests.rs` (the `Rig` mock OSAL keeps every
submitted stream):

- **T1** replaces `a_frame_holds_every_draw_to_one_target_and_a_new_target_closes_it`
  (`backend_tests.rs:1012-1032`) with
  `a_frame_holds_draws_to_several_targets_until_a_flush`. Draws into A, B, A
  give 0 kicks. `gc_flush` then gives 1 kick of 3 draws. In that stream:
  - `pe_pipe_color_addr(0)` is written A, B, A, in draw order;
  - between the last draw into A and the write of B's address there is a
    `GL_FLUSH_CACHE` with `COLOR|DEPTH` and an RA-PE stall;
  - `GC_GL_PIPE_SELECT` is written once (one context init).

  Break: the `gl.target_gpu != g.target_gpu` term back in `:2912`.
- **T2** `a_target_change_keeps_the_groups_and_rewrites_the_pixel_engine`.
  Two draws with the same state into A and then B, in one stream. The second
  writes the PE group (its key holds the target) and no other group. With
  `dedup::set_verify` on (the host-test default), this also proves the key
  and the words agree across a change. Break: a `dedup::invalidate()` at
  the change (every group then appears twice).
- **T3** in `tests/gc_tiled_copy_reuse.rs:482-530`
  (`a_render_into_the_texture_is_tiled_by_the_rs_with_no_finish`). "The
  render and the draw are two kicks" becomes one kick. In it, the render's
  draw comes before the `RS_KICKER` of the copy, and the copy comes before
  the sampling draw. The stream has `FLUSH COLOR|DEPTH` and an RA-PE stall
  between the render's draw and the RS block. `gc_finishes` is unchanged.
  Break: C1 undone (two kicks again).
- **T4** `leaving_a_4x_target_resolves_it_into_its_own_storage` (C3, D2).
  4x on (`gc_debug_set_msaa`, as in `:4509`). Draw into 4x target A, then
  into 4x target B (another address). This gives 2 kicks. Kick 1 ends with
  the RS resolve whose destination (`rs_pipe_dest_addr(0)`) is A. Kick 2,
  after `gc_flush`, ends with the resolve into B. Break: today's order
  (`ms_dest` set before the decision). Kick 1 then resolves into B and
  kick 2 has no resolve. Add a second case: 4x A, then a single-sampled B.
  Kick 1 resolves A, and kick 2 has no resolve.
- **T5** `a_depth_surface_is_stamped_with_the_kick_that_holds_the_draw` (C4).
  `squeeze_frame_arena_for_tests` leaves too little room, so the next depth
  draw submits at `:2913` and lands in the next kick. Complete every kick
  except that one (the mock's fence control, as
  `a_kick_is_pending_until_its_fence_completes` `:4186` does). Then:
  - a draw with another identity, when every other slot is taken, does not
    get that surface (`ENTRIES` identities, as `:1854-1879` does);
  - `gc_depth_resolve` on it answers `-EBUSY`.

  Break: `drawn` without the stamp.
- **T6** replaces `a_chain_asked_for_during_a_frame_into_level_0_follows_that_frames_kick`
  (`:5174-5216`, which mip-one-kick will already rewrite). A frame into
  level 0, then `generate(&b, 7)`:
  - `g().recording` is still true and 0 kicks have been submitted;
  - `chain_kick_pending(base, kick_pending)` is true;
  - `gc_return_mipmap_chain` submits 1 kick of 7 draws and waits for it, so
    `waits` goes up by 1 under a fence that is not complete;
  - each level's `pe_pipe_color_addr(0)` is at its chain offset, in order.

  Break: `mipmap.rs:278` (`gc_flush`) back. The test fails at the
  kicks-before-return assert.
- **T7** `a_gpu_made_chain_stales_the_tiled_copies_of_its_storage_and_names_its_kicks`
  (`:5245-5285`): `last = gc_kick_id() - 1` becomes `gc_kick_id()` (the open
  kick) before the flush.
- **T8** OQ: `counted_draws_arm_a_slot_and_every_change_and_the_stream_end_suspend`
  (`:4927`) gets a variant with a target change between two counted draws.
  The result is one arm and one suspend, at the stream's end, and the count
  is the sum. Break: an `oq_suspend` at the change.

In `tests/`:

- **T9** `tests/gc_state_dedup.rs:657-680` ("the mipmap chain's kicks: the
  draw after them writes every group"): `1 + 6 + 1` streams become 1, and
  the draw after the chain writes only the groups whose words differ from
  the last level's quad.
- **T10** `tests/gc_mipmap_gpu.rs:847`, `:1026`, `:1103`, `:1129` and
  `tests/gc_glmark2_mip_offer.rs:316` ("one kick per level above 0"):
  mip-one-kick turns them into 1 kick a chain, and C5 into 0 kicks until the
  next flush. Their level-by-level checks of the descriptors in the arena
  (`gc_mipmap_gpu.rs:27`, "four kicks on") become easier: every level's
  descriptor is in the one arena.
- **T11 (counts)** the glmark2 host harness, both commands of
  `gc-slow-host-counts.md` "Run", filling its "after" column for this item:

  | scene | row | baseline (`f862e8e7`) | after mip-one-kick (expected) | after kick-continue (expected) |
  |---|---|---:|---:|---:|
  | terrain | kicks | 52.0 | about 9 | 1.0 (0.9 swap + 0.1 glFinish attribution) |
  | terrain | waits other (lag) | 46.2 (45.2 after tiled-copy-rs) | about 9 + 0.2 | 0.2 (glFinish only) |
  | terrain | stream KiB | 74.1 | less | about 74.1 - 51 x (context init, about 0.3 KiB, + the groups the first draw of each kick re-emits) |
  | terrain | flushed calls | 118.0 | about 30 | about 2 + the arena's (one `frame_submit` a frame) |
  | shadow | kicks | 3.0 | 3.0 | 2.0 |
  | shadow | waits other (lag) | 1.8 (1.7 resolve) | 1.8 | 1.0 (the resolve finds one kick pending, K1+K2) |
  | refract | kicks | 13.0 | about 3 | 2.0 |
  | refract | waits other (lag) | 9.4 | 1 to 2 | 1.0 or less |
  | all | draws, draw box Mpx, invalidated KiB | | unchanged | unchanged |

  `gc-scene-kicks`'s ceilings drop to these numbers in the commit that lands
  C1.
- `tools/check.sh --changed origin/main` on the result. C1 reaches
  `src/vivante`, so expect the GC rows of the plan.

## Kicks a frame, scene by scene

From the pass tables in `gc-slow-terrain.md` ("The 52 kicks") and
`gc-slow-shadow-refract.md` ("The frames"):

- **terrain: 1.** All of the following share one stream, submitted at the
  swap: the canvas clear; the height, normal, terrain, bloom_h, bloom_v,
  overlay, tilt_h and tilt_v passes; the 5 chains (43 levels); and the
  vertex-stage maps' RS copies (tiled-copy-rs). Nothing in the frame waits
  on the CPU any more: tiled-copy-rs removed the tiled copy's `finish()`,
  and no buffer or texture is written mid-frame. The stream is about 19 K
  words of 128 K, and the arena 17 KiB of 1 MiB. Mesa's 2 submits are at
  least as many. If the board shows 2, the second is a CPU access the
  harness does not model. The kick boundaries removed are 51 a frame: 8
  after mip-one-kick's 43.
- **shadow: 2.** The canvas clear and the 1600x1200 depth pass share a
  stream (finding 3 of the stream diff goes too). The ground's draw samples
  the depth texture, so `texture.rs:709` flushes and `finish()`es, and the
  CPU resolve follows. The window's draws then form kick 2. With
  depth-sample-gc and C9: 1.
- **refract: 2.** The canvas clear, the 1600x1200 pass and its 10-level
  chain share a stream. The window's draw that samples the depth flushes
  it, as shadow's does. The window's draws form kick 2. With
  depth-sample-gc and C9: 1.

The gain, from `gc-slow-stream-diff.md`'s per-kick cost of 0.03 to 0.08 ms
of GPU time: terrain 8 boundaries after mip-one-kick come to 0.24 to
0.64 ms; shadow and refract 1 each come to 0.03 to 0.08 ms. On top come one
interrupt and one `frame_open` check a boundary on the CPU. These are small
next to the scenes' 250 and 500 ms, as finding 2 (b) said. The step matters
more for what it removes than for its own time:
- the frames-ring pressure (below);
- the per-kick re-emission of state;
- a class of kick-ordering bugs (C3, C4).

## Interactions

- **mip-one-kick (planned, before this).** It is the first change that puts
  a render and a sample of that render in one stream on this board (level
  l-1 drawn, then sampled for level l, with only `emit_flushes` between).
  Its board result is the evidence for A17 in general. mip-one-kick ends
  the chain with a submit; this step removes that submit (C5). Its bypass
  of A1 is deleted (C5). Its expected 9 kicks for terrain are this step's
  starting point. If kick-continue lands first, mip-one-kick shrinks to
  C5's removal of `gc_flush` and the per-level path. Either order works.
  The tests T6, T7, T9 and T10 are rewritten by whichever lands second.
- **resolve-own-kick (ready).** Its goal, the depth resolve waiting for the
  kick that rendered the depth and not for every slot, needs the same
  wait-by-kick-id as C5. So `wait_kick` (D3, C6) should be one function used
  by both. Under kick-continue the kick that rendered the depth is the open
  kick at resolve time in shadow and refract (the clear and the depth pass
  are one kick). The resolve then submits it and waits for that one kick.
  Kick-continue alone already gives shadow's "waits other" 1.0, the number
  resolve-own-kick predicts, because only one kick is pending at the
  resolve. Resolve-own-kick still matters when older kicks are pending (a
  swap-frame split by a full stream). Land resolve-own-kick first if it is
  ready first. C6 then reuses its lookup.
- **gc-frames-ring (todo).** It adds stream slots so that `frame_open`
  (`backend.rs:2238-2243`) stops waiting for a slot whose kick is 4 kicks
  old. Its predicted gain (`gc-slow-host-counts.md`: terrain's 40 + 5
  `frame_open` waits go) comes from terrain's 52 kicks. After mip-one-kick
  there are 9 a frame. After this step there is 1 a frame for terrain and
  2 for shadow and refract. Four slots then hold 2 to 4 frames, and the
  slot `frame_open` reuses belongs to a frame that `gc_finish_previous`
  has already waited for. So after kick-continue, gc-frames-ring moves no
  row of these three scenes. Each slot costs 512 KiB of stream and 1 MiB of
  arena (`:81-82`). Recommendation: do kick-continue first, then decide
  gc-frames-ring by the CPU-wait cases left: glFlush-heavy applications,
  full streams, dEQP. Leave `GC_FRAMES` at 4 for this step.
- **rs-clear-wire (todo).** An RS fill into target B right after draws into
  A, in one stream, needs the colour and depth flush and the RA-PE stall
  before the fill. It also needs a stall after it, before B's first draw
  (`gc_draw.c:217-259`, which the item copies). The item already plans
  both. With them, the fill is right whether or not a kick boundary falls
  there.
- **tiled-wire-design (tile status).** Under TS, a target change inside a
  stream also changes the TS state. Mesa flushes the TS cache when its TS
  state is dirty (`etnaviv_emit.c:294-297`), and `emit_flushes` already
  writes the TS flush at every draw. The TS registers then belong in a
  dedup group keyed by the target, as the PE's are.
- **depth-sample-gc.** See C9: its check 7 decides whether the GL core may
  stop splitting the kick at the depth pass.

## Board steps (not done here)

- **B1.** After mip-one-kick on the X2: glmark2 terrain's picture and
  `-b build` (a single-target control) by eye or by checksum, and the
  chain's levels by gl_test's mipmap case. This is the in-stream render
  then sample evidence.
- **B2.** After C1, a dEQP-GLES2 and GLES3 fbo and texture.size subset on
  the X2. Watch the OSAL's pending-frees close line for refusals of
  "unsubmitted" blocks (A14) and `vivante: every depth/stencil surface is in
  use by a pending kick` (A7). Either one triggers D1's cap.
- **B3.** `x2-gm-after`: page 1 v1 (waits per 100 frames) and page 3 v1 (HI
  idle) for terrain, against mip-one-kick's numbers. Page 1 v4 (kicks) is
  in w3, which the X2 does not keep, so the kick count is read from the
  host harness (T11) or from fb-counters' page 4 once that page has a
  kicks count.
