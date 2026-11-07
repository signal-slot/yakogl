<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P terrain: where kick 187 of 52 comes from

Written 2026-11-07 from the code at main `e2a4e86b`. No code change, no board
run. The host log is the one `docs/notes/pvr-hwrt-reuse.md` ("Terrain's kicks
a frame") recorded at `d83e25b5`; **this item could not re-run
`tools/hostharness/am62p-terrain/run.sh ... kicks`** (the sandbox refused to
start it), so the evidence below is that recorded log read against the code
and glmark2's source, not a fresh run. Re-running it takes one command (see
"Confirms it").

**Answer: kick 187 is not after the window's draw in frame time. It is the
next frame's window clear.** glmark2 starts every frame with
`glClear(COLOR|DEPTH)` on the window, binds an FBO for the first scene draw,
and the pending clear is then emitted as a clear-only kick of its own. That
is candidate 5 of `am62p-slow-terrain.md` (the window's dead clear kick),
the one `pvr-dead-clear-kick` changes. It is the same kick, not a different
one. No second removal design is needed.

## Reading the log

`GM2_KICKS=1` prints, after each draw, `pvr_backend_kick_id()`, which is
`last_fence + 1`: "the kick being recorded" (`src/pvr/backend.rs:9258-9263`),
not the kick that contains the draw once it is submitted. The draw lines of
a frame (`pvr-hwrt-reuse.md`): framebuffers 1, 2, 4, 5, 6, 4, 7, 0 at ids
136, 137, 138, 148, 157, 166, 176, 186, then 188 for the next frame's fb 1.

Walking the ids with the chain sizes (9, 8, 8, 9, 9 kicks after the terrain,
bloom h, bloom v, overlay and tilt-shift h draws):

| id | what |
|---:|---|
| 136, 137 | height map, normal map (256x256) |
| 138 | terrain draw (fb 4); submitted when its chain starts |
| 139-147 | terrain chain, 9 kicks |
| 148 | bloom h (fb 5); chain 149-156 |
| 157 | bloom v (fb 6); chain 158-165 |
| 166 | overlay (fb 4); chain 167-175 |
| 176 | tilt-shift h (fb 7); chain 177-185 |
| 186 | tilt-shift v: the **window's draw** (fb 0), submitted by the swap |
| **187** | **no draw logged: the next frame's window clear** |
| 188 | next frame's height map (fb 1) |

So the window's draw is in 186 (the kick recorded when the draw is logged,
flushed by `eglSwapBuffers`, `src/gl/egl.rs:2396` `backend::flush()`). The
`pvr-hwrt-reuse.md` wording "one kick (187) after the window's draw that
holds no glmark2 draw" is right about the id order and wrong about the
position in the frame loop: 187 is submitted between the swap and the next
frame's first FBO draw.

## The emitting call chain

1. `MainLoop::draw()` calls `canvas_.clear()` first, then `scene_->draw()`
   (glmark2 `src/main-loop.cpp:130-137`; `CanvasGeneric::clear`,
   `src/canvas-generic.cpp:95-104`: `glClear(GL_COLOR_BUFFER_BIT |
   GL_DEPTH_BUFFER_BIT)` on the window, fb 0 bound).
2. `glClear` only records it: `draw::pending_clear_for`
   (`src/gl/state.rs:423` -> `src/gl/draw.rs:2655-2667`) sets `pc.color` and
   `pc.depth` with `pc.fb = 0`. Nothing is submitted.
3. The scene binds fb 1 and draws. The draw's entry reaches
   `settle_pending_clear(c, ...)` (`src/gl/draw.rs:4325` for
   `glDrawArrays`, `:4732` for `glDrawElements`). The pending clear belongs
   to fb 0, the bound framebuffer is fb 1, so `settle_pending_clear`
   (`:2523-2540`) rebinds fb 0 and calls `emit_pending_clear(..., true,
   ...)` (`:2435`), which builds a clear-only `GlesDraw` and calls
   `submit_draw(c, reported, d, "the pending clear", None)` (`:2512`).
4. In the PowerVR backend that draw opens a frame on the window target.
   The fb 1 draw that follows has another target, so `frame_ends_here`
   (`src/pvr/backend.rs:5719-5730`, `b.target_gpu != d.target_gpu`) closes
   the clear's frame and `flush` (`:8493`) submits it as kick 187. The fb 1
   draw then records into 188.

`eglSwapBuffers` is not the emitter: its own `flush_pending_clear()`
(`src/gl/egl.rs:2366`) finds nothing pending after the window's draw, and
`pvr_backend_surface_swapped` (`src/pvr/backend.rs:9146-9158`) only sets
flags on the open frame (`frame_swapped`, `swap_drops_zs`). A resolve or
readback is not involved: terrain reads nothing back. Frame 1 has the same
kick as id 135, before its first draw at 136; ids 136-187 are one 52-kick
period, 188 starts the next.

## Same as candidate 5

Candidate 5 of `am62p-slow-terrain.md` (about `:303`) says: keep a
framebuffer's full pending clear pending across a bind to another
framebuffer, emit it when the framebuffer is next drawn, read, copied from
or swapped, and drop it when a later full clear of the same buffers replaces
it; 52 -> 51 kicks, -1.1 to -1.8 ms. That is this kick: the fb 0 clear that
step 3 settles at the first FBO draw. With the change the clear stays
pending through the 51 other kicks. At the window's draw (kick 186) the
clear is emitted by the fused path (the draw's own background object,
`emit_pending_clear` with the same target) instead of as a kick, so the
window's colour and depth clear come for free in 186. Then `glClear` of the
next frame replaces nothing (the swap consumed it).

Two details the dead-clear item must keep:

- The window's colour clear is dead (the tilt-shift quad covers the window),
  but its **depth** clear is not: the window draw depth-tests against the
  cleared 1.0. Fusing carries both, so nothing is lost; dropping the clear
  would be wrong. After a swap the backend also drops the window's
  depth/stencil (`swap_drops_zs`), so the clear is what re-initialises it.
- Every reader that sees fb 0 must emit it first: `glReadPixels`/copy paths
  already do (`src/gl/framebuffer.rs:1741`, `:2926`, `:3642` per
  `am62p-slow-terrain.md`); the swap does (`egl.rs:2366`).

## Ranked removal design

1. **Candidate 5 as written (`pvr-dead-clear-kick`).** Only this is needed:
   no other emitter exists for kick 187.
   - Expected on the AM62P: 52 -> 51 kicks, **-1.1 to -1.8 ms a frame**
     (one 800x600 kick that also stores the window's depth/stencil; the
     range of `am62p-slow-terrain.md` and `am62p-pvr-mipmap-kicks.md`,
     about 1.8 ms measured for a mip kick, the clear-only kick carrying less
     GPU work than a chain kick and the same fixed per-kick cost). Terrain
     83.1 ms -> about 81.3-82.0 ms.
   - Picture: none expected, since the fused draw sees the identical
     background. Check with the glmark2 picture tolerance
     (`glmark2-pictures.txt`) on the host.
   - ES 2.0 risk: low. The clear is deferred, not dropped, so
     `tests/es2_report_guard.rs` answers must not change; the risk is a
     reader of fb 0 that misses the settle (texture sampling of a texture
     attached to a framebuffer with a pending clear; a shared-colorbuffer
     FBO in dEQP `fbo.render.shared_colorbuffer`, which `frame_ends_here`
     comments at `:5715-5718`). Cover with a host test that binds fb 1,
     draws, and checks fb 0's content after `glReadPixels`.
2. **Nothing else.** The other candidates (3 for the waited kicks, 4 for the
   chain per kick price, 6 for the per-level area) do not touch this kick.

## Confirms it

- Host: `tools/hostharness/am62p-terrain/run.sh <GLMARK2_SRC> <out> kicks`
  then `kicks.py`. Today it should print 52 kicks a frame and 8 draws; after
  `pvr-dead-clear-kick` 51 kicks, with the last id of a frame's period no
  longer a draw-less one. A local `eprintln!` in `emit_pending_clear`
  (kept out of the commit) printing `c.bound_framebuffer` and `pc.fb` would
  show it firing once a frame with `pc.fb == 0` at the first fb 1 draw.
- Board: terrain `FPS:` and the `pvr last completed kick per frame` line's
  kick count 52 -> 51; ms a frame down by 1.1-1.8 (`pvr-dead-clear-kick`'s
  board session, `am62p-board-after`).

## For the planner

- `pvr-hwrt-reuse.md` ("the extra kick's size is not printed. The model takes
  it as 800x600") can be corrected: it is the window clear, 800x600, and it
  is the next frame's. Not edited here (outside this item's one file).
- This item's host run was not done (command approval denied in the
  sandbox); the first check by whoever re-runs the harness is that the
  `gm2: draw` lines for fb 1 show `kick` one higher than the previous
  frame's fb 0 line plus 2 (186 -> 188).
